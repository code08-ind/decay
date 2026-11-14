//! The fake-extension harness shared by the engine's scenario tests (`engine.rs`) and
//! the "never lose work" safety suite (`safety.rs`).

// Each test file uses a different subset.
#![allow(dead_code, clippy::needless_pass_by_value)]

use decay_bridge::ipc::{ConnInfo, HostFrame};
use decay_bridge::protocol::{
    BrowserKind, Hello, Snapshot, TabInfo, TabStatus, WindowInfo, WindowState,
};
use decay_bridge::{Envelope, kinds};
use decay_core::{EventType, Mode};
use decay_engine::{ConnId, Engine, EngineConfig, EngineEvent, Input, Output};
use decay_store::{DbKey, Platform, Store, StorePaths};
use serde_json::{Value, json};

pub const T0: i64 = 1_793_750_400_000;
pub const H: u64 = 3600;

pub struct Harness {
    pub engine: Engine,
    pub now: i64,
    pub sent: Vec<(ConnId, Envelope)>,
    pub closed: Vec<ConnId>,
    pub events: Vec<EngineEvent>,
    pub dir: tempfile::TempDir,
    /// Every connection speaks for this browser profile (a reconnect of the same
    /// browser); `None`: connection `n` is profile `profile{n}`.
    pub profile: Option<String>,
}

pub fn open_store(dir: &std::path::Path, now: i64) -> Store {
    Store::open(StorePaths::in_dir(dir), DbKey::from_bytes([9; 32]), now).unwrap().0
}

