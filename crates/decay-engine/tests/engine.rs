//! End-to-end engine scenarios with a fake extension: handshake, tracking, aging, sweeps
//! (probe → write-before-close → close → settle), crash recovery, browser restarts,
//! shadow mode, Undo, search and restore.

// The harness takes JSON by value so calls read like the messages they send.
#![allow(clippy::needless_pass_by_value)]

use decay_bridge::ipc::{ConnInfo, HostFrame};
use decay_bridge::protocol::{
    BrowserKind, Config, Hello, Snapshot, TabInfo, TabStatus, WindowInfo, WindowState,
};
use decay_bridge::{Envelope, ErrorCode, kinds};
use decay_core::{EventType, ItemState, Mode, SweepStatus};
use decay_engine::{ConnId, Engine, EngineConfig, EngineEvent, Input, Output, timing};
use decay_store::{DbKey, Platform, Store, StorePaths};
use serde_json::{Value, json};

const T0: i64 = 1_793_750_400_000;
const H: u64 = 3600;

struct Harness {
    engine: Engine,
    now: i64,
    sent: Vec<(ConnId, Envelope)>,
    closed: Vec<ConnId>,
    events: Vec<EngineEvent>,
    dir: tempfile::TempDir,
}

fn open_store(dir: &std::path::Path, now: i64) -> Store {
    Store::open(StorePaths::in_dir(dir), DbKey::from_bytes([9; 32]), now).unwrap().0
}

fn tab(id: i64, window: i64, url: &str) -> TabInfo {
    TabInfo {
        tab_id: id,
        window_id: window,
        index: id,
        url: url.into(),
        title: format!("Tab {id}"),
        fav_icon_url: None,
        pinned: false,
        audible: false,
        muted_info: None,
        discarded: false,
        group_id: -1,
        active: false,
        status: TabStatus::Complete,
        last_accessed: None,
    }
}

fn snapshot(tabs: Vec<TabInfo>) -> Snapshot {
    let mut windows: Vec<WindowInfo> = Vec::new();
    for t in tabs {
        match windows.iter_mut().find(|w| w.id == t.window_id) {
            Some(w) => w.tabs.push(t),
            None => windows.push(WindowInfo {
                id: t.window_id,
                focused: windows.is_empty(),
                state: WindowState::Normal,
                incognito: false,
                tabs: vec![t],
            }),
        }
    }
    Snapshot { windows, groups: Vec::new() }
}

impl Harness {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = open_store(dir.path(), T0);
        let config = EngineConfig { platform: Platform::MacOs, pro: false };
        let engine = Engine::new(store, config, T0).unwrap();
        Self { engine, now: T0, sent: Vec::new(), closed: Vec::new(), events: Vec::new(), dir }
    }

    fn collect(&mut self, out: Vec<Output>) {
        for o in out {
            match o {
                Output::Send(c, env) => self.sent.push((c, env)),
                Output::Close(c) => self.closed.push(c),
                Output::Event(e) => self.events.push(e),
            }
        }
    }

    fn store(&self) -> &Store {
        self.engine.store()
    }

    fn set_mode(&mut self, mode: Mode) {
        self.store().set_setting("mode", &mode, self.now).unwrap();
    }

    fn frame(&mut self, conn: ConnId, profile: &str, env: Envelope) {
        let info = ConnInfo {
            browser: BrowserKind::Chrome,
            profile_id: profile.into(),
            pid: 1,
            origin: "chrome-extension://x/".into(),
        };
        let out = self
            .engine
            .handle(Input::Frame(conn, Box::new(HostFrame { conn: info, msg: env })), self.now);
        self.collect(out);
    }

    fn msg(&mut self, conn: ConnId, kind: &str, payload: Value) {
        let profile = format!("profile{conn}");
        self.frame(conn, &profile, Envelope::new(kind, &payload).unwrap());
    }

    fn request(&mut self, conn: ConnId, kind: &str, payload: Value) -> String {
        let env = Envelope::new(kind, &payload).unwrap();
        let id = env.id.clone();
        let profile = format!("profile{conn}");
        self.frame(conn, &profile, env);
        id
    }

    /// Connects `conn` as profile `profile{conn}` and answers the `config`.
    fn connect(&mut self, conn: ConnId, session: &str) {
        self.connect_as(conn, &format!("profile{conn}"), session, "0.1.0");
    }

    fn connect_as(&mut self, conn: ConnId, profile: &str, session: &str, version: &str) {
        let out = self.engine.handle(Input::Connected(conn), self.now);
        self.collect(out);
        let hello = Hello {
            ext_version: version.into(),
            browser: BrowserKind::Chrome,
            profile_id: profile.into(),
            capabilities: vec![],
            launch: false,
            session: Some(session.into()),
        };
        self.frame(conn, profile, Envelope::new(kinds::HELLO, &hello).unwrap());
        if let Some(config) = self.take(conn, kinds::CONFIG).pop() {
            self.ack(conn, &config, json!({"ok": true}), profile);
        }
    }

    fn ack(&mut self, conn: ConnId, cmd: &Envelope, ack: Value, profile: &str) {
        let mut env = Envelope::new(kinds::ACK, &ack).unwrap();
        env.reply_to = Some(cmd.id.clone());
        self.frame(conn, profile, env);
    }

    fn ack_ok(&mut self, conn: ConnId, cmd: &Envelope, result: Value) {
        let profile = format!("profile{conn}");
        self.ack(conn, cmd, json!({"ok": true, "result": result}), &profile);
    }

    fn take(&mut self, conn: ConnId, kind: &str) -> Vec<Envelope> {
        let (hit, rest): (Vec<_>, Vec<_>) =
            self.sent.drain(..).partition(|(c, e)| *c == conn && e.kind == kind);
        self.sent = rest;
        hit.into_iter().map(|(_, e)| e).collect()
    }

    fn one(&mut self, conn: ConnId, kind: &str) -> Envelope {
        let mut all = self.take(conn, kind);
        assert_eq!(
            all.len(),
            1,
            "expected one {kind}, got {all:?} (others: {:?}) events {:?} status {}",
            self.sent,
            self.events,
            self.engine.status_line()
        );
        all.remove(0)
    }

    fn reply_to(&mut self, conn: ConnId, request: &str) -> Envelope {
        let pos = self
            .sent
            .iter()
            .position(|(c, e)| *c == conn && e.reply_to.as_deref() == Some(request))
            .expect("a reply");
        self.sent.remove(pos).1
    }

    fn poll(&mut self) {
        let out = self.engine.poll(self.now);
        self.collect(out);
    }

    /// Moves time forward in steps, polling (and answering heartbeats) on the way.
    fn advance(&mut self, ms: i64, conns: &[ConnId]) {
        let end = self.now + ms;
        while self.now < end {
            self.now = (self.now + 10_000).min(end);
            for c in conns {
                self.msg(*c, kinds::HEARTBEAT, json!({"at": self.now}));
            }
            self.poll();
        }
    }

    fn item_for_url(&self, url: &str) -> decay_store::Item {
        let page = self
            .store()
            .live_items()
            .unwrap()
            .into_iter()
            .chain(archived(self.store()))
            .find(|i| i.url.as_deref() == Some(url));
        page.unwrap_or_else(|| panic!("no item for {url}"))
    }
}