pub fn tab(id: i64, window: i64, url: &str) -> TabInfo {
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

pub fn snapshot(tabs: Vec<TabInfo>) -> Snapshot {
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
    pub fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = open_store(dir.path(), T0);
        let config = EngineConfig { platform: Platform::MacOs, pro: false };
        let engine = Engine::new(store, config, T0).unwrap();
        Self {
            engine,
            now: T0,
            sent: Vec::new(),
            closed: Vec::new(),
            events: Vec::new(),
            dir,
            profile: None,
        }
    }

    pub fn profile_of(&self, conn: ConnId) -> String {
        self.profile.clone().unwrap_or_else(|| format!("profile{conn}"))
    }

    pub fn collect(&mut self, out: Vec<Output>) {
        for o in out {
            match o {
                Output::Send(c, env) => self.sent.push((c, env)),
                Output::Close(c) => self.closed.push(c),
                Output::Event(e) => self.events.push(e),
            }
        }
    }

    pub fn store(&self) -> &Store {
        self.engine.store()
    }

    pub fn set_mode(&mut self, mode: Mode) {
        self.store().set_setting("mode", &mode, self.now).unwrap();
    }

    pub fn frame(&mut self, conn: ConnId, profile: &str, env: Envelope) {
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

    pub fn msg(&mut self, conn: ConnId, kind: &str, payload: Value) {
        let profile = self.profile_of(conn);
        self.frame(conn, &profile, Envelope::new(kind, &payload).unwrap());
    }

    pub fn request(&mut self, conn: ConnId, kind: &str, payload: Value) -> String {
        let env = Envelope::new(kind, &payload).unwrap();
        let id = env.id.clone();
        let profile = self.profile_of(conn);
        self.frame(conn, &profile, env);
        id
    }

    /// Connects `conn` as profile `profile{conn}` and answers the `config`.
    pub fn connect(&mut self, conn: ConnId, session: &str) {
        let profile = self.profile_of(conn);
        self.connect_as(conn, &profile, session, "0.1.0");
    }

    pub fn connect_as(&mut self, conn: ConnId, profile: &str, session: &str, version: &str) {
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

    pub fn ack(&mut self, conn: ConnId, cmd: &Envelope, ack: Value, profile: &str) {
        let mut env = Envelope::new(kinds::ACK, &ack).unwrap();
        env.reply_to = Some(cmd.id.clone());
        self.frame(conn, profile, env);
    }

    pub fn ack_ok(&mut self, conn: ConnId, cmd: &Envelope, result: Value) {
        let profile = self.profile_of(conn);
        self.ack(conn, cmd, json!({"ok": true, "result": result}), &profile);
    }

    pub fn take(&mut self, conn: ConnId, kind: &str) -> Vec<Envelope> {
        let (hit, rest): (Vec<_>, Vec<_>) =
            self.sent.drain(..).partition(|(c, e)| *c == conn && e.kind == kind);
        self.sent = rest;
        hit.into_iter().map(|(_, e)| e).collect()
    }

    pub fn one(&mut self, conn: ConnId, kind: &str) -> Envelope {
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

    pub fn reply_to(&mut self, conn: ConnId, request: &str) -> Envelope {
        let pos = self
            .sent
            .iter()
            .position(|(c, e)| *c == conn && e.reply_to.as_deref() == Some(request))
            .expect("a reply");
        self.sent.remove(pos).1
    }

    pub fn poll(&mut self) {
        let out = self.engine.poll(self.now);
        self.collect(out);
    }

    /// Moves time forward in steps, polling (and answering heartbeats) on the way.
    pub fn advance(&mut self, ms: i64, conns: &[ConnId]) {
        let end = self.now + ms;
        while self.now < end {
            self.now = (self.now + 10_000).min(end);
            for c in conns {
                self.msg(*c, kinds::HEARTBEAT, json!({"at": self.now}));
            }
            self.poll();
        }
    }

    pub fn item_for_url(&self, url: &str) -> decay_store::Item {
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

pub fn archived(store: &Store) -> Vec<decay_store::Item> {
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

pub fn events_of(store: &Store, item: &str) -> Vec<EventType> {
    let mut e: Vec<EventType> =
        store.events_for_item(item, 100).unwrap().into_iter().map(|e| e.kind).collect();
    e.reverse();
    e
}

/// Three tabs in window 1: an old page, the active tab, and a pinned one.
pub fn three_tabs() -> Vec<TabInfo> {
    let mut active = tab(2, 1, "https://active.example/");
    active.active = true;
    let mut pinned = tab(3, 1, "https://pinned.example/");
    pinned.pinned = true;
    vec![tab(1, 1, "https://old.example/page"), active, pinned]
}

/// Connected, automatic mode, one old web tab past the threshold, quiet.
pub fn ready_to_sweep(mode: Mode) -> Harness {
    let mut h = Harness::new();
    h.set_mode(mode);
    h.connect(1, "s");
    h.msg(1, kinds::SNAPSHOT, serde_json::to_value(snapshot(three_tabs())).unwrap());
    h.engine.dev_add_age(61 * H, h.now);
    h
}

pub fn os(idle: u64, key: u64) -> decay_platform::PlatformSample {
    decay_platform::PlatformSample {
        idle_secs: Some(idle),
        secs_since_key: Some(key),
        locked: Some(false),
        display_asleep: Some(false),
        audio_playing: Some(false),
        frontmost: Some(decay_platform::FrontApp { pid: 10, name: "Google Chrome".into() }),
        accessibility: Some(true),
        continuous_ms: None,
    }
}

impl Harness {
    pub fn os(&mut self, sample: decay_platform::PlatformSample) {
        let out = self.engine.handle(Input::Platform(Box::new(sample)), self.now);
        self.collect(out);
    }

    /// Advances in 1-s steps, feeding `sample` (adjusted per step by `f`) before each poll.
    pub fn advance_os(
        &mut self,
        ms: i64,
        conns: &[ConnId],
        f: impl Fn(i64) -> decay_platform::PlatformSample,
    ) {
        let end = self.now + ms;
        while self.now < end {
            self.now = (self.now + 1_000).min(end);
            for c in conns {
                self.msg(*c, kinds::HEARTBEAT, json!({"at": self.now}));
            }
            self.os(f(self.now));
            self.poll();
            // Stop at a probe, so the test answers it before it times out.
            if self.sent.iter().any(|(_, e)| e.kind == kinds::PROBE) {
                break;
            }
        }
    }
}

impl Harness {
    /// The latest published status.
    pub fn status(&self) -> decay_engine::EngineStatus {
        self.events
            .iter()
            .rev()
            .find_map(|e| match e {
                EngineEvent::StatusChanged(s) => Some((**s).clone()),
                _ => None,
            })
            .expect("a status was published")
    }

    pub fn status_events(&self) -> usize {
        self.events.iter().filter(|e| matches!(e, EngineEvent::StatusChanged(_))).count()
    }
}