fn archived(store: &Store) -> Vec<decay_store::Item> {
    store
        .search(&decay_store::SearchQuery {
            text: String::new(),
            filters: decay_store::SearchFilters::default(),
            limit: 200,
            offset: 0,
            now: T0 + 1_000_000_000,
            utc_offset_minutes: 0,
        })
        .unwrap()
        .hits
        .into_iter()
        .map(|h| h.item)
        .collect()
}

fn events_of(store: &Store, item: &str) -> Vec<EventType> {
    let mut e: Vec<EventType> =
        store.events_for_item(item, 100).unwrap().into_iter().map(|e| e.kind).collect();
    e.reverse();
    e
}

/// Three tabs in window 1: an old page, the active tab, and a pinned one.
fn three_tabs() -> Vec<TabInfo> {
    let mut active = tab(2, 1, "https://active.example/");
    active.active = true;
    let mut pinned = tab(3, 1, "https://pinned.example/");
    pinned.pinned = true;
    vec![tab(1, 1, "https://old.example/page"), active, pinned]
}

#[test]
fn handshake_sends_welcome_then_config() {
    let mut h = Harness::new();
    let out = h.engine.handle(Input::Connected(1), h.now);
    h.collect(out);
    let hello = Envelope::new(
        kinds::HELLO,
        &json!({"extVersion": "0.1.0", "browser": "edge", "profileId": "p", "session": "s"}),
    )
    .unwrap();
    let hello_id = hello.id.clone();
    h.frame(1, "p", hello);
    let welcome = h.one(1, kinds::WELCOME);
    assert_eq!(welcome.reply_to.as_deref(), Some(hello_id.as_str()));
    assert_eq!(welcome.payload["mode"], "shadow");
    assert_eq!(welcome.payload["config"]["heartbeatSec"], 20);
    let config: Config = h.one(1, kinds::CONFIG).payload_as().unwrap();
    assert!(config.probe_enabled);
    assert!(matches!(
        h.events[0],
        EngineEvent::AdapterConnected { browser: BrowserKind::Edge, .. }
    ));
    // Edge is registered under the platform's catalog id.
    assert!(h.store().app("com.microsoft.edgemac").unwrap().is_some());
}

#[test]
fn an_outdated_extension_is_told_and_dropped() {
    let mut h = Harness::new();
    h.connect_as(1, "p", "s", "0.0.9");
    let err = h.one(1, kinds::ERROR);
    assert_eq!(err.payload["code"], "VERSION_MISMATCH");
    assert_eq!(h.closed, vec![1]);
    // Nothing it says afterwards is used.
    h.msg(1, kinds::SNAPSHOT, serde_json::to_value(snapshot(three_tabs())).unwrap());
    assert_eq!(h.store().live_items().unwrap(), [] as [decay_store::Item; 0]);
}

#[test]
fn excluded_domains_reach_the_extension_and_other_exclusions_are_applied_here() {
    let mut h = Harness::new();
    let rule = |id: &str, scope: &str, pattern: &str| decay_core::Rule {
        id: id.into(),
        scope: serde_json::from_value(json!(scope)).unwrap(),
        pattern: pattern.into(),
        action: decay_core::RuleAction::Exclude,
        threshold_hours: None,
        enabled: true,
        source: decay_core::RuleSource::User,
        created_at: 0,
        hit_count: 0,
    };
    h.store().insert_rule(&rule("x1", "domain", "bank.example")).unwrap();
    h.store().insert_rule(&rule("x2", "titleRegex", "secret")).unwrap();
    let out = h.engine.handle(Input::Connected(1), h.now);
    h.collect(out);
    h.msg(
        1,
        kinds::HELLO,
        json!({"extVersion": "0.1.0", "browser": "chrome", "profileId": "profile1"}),
    );
    let config: Config = h.one(1, kinds::CONFIG).payload_as().unwrap();
    assert_eq!(config.excluded_domains, vec!["bank.example".to_owned()]);

    let mut secret = tab(2, 1, "https://a.example/");
    secret.title = "My secret plan".into();
    let mut loading = tab(3, 1, "https://b.example/");
    loading.title = String::new();
    loading.status = TabStatus::Loading;
    h.msg(
        1,
        kinds::SNAPSHOT,
        serde_json::to_value(snapshot(vec![tab(1, 1, "https://ok.example/"), secret, loading]))
            .unwrap(),
    );
    let live = h.store().live_items().unwrap();
    // The excluded title is never stored; the loading tab waits for its title.
    assert_eq!(live.len(), 1);
    h.msg(
        1,
        kinds::TAB_UPDATED,
        json!({"tabId": 3, "changes": {"title": "Secret recipe", "status": "complete"}}),
    );
    assert_eq!(h.store().live_items().unwrap().len(), 1);
    h.msg(1, kinds::TAB_UPDATED, json!({"tabId": 2, "changes": {"title": "Public now"}}));
    assert_eq!(h.store().live_items().unwrap().len(), 2);
}

#[test]
fn tabs_age_only_while_the_user_is_active_and_not_when_in_use() {
    let mut h = Harness::new();
    h.connect(1, "s");
    h.msg(1, kinds::SNAPSHOT, serde_json::to_value(snapshot(three_tabs())).unwrap());
    // No browser activity yet: idle, nothing ages.
    h.advance(120_000, &[1]);
    assert_eq!(h.item_for_url("https://old.example/page").active_age_s, 0);
    // Activity: the active tab is used, the others age.
    h.msg(
        1,
        kinds::TAB_USED,
        json!({"tabId": 2, "windowId": 1, "at": h.now, "signal": "activated"}),
    );
    h.advance(90_000, &[1]);
    let old = h.item_for_url("https://old.example/page").active_age_s;
    assert!((60..=90).contains(&old), "aged {old}");
    assert_eq!(h.item_for_url("https://active.example/").active_age_s, 0);
    // Idle past the cutoff (300 s): no more credit.
    h.advance(400_000, &[1]);
    let later = h.item_for_url("https://old.example/page").active_age_s;
    assert!(later < old + 330, "{later} vs {old}");
    // A disconnected browser's tabs are frozen.
    let out = h.engine.handle(Input::Disconnected(1), h.now);
    h.collect(out);
    h.advance(120_000, &[]);
    assert_eq!(h.item_for_url("https://old.example/page").active_age_s, later);
}

/// Connected, automatic mode, one old web tab past the threshold, quiet.
fn ready_to_sweep(mode: Mode) -> Harness {
    let mut h = Harness::new();
    h.set_mode(mode);
    h.connect(1, "s");
    h.msg(1, kinds::SNAPSHOT, serde_json::to_value(snapshot(three_tabs())).unwrap());
    h.engine.dev_add_age(61 * H, h.now);
    h
}

#[test]
fn a_sweep_probes_writes_before_closing_and_archives_on_ack() {
    let mut h = ready_to_sweep(Mode::Automatic);
    h.poll(); // the tick runs now (dev_add_age moved it forward)
    // Only the old web page is a candidate: probed first.
    let probe = h.one(1, kinds::PROBE);
    assert_eq!(probe.payload["tabIds"], json!([1]));
    h.ack_ok(1, &probe, json!({"results": [{"tabId": 1, "dirtyForm": false, "beforeUnload": false, "uploading": false}], "missing": []}));
    let close = h.one(1, kinds::TABS_CLOSE);
    assert_eq!(close.payload["tabIds"], json!([1]));
    assert_eq!(close.payload["urls"]["1"], "https://old.example/page");
    // Write before close: the record and the sweep exist before any tab closes.
    let item = h.item_for_url("https://old.example/page");
    assert_eq!(item.state, ItemState::Archiving);
    let sweep_id = close.payload["sweepId"].as_str().unwrap().to_owned();
    assert_eq!(h.store().sweep(&sweep_id).unwrap().unwrap().status, SweepStatus::Running);

    h.ack_ok(1, &close, json!({"closed": [1], "skipped": [], "unchecked": []}));
    let item = h.store().item(&item.id).unwrap().unwrap();
    assert_eq!(item.state, ItemState::Archived);
    assert_eq!(item.last_sweep_id.as_deref(), Some(sweep_id.as_str()));
    assert!(matches!(
        item.archive_reason,
        Some(decay_core::Reason::Unused { active_hours: 61, .. })
    ));
    let sweep = h.store().sweep(&sweep_id).unwrap().unwrap();
    assert_eq!((sweep.status, sweep.item_count, sweep.failed_count), (SweepStatus::Done, 1, 0));
    assert!(
        h.events
            .iter()
            .any(|e| matches!(e, EngineEvent::SweepCompleted { archived: 1, shadow: false, .. }))
    );
    assert_eq!(events_of(h.store(), &item.id).last(), Some(&EventType::Archived));
    // The tab's removal event (closed by Decay) changes nothing.
    h.msg(1, kinds::TAB_REMOVED, json!({"tabId": 1, "byUser": false}));
    assert_eq!(h.store().item(&item.id).unwrap().unwrap().state, ItemState::Archived);
    // Not again for 15 minutes.
    h.engine.dev_add_age(100 * H, h.now);
    h.advance(5 * 60_000, &[1]);
    assert!(h.take(1, kinds::PROBE).is_empty() && h.take(1, kinds::TABS_CLOSE).is_empty());
}

#[test]
fn a_tab_skipped_because_it_is_in_use_goes_back_and_gets_younger() {
    let mut h = ready_to_sweep(Mode::Automatic);
    h.poll(); // the tick runs now (dev_add_age moved it forward)
    let probe = h.one(1, kinds::PROBE);
    h.ack_ok(1, &probe, json!({"results": [{"tabId": 1, "dirtyForm": false, "beforeUnload": false, "uploading": false}], "missing": []}));
    let close = h.one(1, kinds::TABS_CLOSE);
    // The user switched to it while the close was on its way.
    h.msg(
        1,
        kinds::TAB_USED,
        json!({"tabId": 1, "windowId": 1, "at": h.now, "signal": "activated"}),
    );
    h.ack_ok(
        1,
        &close,
        json!({"closed": [], "skipped": [{"tabId": 1, "reason": "inUse"}], "unchecked": []}),
    );
    let item = h.item_for_url("https://old.example/page");
    assert_eq!((item.state, item.active_age_s), (ItemState::Active, 0));
    let sweep_id = close.payload["sweepId"].as_str().unwrap();
    let sweep = h.store().sweep(sweep_id).unwrap().unwrap();
    assert_eq!((sweep.status, sweep.item_count, sweep.failed_count), (SweepStatus::Done, 0, 1));
    assert!(events_of(h.store(), &item.id).contains(&EventType::CloseFailed));
}

#[test]
fn typed_input_protects_and_unknown_pages_wait_twice_as_long() {
    let mut h = ready_to_sweep(Mode::Automatic);
    h.poll(); // the tick runs now (dev_add_age moved it forward)
    let probe = h.one(1, kinds::PROBE);
    h.ack_ok(1, &probe, json!({"results": [{"tabId": 1, "dirtyForm": false, "beforeUnload": false, "uploading": false, "unknown": true}], "missing": []}));
    // Unknown at 61 h: waits for 120 h.
    assert_eq!(h.take(1, kinds::TABS_CLOSE), [] as [decay_bridge::Envelope; 0]);
    assert_eq!(h.item_for_url("https://old.example/page").state, ItemState::Stale);

    let mut h = ready_to_sweep(Mode::Automatic);
    h.poll(); // the tick runs now (dev_add_age moved it forward)
    let probe = h.one(1, kinds::PROBE);
    h.ack_ok(1, &probe, json!({"results": [{"tabId": 1, "dirtyForm": true, "beforeUnload": false, "uploading": false}], "missing": []}));
    assert_eq!(h.take(1, kinds::TABS_CLOSE), [] as [decay_bridge::Envelope; 0]);
    // No answer at all counts as unknown too.
    let mut h = ready_to_sweep(Mode::Automatic);
    h.engine.dev_add_age(60 * H, h.now); // 121 h
    h.poll(); // the tick runs now (dev_add_age moved it forward)
    let _probe = h.one(1, kinds::PROBE);
    h.advance(20_000, &[1]); // the probe times out
    let close = h.one(1, kinds::TABS_CLOSE);
    assert_eq!(close.payload["tabIds"], json!([1]));
}

#[test]
fn shadow_mode_only_records_what_it_would_archive() {
    let mut h = ready_to_sweep(Mode::Shadow);
    h.poll(); // the tick runs now (dev_add_age moved it forward)
    let probe = h.one(1, kinds::PROBE);
    h.ack_ok(1, &probe, json!({"results": [{"tabId": 1, "dirtyForm": false, "beforeUnload": false, "uploading": false}], "missing": []}));
    assert_eq!(h.take(1, kinds::TABS_CLOSE), [] as [decay_bridge::Envelope; 0]);
    let item = h.item_for_url("https://old.example/page");
    assert_eq!(item.state, ItemState::Stale);
    assert!(events_of(h.store(), &item.id).contains(&EventType::ShadowWouldArchive));
    let sweep = &h.store().recent_sweeps(1).unwrap()[0];
    assert!(sweep.is_shadow && sweep.status == SweepStatus::Done && sweep.item_count == 1);
}

#[test]
fn a_hold_or_pause_stops_sweeps_and_sweep_now_says_why() {
    let mut h = ready_to_sweep(Mode::Automatic);
    h.store().set_setting("hold_until", &(h.now + 3_600_000), h.now).unwrap();
    h.advance(2 * timing::TICK_MS, &[1]);
    assert_eq!(h.take(1, kinds::PROBE), [] as [decay_bridge::Envelope; 0]);
    assert!(matches!(h.engine.sweep_now(h.now), Err(decay_engine::SweepRefused::OnHold(_))));
    h.store().set_setting("hold_until", &Value::Null, h.now).unwrap();
    h.set_mode(Mode::Paused);
    assert_eq!(h.engine.sweep_now(h.now), Err(decay_engine::SweepRefused::Paused));
}

#[test]
fn an_unanswered_close_is_settled_by_the_next_snapshot() {
    for still_open in [false, true] {
        let mut h = ready_to_sweep(Mode::Automatic);
        h.poll(); // the tick runs now (dev_add_age moved it forward)
        let probe = h.one(1, kinds::PROBE);
        h.ack_ok(1, &probe, json!({"results": [{"tabId": 1, "dirtyForm": false, "beforeUnload": false, "uploading": false}], "missing": []}));
        let close = h.one(1, kinds::TABS_CLOSE);
        let sweep_id = close.payload["sweepId"].as_str().unwrap().to_owned();
        h.advance(30_000, &[1]); // the close is late…
        assert_eq!(h.item_for_url("https://old.example/page").state, ItemState::Archiving);
        h.advance(timing::LATE_CLOSE_MS, &[1]); // …and then lost
        // Unknown state: never closed again, never reverted blindly.
        assert_eq!(h.item_for_url("https://old.example/page").state, ItemState::Archiving);
        let resync = h.take(1, kinds::RESYNC).pop().expect("a resync");

        h.ack_ok(1, &resync, json!(null));
        let tabs = if still_open { three_tabs() } else { three_tabs()[1..].to_vec() };
        h.msg(1, kinds::SNAPSHOT, serde_json::to_value(snapshot(tabs)).unwrap());
        let item = h.item_for_url("https://old.example/page");
        let want = if still_open { ItemState::Stale } else { ItemState::Archived };
        assert_eq!(item.state, want, "still open: {still_open}");
        assert_eq!(h.store().sweep(&sweep_id).unwrap().unwrap().status, SweepStatus::Done);
    }
}

#[test]
fn after_a_crash_archiving_items_settle_when_the_browser_reconnects() {
    let mut h = ready_to_sweep(Mode::Automatic);
    h.poll(); // the tick runs now (dev_add_age moved it forward)
    let probe = h.one(1, kinds::PROBE);
    h.ack_ok(1, &probe, json!({"results": [{"tabId": 1, "dirtyForm": false, "beforeUnload": false, "uploading": false}], "missing": []}));
    let close = h.one(1, kinds::TABS_CLOSE);
    let sweep_id = close.payload["sweepId"].as_str().unwrap().to_owned();
    // The app dies before the ack: a new engine over the same database.
    let Harness { engine, dir, .. } = h;
    drop(engine.into_store());
    let store = open_store(dir.path(), T0 + 60_000);
    let engine =
        Engine::new(store, EngineConfig { platform: Platform::MacOs, pro: false }, T0 + 60_000)
            .unwrap();
    let mut h = Harness {
        engine,
        now: T0 + 60_000,
        sent: Vec::new(),
        closed: Vec::new(),
        events: Vec::new(),
        dir,
    };
    assert_eq!(h.store().unfinished_sweeps().unwrap().len(), 1);
    h.connect(1, "s");
    // The tab was closed before the crash.
    h.msg(1, kinds::SNAPSHOT, serde_json::to_value(snapshot(three_tabs()[1..].to_vec())).unwrap());
    let item = h.item_for_url("https://old.example/page");
    assert_eq!(item.state, ItemState::Archived);
    assert_eq!(h.store().sweep(&sweep_id).unwrap().unwrap().status, SweepStatus::Done);
}

#[test]
fn ages_survive_a_browser_restart_with_new_tab_ids() {
    let mut h = Harness::new();
    h.connect(1, "s1");
    h.msg(
        1,
        kinds::SNAPSHOT,
        serde_json::to_value(snapshot(vec![
            tab(1, 1, "https://a.example/"),
            tab(2, 1, "https://b.example/"),
        ]))
        .unwrap(),
    );
    h.engine.dev_add_age(5 * H, h.now);
    let a = h.item_for_url("https://a.example/");
    let out = h.engine.handle(Input::Disconnected(1), h.now);
    h.collect(out);
    // New browser session, same profile: ids swapped (b got 1, a got 2), a new tab 3.
    h.connect_as(3, "profile1", "s2", "0.1.0");
    let profile = "profile1";
    let snap = snapshot(vec![
        tab(1, 7, "https://b.example/"),
        tab(2, 7, "https://a.example/"),
        tab(3, 7, "https://c.example/"),
    ]);
    h.frame(3, profile, Envelope::new(kinds::SNAPSHOT, &snap).unwrap());
    let a2 = h.item_for_url("https://a.example/");
    assert_eq!(a2.id, a.id);
    assert_eq!(a2.active_age_s, 5 * H);
    assert_eq!(a2.external_id.as_deref(), Some("profile1:2"));
    assert_eq!(h.item_for_url("https://c.example/").active_age_s, 0);
    assert_eq!(h.store().live_items().unwrap().len(), 3);
}

#[test]
fn a_closed_tab_is_forgotten_unless_it_comes_back_within_a_minute() {
    let mut h = Harness::new();
    h.connect(1, "s");
    h.msg(
        1,
        kinds::SNAPSHOT,
        serde_json::to_value(snapshot(vec![
            tab(1, 1, "https://a.example/"),
            tab(2, 1, "https://b.example/"),
        ]))
        .unwrap(),
    );
    h.engine.dev_add_age(3 * H, h.now);
    let a = h.item_for_url("https://a.example/");
    // Closed, then reopened ("reopen closed tab"): the item comes back with its age.
    h.msg(1, kinds::TAB_REMOVED, json!({"tabId": 1, "byUser": true}));
    h.msg(1, kinds::TAB_CREATED, serde_json::to_value(tab(9, 1, "https://a.example/")).unwrap());
    let back = h.item_for_url("https://a.example/");
    assert_eq!((back.id.as_str(), back.active_age_s), (a.id.as_str(), 3 * H));
    // Closed for good: deleted after a minute and a fresh snapshot.
    h.msg(1, kinds::TAB_REMOVED, json!({"tabId": 2, "byUser": true}));
    let b = h
        .store()
        .live_items()
        .unwrap()
        .into_iter()
        .find(|i| i.url.as_deref() == Some("https://b.example/"))
        .unwrap();
    h.advance(timing::MISSING_GRACE_MS + 1_000, &[1]);
    let resync = h.take(1, kinds::RESYNC);
    assert_ne!(resync, [] as [decay_bridge::Envelope; 0]);
    h.msg(
        1,
        kinds::SNAPSHOT,
        serde_json::to_value(snapshot(vec![tab(9, 1, "https://a.example/")])).unwrap(),
    );
    assert_eq!(h.store().item(&b.id).unwrap(), None);
    assert!(h.store().item(&a.id).unwrap().is_some());
}

#[test]
fn two_profiles_of_one_browser_keep_their_tabs_apart() {
    let mut h = Harness::new();
    h.connect(1, "s1");
    h.connect(2, "s2");
    h.msg(
        1,
        kinds::SNAPSHOT,
        serde_json::to_value(snapshot(vec![tab(1, 1, "https://one.example/")])).unwrap(),
    );
    h.msg(
        2,
        kinds::SNAPSHOT,
        serde_json::to_value(snapshot(vec![tab(1, 1, "https://two.example/")])).unwrap(),
    );
    assert_eq!(h.store().live_items().unwrap().len(), 2);
    // Profile 2's snapshot didn't touch profile 1's item.
    h.msg(2, kinds::SNAPSHOT, serde_json::to_value(snapshot(vec![])).unwrap());
    assert!(h.item_for_url("https://one.example/").state.is_live());
}

#[test]
fn a_newer_connection_for_the_same_profile_replaces_the_old_one() {
    let mut h = Harness::new();
    h.connect_as(1, "p", "s", "0.1.0");
    h.connect_as(2, "p", "s", "0.1.0");
    assert_eq!(h.closed, vec![1]);
}

#[test]
fn a_tab_the_user_closes_during_the_sweep_is_not_archived() {
    let mut h = ready_to_sweep(Mode::Automatic);
    h.poll(); // the tick runs now (dev_add_age moved it forward)
    let probe = h.one(1, kinds::PROBE);
    h.ack_ok(1, &probe, json!({"results": [{"tabId": 1, "dirtyForm": false, "beforeUnload": false, "uploading": false}], "missing": []}));
    let close = h.one(1, kinds::TABS_CLOSE);
    let id = h.item_for_url("https://old.example/page").id;
    h.msg(1, kinds::TAB_REMOVED, json!({"tabId": 1, "byUser": true}));
    h.ack_ok(1, &close, json!({"closed": [1], "skipped": [], "unchecked": []}));
    assert_eq!(h.store().item(&id).unwrap(), None);
}

/// A sweep that archived tabs 1 and 4 (window 1, indexes 1 and 4, one in a group).
fn swept() -> (Harness, String) {
    let mut h = Harness::new();
    h.set_mode(Mode::Automatic);
    h.connect(1, "sess");
    let mut tabs = three_tabs();
    let mut grouped = tab(4, 1, "chrome://settings/");
    grouped.group_id = 10;
    tabs.push(grouped);
    let mut snap = snapshot(tabs);
    snap.groups.push(decay_bridge::protocol::GroupInfo {
        id: 10,
        title: "Work".into(),
        color: "blue".into(),
        window_id: 1,
    });
    h.msg(1, kinds::SNAPSHOT, serde_json::to_value(snap).unwrap());
    h.engine.dev_add_age(70 * H, h.now);
    h.poll();
    let probe = h.one(1, kinds::PROBE);
    h.ack_ok(1, &probe, json!({"results": [{"tabId": 1, "dirtyForm": false, "beforeUnload": false, "uploading": false}], "missing": []}));
    let close = h.one(1, kinds::TABS_CLOSE);
    let mut ids = close.payload["tabIds"].as_array().unwrap().clone();
    ids.sort_by_key(serde_json::Value::as_i64);
    assert_eq!(ids, vec![json!(1), json!(4)]);
    h.ack_ok(1, &close, json!({"closed": [1, 4], "skipped": [], "unchecked": []}));
    let sweep = close.payload["sweepId"].as_str().unwrap().to_owned();
    (h, sweep)
}

#[test]
fn undo_reopens_the_sweep_in_its_window_and_protects_the_tabs() {
    let (mut h, sweep_id) = swept();
    let req = h.request(1, kinds::ARCHIVE_UNDO, json!({}));
    let restore = h.one(1, kinds::TABS_RESTORE);
    let items = restore.payload["items"].as_array().unwrap().clone();
    assert_eq!(items.len(), 2);
    assert!(items.iter().all(|i| i["windowKey"] == "sess:1"));
    let grouped = items.iter().find(|i| i["url"] == "chrome://settings/").unwrap();
    assert_eq!(grouped["group"], json!({"title": "Work", "color": "blue"}));
    assert_eq!(grouped["index"], 4);
    let ids: Vec<String> = items.iter().map(|i| i["itemId"].as_str().unwrap().to_owned()).collect();
    // One tab reported before the ack, one after.
    h.msg(
        1,
        kinds::TAB_CREATED,
        serde_json::to_value(tab(20, 1, items[0]["url"].as_str().unwrap())).unwrap(),
    );
    h.ack_ok(1, &restore, json!({"restored": [{"itemId": ids[0], "tabId": 20}, {"itemId": ids[1], "tabId": 21}], "failed": []}));
    h.msg(
        1,
        kinds::TAB_CREATED,
        serde_json::to_value(tab(21, 1, items[1]["url"].as_str().unwrap())).unwrap(),
    );
    let reply = h.reply_to(1, &req);
    assert_eq!(reply.payload["ok"], true);
    assert_eq!(reply.payload["result"], json!({"restored": 2, "failed": 0, "skipped": 0}));
    for id in &ids {
        let old = h.store().item(id).unwrap().unwrap();
        assert_eq!((old.state, old.restore_count), (ItemState::Restored, 1));
        assert!(events_of(h.store(), id).contains(&EventType::Undo));
    }
    let live = h.store().live_items().unwrap();
    for id in &ids {
        let new = live
            .iter()
            .find(|i| i.restored_from_id.as_deref() == Some(id.as_str()))
            .expect("linked");
        assert_eq!(new.grace_until, Some(h.now + timing::RESTORE_GRACE_MS));
    }
    let sweep = h.store().sweep(&sweep_id).unwrap().unwrap();
    assert_eq!(sweep.status, SweepStatus::Undone);
    // Nothing left to undo.
    let again = h.request(1, kinds::ARCHIVE_UNDO, json!({"sweepId": sweep_id}));
    assert_eq!(h.reply_to(1, &again).payload["error"]["code"], "NOT_FOUND");
}

#[test]
fn a_partial_undo_can_be_retried_for_the_rest() {
    let (mut h, sweep_id) = swept();
    let req = h.request(1, kinds::ARCHIVE_UNDO, json!({"sweepId": sweep_id}));
    let restore = h.one(1, kinds::TABS_RESTORE);
    let first = restore.payload["items"][0]["itemId"].as_str().unwrap().to_owned();
    let second = restore.payload["items"][1]["itemId"].as_str().unwrap().to_owned();
    h.ack_ok(
        1,
        &restore,
        json!({"restored": [{"itemId": first, "tabId": 30}], "failed": [second]}),
    );
    assert_eq!(
        h.reply_to(1, &req).payload["result"],
        json!({"restored": 1, "failed": 1, "skipped": 0})
    );
    assert_eq!(h.store().sweep(&sweep_id).unwrap().unwrap().status, SweepStatus::Done);
    h.request(1, kinds::ARCHIVE_UNDO, json!({"sweepId": sweep_id}));
    let retry = h.one(1, kinds::TABS_RESTORE);
    assert_eq!(retry.payload["items"].as_array().unwrap().len(), 1);
    assert_eq!(retry.payload["items"][0]["itemId"], second.as_str());
}

#[test]
fn search_and_restore_through_the_app() {
    let (mut h, sweep_id) = swept();
    let req = h.request(1, kinds::ARCHIVE_SEARCH, json!({"query": ""}));
    let reply = h.reply_to(1, &req);
    let result = &reply.payload["result"];
    assert_eq!(result["items"].as_array().unwrap().len(), 2);
    assert_eq!(result["items"][0]["windowKey"], "sess:1");
    assert_eq!(result["lastSweep"]["id"], sweep_id.as_str());
    assert_eq!(result["lastSweep"]["remaining"], 2);

    let req = h.request(1, kinds::ARCHIVE_SEARCH, json!({"query": "old"}));
    let reply = h.reply_to(1, &req);
    let items = reply.payload["result"]["items"].as_array().unwrap().clone();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["url"], "https://old.example/page");
    assert!(reply.payload["result"].get("lastSweep").is_none());

    // Enter: into the current window, active.
    let id = items[0]["id"].as_str().unwrap().to_owned();
    let req = h.request(
        1,
        kinds::ARCHIVE_RESTORE,
        json!({"itemIds": [id, "missing-id"], "intoWindowId": 5}),
    );
    let restore = h.one(1, kinds::TABS_RESTORE);
    assert_eq!(restore.payload["items"][0]["windowKey"], "sess:5");
    assert_eq!(restore.payload["items"][0]["active"], true);
    assert!(restore.payload["items"][0].get("index").is_none());
    h.ack_ok(1, &restore, json!({"restored": [{"itemId": id, "tabId": 40}], "failed": []}));
    assert_eq!(
        h.reply_to(1, &req).payload["result"],
        json!({"restored": 1, "failed": 0, "skipped": 1})
    );
    assert!(events_of(h.store(), &id).contains(&EventType::Restored));

    // Batch: the rest of that sweep's window.
    let other = archived(h.store()).into_iter().next().unwrap();
    let req = h.request(1, kinds::ARCHIVE_RESTORE_BATCH, json!({"itemId": other.id}));
    let restore = h.one(1, kinds::TABS_RESTORE);
    assert_eq!(restore.payload["items"].as_array().unwrap().len(), 1);
    assert_eq!(restore.payload["items"][0]["windowKey"], "sess:1");
    h.ack_ok(1, &restore, json!({"restored": [], "failed": [other.id]}));
    assert_eq!(h.reply_to(1, &req).payload["result"]["failed"], 1);
    assert_eq!(h.store().item(&other.id).unwrap().unwrap().state, ItemState::Archived);
}

#[test]
fn bad_requests_get_an_error_ack() {
    let mut h = Harness::new();
    h.connect(1, "s");
    let req = h.request(1, kinds::ARCHIVE_RESTORE, json!({"nope": 1}));
    let reply = h.reply_to(1, &req);
    assert_eq!(reply.payload["ok"], false);
    assert_eq!(
        reply.payload["error"]["code"],
        serde_json::to_value(ErrorCode::BadMessage).unwrap()
    );
}

#[test]
fn import_archive_is_acked_with_its_report() {
    let mut h = Harness::new();
    h.connect(1, "s");
    let req =
        h.request(1, kinds::IMPORT_ARCHIVE, json!({"items": [], "rules": [], "settings": {}}));
    let reply = h.reply_to(1, &req);
    assert_eq!(reply.payload["ok"], true);
    assert_eq!(reply.payload["result"]["itemsImported"], 0);
}

#[test]
fn chunked_messages_are_reassembled() {
    let mut h = Harness::new();
    h.connect(1, "s");
    let tabs: Vec<TabInfo> = (0..3000)
        .map(|i| tab(i, 1, &format!("https://e.example/{i}/{}", "x".repeat(300))))
        .collect();
    let env = Envelope::new(kinds::SNAPSHOT, &snapshot(tabs)).unwrap();
    let chunks = decay_bridge::chunk::split(&env, decay_bridge::frame::MAX_TO_BROWSER).unwrap();
    assert!(chunks.len() > 1);
    for c in chunks {
        h.frame(1, "profile1", c);
    }
    assert_eq!(h.store().live_items().unwrap().len(), 3000);
}

// --- Review fixes (P1.5 code review) ---------------------------------------------------

fn clean_probe(h: &mut Harness) {
    let probe = h.one(1, kinds::PROBE);
    h.ack_ok(
        1,
        &probe,
        json!({"results": [{"tabId": 1, "dirtyForm": false, "beforeUnload": false, "uploading": false}], "missing": []}),
    );
}

#[test]
fn a_late_close_is_still_in_flight_and_its_ack_still_counts() {
    let mut h = ready_to_sweep(Mode::Automatic);
    h.poll();
    clean_probe(&mut h);
    let close = h.one(1, kinds::TABS_CLOSE);
    let id = h.item_for_url("https://old.example/page").id;
    h.advance(30_000, &[1]); // past the first deadline: late, not lost
    // The resync's snapshot still shows the tab (the extension is still working on it).
    let resync = h.one(1, kinds::RESYNC);
    h.ack_ok(1, &resync, json!(null));
    h.msg(1, kinds::SNAPSHOT, serde_json::to_value(snapshot(three_tabs())).unwrap());
    assert_eq!(h.store().item(&id).unwrap().unwrap().state, ItemState::Archiving);
    // The executor isn't stuck meanwhile.
    assert_ne!(h.engine.sweep_now(h.now), Err(decay_engine::SweepRefused::Busy));
    // The late ack archives it.
    h.ack_ok(1, &close, json!({"closed": [1], "skipped": [], "unchecked": []}));
    assert_eq!(h.store().item(&id).unwrap().unwrap().state, ItemState::Archived);
}

#[test]
fn a_tab_decay_closes_after_its_item_went_back_is_archived_not_forgotten() {
    let mut h = ready_to_sweep(Mode::Automatic);
    h.poll();
    clean_probe(&mut h);
    let close = h.one(1, kinds::TABS_CLOSE);
    let sweep_id = close.payload["sweepId"].as_str().unwrap().to_owned();
    let id = h.item_for_url("https://old.example/page").id;
    // The link drops mid-close and comes back; the first snapshot still has the tab.
    let out = h.engine.handle(Input::Disconnected(1), h.now);
    h.collect(out);
    h.connect_as(2, "profile1", "s", "0.1.0");
    h.frame(2, "profile1", Envelope::new(kinds::SNAPSHOT, &snapshot(three_tabs())).unwrap());
    assert_eq!(h.store().item(&id).unwrap().unwrap().state, ItemState::Stale);
    // Then the old close loop's removal lands: Decay closed it.
    h.frame(
        2,
        "profile1",
        Envelope::new(kinds::TAB_REMOVED, &json!({"tabId": 1, "byUser": false})).unwrap(),
    );
    h.advance(timing::MISSING_GRACE_MS + 5_000, &[]);
    h.frame(
        2,
        "profile1",
        Envelope::new(kinds::SNAPSHOT, &snapshot(three_tabs()[1..].to_vec())).unwrap(),
    );
    let item = h.store().item(&id).unwrap().expect("the record is kept");
    assert_eq!(item.state, ItemState::Archived);
    assert_eq!(item.last_sweep_id.as_deref(), Some(sweep_id.as_str()));
    assert!(item.archive_reason.is_some());
    let sweep = h.store().sweep(&sweep_id).unwrap().unwrap();
    assert_eq!((sweep.status, sweep.item_count), (SweepStatus::Done, 1));
}

#[test]
fn a_new_url_or_title_exclusion_applies_to_tabs_already_tracked() {
    let mut h = ready_to_sweep(Mode::Automatic);
    let id = h.item_for_url("https://old.example/page").id;
    let req = h.request(
        1,
        kinds::IMPORT_ARCHIVE,
        json!({"items": [], "settings": {}, "rules": [{"id": "x", "scope": "urlGlob", "match": "https://old.example/*",
            "action": "exclude", "enabled": true, "source": "user", "createdAt": 0, "hitCount": 0}]}),
    );
    assert_eq!(h.reply_to(1, &req).payload["result"]["rulesImported"], 1);
    h.poll();
    assert_eq!(h.take(1, kinds::PROBE), [] as [decay_bridge::Envelope; 0]);
    assert_eq!(h.take(1, kinds::TABS_CLOSE), [] as [decay_bridge::Envelope; 0]);
    assert_eq!(h.store().item(&id).unwrap(), None, "excluded items aren't stored");
}

#[test]
fn undo_waits_for_a_sweep_that_is_still_closing() {
    let mut h = ready_to_sweep(Mode::Automatic);
    h.poll();
    clean_probe(&mut h);
    let close = h.one(1, kinds::TABS_CLOSE);
    let sweep_id = close.payload["sweepId"].as_str().unwrap().to_owned();
    let req = h.request(1, kinds::ARCHIVE_UNDO, json!({"sweepId": sweep_id}));
    assert_eq!(h.reply_to(1, &req).payload["error"]["code"], "NOT_FOUND");
    let req = h.request(1, kinds::ARCHIVE_UNDO, json!({}));
    assert_eq!(h.reply_to(1, &req).payload["error"]["code"], "NOT_FOUND");
    assert_eq!(h.store().sweep(&sweep_id).unwrap().unwrap().status, SweepStatus::Running);
    h.ack_ok(1, &close, json!({"closed": [1], "skipped": [], "unchecked": []}));
    assert!(h.events.iter().any(|e| matches!(e, EngineEvent::SweepCompleted { archived: 1, .. })));
    h.request(1, kinds::ARCHIVE_UNDO, json!({}));
    assert_eq!(h.one(1, kinds::TABS_RESTORE).payload["items"].as_array().unwrap().len(), 1);
}

#[test]
fn a_replaced_connection_is_told_to_go() {
    let mut h = Harness::new();
    h.connect_as(1, "p", "s", "0.1.0");
    h.connect_as(2, "p", "s", "0.1.0");
    let bye = h.one(1, kinds::ERROR);
    assert_eq!(bye.payload["code"], "APP_UNAVAILABLE");
    assert_eq!(h.closed, vec![1]);
}

#[test]
fn an_item_is_restored_once_even_when_asked_twice() {
    let (mut h, _) = swept();
    let item = archived(h.store()).into_iter().next().unwrap();
    let other = archived(h.store()).into_iter().nth(1).unwrap();
    let first = h.request(
        1,
        kinds::ARCHIVE_RESTORE,
        json!({"itemIds": [item.id, other.id, item.id], "intoWindowId": 1}),
    );
    let restore = h.one(1, kinds::TABS_RESTORE);
    assert_eq!(restore.payload["items"].as_array().unwrap().len(), 2);
    // A second request while the first is on its way: nothing sent again.
    let second =
        h.request(1, kinds::ARCHIVE_RESTORE, json!({"itemIds": [item.id], "intoWindowId": 1}));
    assert_eq!(h.take(1, kinds::TABS_RESTORE), [] as [decay_bridge::Envelope; 0]);
    assert_eq!(
        h.reply_to(1, &second).payload["result"],
        json!({"restored": 0, "failed": 0, "skipped": 1})
    );
    h.ack_ok(
        1,
        &restore,
        json!({"restored": [{"itemId": item.id, "tabId": 50}, {"itemId": other.id, "tabId": 51}], "failed": []}),
    );
    assert_eq!(h.reply_to(1, &first).payload["result"]["restored"], 2);
}

#[test]
fn pages_planned_on_a_clean_check_must_still_be_checkable_when_closed() {
    let mut h = ready_to_sweep(Mode::Automatic);
    h.poll();
    clean_probe(&mut h);
    let close = h.one(1, kinds::TABS_CLOSE);
    assert_eq!(close.payload["requireChecked"], json!([1]));
    h.ack_ok(
        1,
        &close,
        json!({"closed": [], "skipped": [{"tabId": 1, "reason": "unchecked"}], "unchecked": []}),
    );
    assert_eq!(h.item_for_url("https://old.example/page").state, ItemState::Stale);
}

#[test]
fn a_probe_that_finds_tabs_gone_asks_for_a_snapshot() {
    let mut h = ready_to_sweep(Mode::Automatic);
    h.poll();
    let probe = h.one(1, kinds::PROBE);
    h.ack_ok(1, &probe, json!({"results": [], "missing": [1]}));
    h.poll();
    assert_eq!(h.take(1, kinds::RESYNC).len(), 1);
}

#[test]
fn a_pause_during_the_probe_stops_the_sweep() {
    let mut h = ready_to_sweep(Mode::Automatic);
    h.poll();
    let probe = h.one(1, kinds::PROBE);
    h.set_mode(Mode::Paused);
    h.ack_ok(1, &probe, json!({"results": [{"tabId": 1, "dirtyForm": false, "beforeUnload": false, "uploading": false}], "missing": []}));
    assert_eq!(h.take(1, kinds::TABS_CLOSE), [] as [decay_bridge::Envelope; 0]);
    assert_eq!(h.item_for_url("https://old.example/page").state, ItemState::Stale);
}

// --- QA fixes ---------------------------------------------------------------------------

#[test]
fn an_in_use_skip_without_a_real_use_keeps_the_age() {
    // Chrome marks every tab it restores at start-up as just accessed.
    let mut h = ready_to_sweep(Mode::Automatic);
    h.poll();
    clean_probe(&mut h);
    let close = h.one(1, kinds::TABS_CLOSE);
    h.ack_ok(
        1,
        &close,
        json!({"closed": [], "skipped": [{"tabId": 1, "reason": "inUse"}], "unchecked": []}),
    );
    let item = h.item_for_url("https://old.example/page");
    assert_eq!((item.state, item.active_age_s), (ItemState::Stale, 61 * H));
}

#[test]
fn the_first_snapshot_is_asked_for_again_until_it_comes() {
    let mut h = Harness::new();
    h.connect(1, "s"); // the extension's snapshot after config failed (no window open)
    h.advance(timing::FIRST_SNAPSHOT_RETRY_MS + 1_000, &[1]);
    let resync = h.one(1, kinds::RESYNC);
    h.ack_ok(1, &resync, json!(null));
    h.msg(
        1,
        kinds::SNAPSHOT,
        serde_json::to_value(snapshot(vec![tab(1, 1, "https://a.example/")])).unwrap(),
    );
    assert_eq!(h.store().live_items().unwrap().len(), 1);
}

#[test]
fn search_reports_the_apps_mode_and_an_empty_sweep_now_says_so() {
    let mut h = Harness::new();
    h.set_mode(Mode::Automatic);
    h.connect(1, "s");
    h.msg(
        1,
        kinds::SNAPSHOT,
        serde_json::to_value(snapshot(vec![tab(1, 1, "https://a.example/")])).unwrap(),
    );
    let req = h.request(1, kinds::ARCHIVE_SEARCH, json!({"query": ""}));
    assert_eq!(h.reply_to(1, &req).payload["result"]["mode"], "automatic");
    let out = h.engine.sweep_now(h.now).unwrap();
    h.collect(out);
    assert!(h.events.contains(&EngineEvent::NothingToSweep { shadow: false }));
    assert!(!h.events.iter().any(|e| matches!(e, EngineEvent::SweepCompleted { .. })));
}
