//! End-to-end engine scenarios with a fake extension: handshake, tracking, aging, sweeps
//! (probe → write-before-close → close → settle), crash recovery, browser restarts,
//! shadow mode, Undo, search and restore.

// The harness takes JSON by value so calls read like the messages they send.
#![allow(clippy::needless_pass_by_value)]

mod support;

#[allow(clippy::wildcard_imports)]
use support::*;

use decay_bridge::protocol::{BrowserKind, Config, TabInfo, TabStatus};
use decay_bridge::{Envelope, ErrorCode, kinds};
use decay_core::{EventType, ItemState, Mode, Preset, ProtectedBy, SweepStatus, Thresholds};
use decay_engine::{Engine, EngineConfig, EngineEvent, Input, WindowManagement, timing};
use decay_store::Platform;
use serde_json::{Value, json};

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

    // Sweeping again in the same stale period writes nothing new: each tab is reported
    // once, and a manual sweep says there's nothing (new) to sweep.
    h.events.clear();
    let out = h.engine.sweep_now(h.now).unwrap();
    h.collect(out);
    for probe in h.take(1, kinds::PROBE) {
        h.ack_ok(1, &probe, json!({"results": [{"tabId": 1, "dirtyForm": false, "beforeUnload": false, "uploading": false}], "missing": []}));
    }
    let shadow_events = events_of(h.store(), &item.id)
        .into_iter()
        .filter(|e| *e == EventType::ShadowWouldArchive)
        .count();
    assert_eq!(shadow_events, 1);
    assert_eq!(h.store().recent_sweeps(10).unwrap().len(), 1);
    assert!(h.events.iter().any(|e| matches!(e, EngineEvent::NothingToSweep { shadow: true })));
}

#[test]
fn only_a_deliberate_use_of_a_flagged_tab_is_shadow_regret() {
    let mut h = ready_to_sweep(Mode::Shadow);
    h.engine.set_time_zone(jiff::tz::TimeZone::UTC);
    h.poll();
    let probe = h.one(1, kinds::PROBE);
    h.ack_ok(1, &probe, json!({"results": [{"tabId": 1, "dirtyForm": false, "beforeUnload": false, "uploading": false}], "missing": []}));
    let regret = |h: &Harness| h.engine.metrics(0, h.now).unwrap().days.pop().unwrap();
    assert_eq!(regret(&h).shadow_flagged_count, 1);
    // The flagged tab starts playing audio while the user works in another tab: it's in
    // use (doesn't age), but nobody chose it.
    h.msg(1, kinds::TAB_USED, json!({"tabId": 1, "windowId": 1, "at": h.now, "signal": "audible"}));
    h.msg(
        1,
        kinds::TAB_USED,
        json!({"tabId": 2, "windowId": 1, "at": h.now, "signal": "activated"}),
    );
    h.advance(decay_engine::timing::TICK_MS + 1, &[1]);
    h.advance(decay_engine::timing::TICK_MS + 1, &[1]);
    assert_eq!(regret(&h).shadow_regret_count, 0, "audio isn't a regret");
    // Switching to it is.
    h.msg(
        1,
        kinds::TAB_USED,
        json!({"tabId": 1, "windowId": 1, "at": h.now, "signal": "activated"}),
    );
    h.advance(decay_engine::timing::TICK_MS + 1, &[1]);
    assert_eq!(regret(&h).shadow_regret_count, 1);
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
        profile: None,
        window_cmds: Vec::new(),
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

/// P2.8: profiles are recorded on connect, listed with their tabs while connected, and
/// can be named (blank: unnamed again); an unknown one is refused.
#[test]
fn profiles_are_listed_and_named() {
    let mut h = Harness::new();
    h.connect(1, "s1");
    h.now += 1_000;
    h.connect(2, "s2");
    h.msg(
        1,
        kinds::SNAPSHOT,
        serde_json::to_value(snapshot(vec![tab(1, 1, "https://one.example/")])).unwrap(),
    );
    h.msg(2, kinds::SNAPSHOT, serde_json::to_value(snapshot(vec![])).unwrap());
    let list = h.engine.profiles(h.now).unwrap();
    let got: Vec<_> =
        list.iter().map(|p| (p.profile_id.as_str(), p.ordinal, p.connected, p.tabs)).collect();
    assert_eq!(got, [("profile1", 1, true, 1), ("profile2", 2, true, 0)]);
    assert!(list.iter().all(|p| p.has_siblings && p.app_name == "Google Chrome"));
    let app = list[0].app_id.clone();
    let (name, out) = h.engine.rename_profile(&app, "profile2", Some(" Work ")).unwrap().unwrap();
    assert_eq!(name.as_deref(), Some("Work"));
    h.collect(out);
    assert!(h.events.contains(&EngineEvent::ProfilesChanged));
    assert_eq!(h.engine.profiles(h.now).unwrap()[1].name.as_deref(), Some("Work"));
    assert_eq!(
        h.engine.rename_profile(&app, "nope", Some("x")).unwrap().unwrap_err(),
        decay_engine::RenameRefused::UnknownProfile
    );
    // Disconnected: still listed, not connected, seen until then.
    h.now += 5_000;
    let out = h.engine.handle(decay_engine::Input::Disconnected(1), h.now);
    h.collect(out);
    let p1 = &h.engine.profiles(h.now).unwrap()[0];
    assert!(!p1.connected);
    assert_eq!(p1.last_seen_at, h.now);
}

/// P2.8: the popup sees its own profile in a search and can name it.
#[test]
fn the_popup_names_its_own_profile() {
    let mut h = Harness::new();
    h.connect(1, "s1");
    h.msg(1, kinds::SNAPSHOT, serde_json::to_value(snapshot(vec![])).unwrap());
    let req = h.request(1, kinds::ARCHIVE_SEARCH, json!({"query": ""}));
    let reply = h.reply_to(1, &req);
    assert_eq!(reply.payload["result"]["profile"], json!({"ordinal": 1, "hasSiblings": false}));
    let req = h.request(1, kinds::PROFILE_RENAME, json!({"name": "  Home  "}));
    let reply = h.reply_to(1, &req);
    assert_eq!(reply.payload["result"], json!({"name": "Home"}));
    assert!(h.events.contains(&EngineEvent::ProfilesChanged));
    let req = h.request(1, kinds::ARCHIVE_SEARCH, json!({"query": ""}));
    let reply = h.reply_to(1, &req);
    assert_eq!(reply.payload["result"]["profile"]["name"], "Home");
    let req = h.request(1, kinds::PROFILE_RENAME, json!({"name": null}));
    assert_eq!(h.reply_to(1, &req).payload["result"], json!({"name": null}));
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
    // No settings came in: nothing to announce.
    assert!(!h.events.iter().any(|e| matches!(e, EngineEvent::SettingsChanged(_))));
}

#[test]
fn imported_settings_are_announced() {
    let mut h = Harness::new();
    h.connect(1, "s");
    let req = h.request(
        1,
        kinds::IMPORT_ARCHIVE,
        json!({"items": [], "rules": [], "settings": {"retentionDays": 30}}),
    );
    assert_eq!(h.reply_to(1, &req).payload["ok"], true);
    let settings = h
        .events
        .iter()
        .find_map(|e| match e {
            EngineEvent::SettingsChanged(s) => Some(s.clone()),
            _ => None,
        })
        .expect("announced");
    assert_eq!(settings["retention_days"], 30);
    assert!(settings.contains_key("theme"), "every setting, defaults filled in");
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

// --- Platform signals (P1.6) -----------------------------------------------------------

#[test]
fn os_idle_lock_and_display_sleep_drive_the_aging_clock() {
    let mut h = Harness::new();
    h.connect(1, "s");
    h.msg(1, kinds::SNAPSHOT, serde_json::to_value(snapshot(three_tabs())).unwrap());
    let age = |h: &Harness| h.item_for_url("https://old.example/page").active_age_s;
    // Active at the keyboard (no browser event needed): it ages.
    h.advance_os(61_000, &[1], |_| os(5, 5));
    let first = age(&h);
    assert!(first >= 30, "aged {first}");
    // Locked: nothing.
    h.advance_os(61_000, &[1], |_| decay_platform::PlatformSample {
        locked: Some(true),
        ..os(5, 5)
    });
    assert_eq!(age(&h), first);
    // Displays asleep: nothing.
    h.advance_os(61_000, &[1], |_| decay_platform::PlatformSample {
        display_asleep: Some(true),
        ..os(5, 5)
    });
    assert_eq!(age(&h), first);
    // Idle past the cutoff: nothing, unless audio plays (a video).
    h.advance_os(61_000, &[1], |_| os(900, 900));
    assert_eq!(age(&h), first);
    h.advance_os(61_000, &[1], |_| decay_platform::PlatformSample {
        audio_playing: Some(true),
        ..os(900, 900)
    });
    assert!(age(&h) > first);
}

#[test]
fn a_stale_os_sample_falls_back_to_browser_activity() {
    let mut h = Harness::new();
    h.connect(1, "s");
    h.msg(1, kinds::SNAPSHOT, serde_json::to_value(snapshot(three_tabs())).unwrap());
    h.os(os(5, 5)); // one sample, then silence
    h.advance(120_000, &[1]);
    // No browser activity either: idle, nothing aged.
    assert_eq!(h.item_for_url("https://old.example/page").active_age_s, 0);
}

#[test]
fn typing_or_an_app_switch_postpones_a_sweep() {
    let mut h = ready_to_sweep(Mode::Automatic);
    // Typing (last key 10 s ago): no sweep.
    h.advance_os(31_000, &[1], |_| os(10, 10));
    assert_eq!(h.take(1, kinds::PROBE).len(), 0);
    // An app switch just now: no sweep either.
    h.advance_os(65_000, &[1], |t| {
        // Switching apps every 20 s (under the 30-s quiet time).
        let pid = 10 + i32::from((t - T0) / 20_000 % 2 == 1);
        decay_platform::PlatformSample {
            frontmost: Some(decay_platform::FrontApp { pid, name: "x".into() }),
            ..os(70, 70)
        }
    });
    assert_eq!(h.take(1, kinds::PROBE).len(), 0);
    // Quiet: no typing for a minute, no switch for 30 s (then the next tick sweeps).
    h.advance_os(70_000, &[1], |_| decay_platform::PlatformSample {
        frontmost: Some(decay_platform::FrontApp { pid: 11, name: "x".into() }),
        ..os(120, 120)
    });
    assert_eq!(h.take(1, kinds::PROBE).len(), 1);
}

#[test]
fn no_sweep_while_locked_and_a_morning_cleanup_after_unlocking() {
    let mut h = ready_to_sweep(Mode::Automatic);
    let locked = || decay_platform::PlatformSample { locked: Some(true), ..os(4000, 4000) };
    h.advance_os(60_000, &[1], |_| locked());
    assert_eq!(h.take(1, kinds::PROBE).len(), 0);
    // Unlocked and typing straight away: the morning cleanup doesn't wait.
    h.advance_os(31_000, &[1], |_| os(1, 1));
    let probe = h.one(1, kinds::PROBE);
    h.ack_ok(1, &probe, json!({"results": [{"tabId": 1, "dirtyForm": false, "beforeUnload": false, "uploading": false}], "missing": []}));
    let close = h.one(1, kinds::TABS_CLOSE);
    let sweep = h.store().sweep(close.payload["sweepId"].as_str().unwrap()).unwrap().unwrap();
    assert_eq!(sweep.trigger, decay_core::SweepTrigger::Wake);
}

#[test]
fn a_sleep_gap_counts_as_a_wake() {
    let mut h = ready_to_sweep(Mode::Automatic);
    h.advance_os(5_000, &[1], |_| os(1, 1));
    h.now += 3 * 3_600_000; // asleep: no samples
    h.advance_os(31_000, &[1], |_| os(1, 1));
    assert_eq!(h.take(1, kinds::PROBE).len(), 1);
}

#[test]
fn a_tick_earns_nothing_for_time_locked_since_the_last_tick() {
    let mut h = Harness::new();
    h.connect(1, "s");
    h.msg(1, kinds::SNAPSHOT, serde_json::to_value(snapshot(three_tabs())).unwrap());
    let age = |h: &Harness| h.item_for_url("https://old.example/page").active_age_s;
    h.advance_os(61_000, &[1], |_| os(5, 5));
    let before = age(&h);
    // Locked for 28 of the next 30 s, unlocked (and active) only when the tick comes.
    let start = h.now;
    h.advance_os(30_000, &[1], move |t| {
        if t - start < 28_000 {
            decay_platform::PlatformSample { locked: Some(true), ..os(5, 5) }
        } else {
            os(1, 1)
        }
    });
    assert_eq!(age(&h), before);
    // The next tick fully active is credited again.
    h.advance_os(30_000, &[1], |_| os(1, 1));
    assert!(age(&h) > before);
}

#[test]
fn a_gap_in_the_samples_earns_no_credit() {
    let mut h = Harness::new();
    h.connect(1, "s");
    h.msg(1, kinds::SNAPSHOT, serde_json::to_value(snapshot(three_tabs())).unwrap());
    let age = |h: &Harness| h.item_for_url("https://old.example/page").active_age_s;
    h.advance_os(61_000, &[1], |_| os(5, 5));
    let before = age(&h);
    // A 20-s sleep (shorter than the aging clock's own gap limit), then active.
    h.now += 20_000;
    h.advance_os(10_000, &[1], |_| os(1, 1));
    // Whichever tick spans the gap is void; at most the ticks after it count.
    h.advance_os(1_000, &[1], |_| os(1, 1));
    assert!(age(&h) - before <= 1, "credited the gap: {}", age(&h) - before);
}

#[test]
fn os_audio_stops_counting_after_hours_without_input() {
    let mut h = Harness::new();
    h.connect(1, "s");
    h.msg(1, kinds::SNAPSHOT, serde_json::to_value(snapshot(three_tabs())).unwrap());
    let age = |h: &Harness| h.item_for_url("https://old.example/page").active_age_s;
    let playing =
        |idle| decay_platform::PlatformSample { audio_playing: Some(true), ..os(idle, idle) };
    h.advance_os(61_000, &[1], |_| playing(3_600));
    let watching = age(&h);
    assert!(watching >= 30, "an hour into a film still ages: {watching}");
    h.advance_os(61_000, &[1], |_| playing(3 * 3_600));
    assert_eq!(age(&h), watching, "a silent stream overnight doesn't");
}

#[test]
fn a_wall_clock_jump_is_not_a_wake() {
    let mut h = ready_to_sweep(Mode::Automatic);
    let base = h.now;
    // The continuous clock follows real time; the wall clock jumps 3 h ahead.
    let at = |offset: i64| {
        move |t: i64| decay_platform::PlatformSample {
            continuous_ms: Some(u64::try_from(t - base - offset).unwrap()),
            ..os(1, 1)
        }
    };
    h.advance_os(5_000, &[1], at(0));
    h.now += 3 * 3_600_000;
    // Still typing: no "morning cleanup", the sweep waits for a quiet moment.
    h.advance_os(31_000, &[1], at(3 * 3_600_000));
    assert_eq!(h.take(1, kinds::PROBE).len(), 0);
}

#[test]
fn a_sleep_on_the_continuous_clock_is_a_wake() {
    let mut h = ready_to_sweep(Mode::Automatic);
    let base = h.now;
    let sample = move |t: i64| decay_platform::PlatformSample {
        continuous_ms: Some(u64::try_from(t - base).unwrap()),
        ..os(1, 1)
    };
    h.advance_os(5_000, &[1], sample);
    h.now += 3 * 3_600_000;
    h.advance_os(31_000, &[1], sample);
    assert_eq!(h.take(1, kinds::PROBE).len(), 1);
}

#[test]
fn accessibility_changes_are_reported() {
    let mut h = Harness::new();
    h.os(os(1, 1));
    h.os(os(1, 1));
    h.os(decay_platform::PlatformSample { accessibility: Some(false), ..os(1, 1) });
    let changes: Vec<bool> = h
        .events
        .iter()
        .filter_map(|e| match e {
            EngineEvent::AccessibilityChanged { trusted } => Some(*trusted),
            _ => None,
        })
        .collect();
    assert_eq!(changes, vec![true, false]);
}

// --- App controls (P1.7) ----------------------------------------------------------------

#[test]
fn the_status_counts_stale_tabs_of_connected_browsers_and_only_changes_are_published() {
    let mut h = Harness::new();
    h.poll();
    let first = h.status();
    assert_eq!((first.mode, first.stale_count, first.live_tabs), (Mode::Shadow, 0, 0));
    assert_eq!(first.browsers, []);
    h.poll();
    assert_eq!(h.status_events(), 1, "unchanged status isn't sent again");
    h.connect(1, "s");
    h.msg(1, kinds::SNAPSHOT, serde_json::to_value(snapshot(three_tabs())).unwrap());
    h.engine.dev_add_age(13 * H, h.now);
    h.poll();
    let s = h.status();
    assert_eq!(s.live_tabs, 3);
    assert_eq!(s.browsers.len(), 1);
    assert_eq!((s.browsers[0].profile_id.as_str(), s.browsers[0].tabs), ("profile1", 3));
    // Only the old tab is stale: the active one is in use, the pinned one protected.
    assert_eq!(s.stale_count, 1);
    // A disconnected browser isn't counted.
    let out = h.engine.handle(Input::Disconnected(1), h.now);
    h.collect(out);
    h.poll();
    assert_eq!((h.status().live_tabs, h.status().stale_count), (0, 0));
}

#[test]
fn mode_and_hold_are_set_from_the_app_and_a_hold_runs_out() {
    let mut h = ready_to_sweep(Mode::Automatic);
    let out = h.engine.set_mode(Mode::Paused, h.now).unwrap();
    h.collect(out);
    assert_eq!(h.status().mode, Mode::Paused);
    assert_eq!(h.engine.sweep_now(h.now), Err(decay_engine::SweepRefused::Paused));
    // Resume goes back to the mode before the pause.
    let out = h.engine.resume(h.now).unwrap();
    h.collect(out);
    assert_eq!(h.status().mode, Mode::Automatic);
    let out = h.engine.set_mode(Mode::Shadow, h.now).unwrap();
    h.collect(out);
    for _ in 0..2 {
        let out = h.engine.set_mode(Mode::Paused, h.now).unwrap();
        h.collect(out); // pausing twice keeps the mode to go back to
    }
    let out = h.engine.resume(h.now).unwrap();
    h.collect(out);
    assert_eq!(h.status().mode, Mode::Shadow);
    let out = h.engine.set_mode(Mode::Automatic, h.now).unwrap();
    h.collect(out);
    let until = h.now + 3_600_000;
    let out = h.engine.hold(Some(until), h.now).unwrap();
    h.collect(out);
    assert_eq!(h.status().hold_until, Some(until));
    assert!(matches!(h.engine.sweep_now(h.now), Err(decay_engine::SweepRefused::OnHold(_))));
    // A hold in the past is no hold.
    let out = h.engine.hold(Some(h.now - 1), h.now).unwrap();
    h.collect(out);
    assert_eq!(h.status().hold_until, None);
    let out = h.engine.hold(Some(h.now + 60_000), h.now).unwrap();
    h.collect(out);
    // It runs out by itself, and the status says so.
    h.advance(61_000, &[1]);
    assert_eq!(h.status().hold_until, None);
}

#[test]
fn undo_last_sweep_from_the_app() {
    let (mut h, sweep_id) = swept();
    h.poll();
    assert_eq!(h.status().undoable_sweep.as_deref(), Some(sweep_id.as_str()));
    let out = h.engine.undo(None, h.now).unwrap().unwrap();
    h.collect(out);
    let restore = h.one(1, kinds::TABS_RESTORE);
    let items = restore.payload["items"].as_array().unwrap().clone();
    assert_eq!(items.len(), 2);
    let restored: Vec<Value> = items
        .iter()
        .enumerate()
        .map(|(n, i)| json!({"itemId": i["itemId"], "tabId": 30 + n}))
        .collect();
    h.ack_ok(1, &restore, json!({"restored": restored, "failed": []}));
    assert!(h.events.contains(&EngineEvent::SweepUndone {
        sweep_id: sweep_id.clone(),
        restored: 2,
        failed: 0
    }));
    assert_eq!(h.store().sweep(&sweep_id).unwrap().unwrap().status, SweepStatus::Undone);
    h.poll();
    assert_eq!(h.status().undoable_sweep, None);
    assert_eq!(h.engine.undo(None, h.now).unwrap(), Err(decay_engine::UndoRefused::NothingToUndo));
}

#[test]
fn undo_from_the_app_needs_the_browser_the_tabs_came_from() {
    let (mut h, sweep_id) = swept();
    let out = h.engine.handle(Input::Disconnected(1), h.now);
    h.collect(out);
    assert_eq!(
        h.engine.undo(Some(&sweep_id), h.now).unwrap(),
        Err(decay_engine::UndoRefused::NoBrowser)
    );
    assert_eq!(h.store().sweep(&sweep_id).unwrap().unwrap().status, SweepStatus::Done);
}

#[test]
fn undo_from_the_app_settles_a_sweep_already_restored_item_by_item() {
    let (mut h, sweep_id) = swept();
    // Every tab of the sweep restored from search, one by one.
    let ids: Vec<String> =
        h.store().archived_in_sweep(&sweep_id).unwrap().into_iter().map(|i| i.id).collect();
    let req = h.request(1, kinds::ARCHIVE_RESTORE, json!({"itemIds": ids}));
    let restore = h.one(1, kinds::TABS_RESTORE);
    let restored: Vec<Value> =
        ids.iter().enumerate().map(|(n, id)| json!({"itemId": id, "tabId": 40 + n})).collect();
    h.ack_ok(1, &restore, json!({"restored": restored, "failed": []}));
    h.reply_to(1, &req);
    // The tray's Undo still offers the sweep; undoing it settles it (and works offline).
    let out = h.engine.handle(Input::Disconnected(1), h.now);
    h.collect(out);
    h.poll();
    assert_eq!(h.status().undoable_sweep.as_deref(), Some(sweep_id.as_str()));
    let out = h.engine.undo(None, h.now).unwrap().unwrap();
    h.collect(out);
    assert_eq!(h.store().sweep(&sweep_id).unwrap().unwrap().status, SweepStatus::Undone);
    h.poll();
    assert_eq!(h.status().undoable_sweep, None);
}

// --- Panel actions (P1.9) ---------------------------------------------------------------

fn archived_ids(h: &Harness, sweep: &str) -> Vec<String> {
    h.store().archived_in_sweep(sweep).unwrap().into_iter().map(|i| i.id).collect()
}

#[test]
fn the_app_restores_items_where_they_were_and_hears_the_result() {
    let (mut h, sweep) = swept();
    let ids = archived_ids(&h, &sweep);
    let first = ids[0].clone();
    let out = h.engine.restore(&[first.clone(), "nope".into()], false, h.now).unwrap().unwrap();
    h.collect(out);
    let restore = h.one(1, kinds::TABS_RESTORE);
    assert_eq!(restore.payload.get("newWindow"), None);
    let items = restore.payload["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["windowKey"], "sess:1");
    assert_eq!(items[0]["active"], true);
    h.ack_ok(1, &restore, json!({"restored": [{"itemId": first, "tabId": 50}], "failed": []}));
    assert!(h.events.contains(&EngineEvent::Restored { restored: 1, failed: 0, skipped: 1 }));
    assert_eq!(h.store().item(&first).unwrap().unwrap().state, ItemState::Restored);
    // Already restored: nothing to do.
    assert_eq!(
        h.engine.restore(&[first], false, h.now).unwrap(),
        Err(decay_engine::RestoreRefused::NothingToRestore)
    );
}

#[test]
fn the_app_restores_into_a_new_window_or_a_whole_batch() {
    let (mut h, sweep) = swept();
    let ids = archived_ids(&h, &sweep);
    let out = h.engine.restore(&ids, true, h.now).unwrap().unwrap();
    h.collect(out);
    let restore = h.one(1, kinds::TABS_RESTORE);
    assert_eq!(restore.payload["newWindow"], true);
    let items = restore.payload["items"].as_array().unwrap().clone();
    assert!(items.iter().all(|i| i.get("windowKey").is_none()));
    h.ack_ok(1, &restore, json!({"restored": [], "failed": []}));

    // ⌘↵: the batch of one item = everything archived with it from its window.
    let out = h.engine.restore_batch(&ids[0], h.now).unwrap().unwrap();
    h.collect(out);
    let batch = h.one(1, kinds::TABS_RESTORE);
    assert_eq!(batch.payload["items"].as_array().unwrap().len(), ids.len());
}

/// P2.8: a refused restore names the profile the tabs came from, and the user can send
/// them to another connected profile instead (a new window there), only when asked.
#[test]
fn a_refused_restore_names_its_profile_and_can_go_elsewhere_on_request() {
    let (mut h, sweep) = swept();
    let ids = archived_ids(&h, &sweep);
    let out = h.engine.handle(Input::Disconnected(1), h.now);
    h.collect(out);
    let app = h.item_for_url("https://old.example/page").app_id;
    assert_eq!(
        h.engine.unreachable_home(&ids, h.now).unwrap(),
        Some((app.clone(), "profile1".to_owned()))
    );
    // Not connected: refused.
    assert_eq!(
        h.engine.restore_into(&ids, &app, "other-profile", h.now).unwrap(),
        Err(decay_engine::RestoreRefused::NoBrowser)
    );
    h.connect_as(2, "other-profile", "s2", "0.1.0");
    h.msg(2, kinds::SNAPSHOT, serde_json::to_value(snapshot(vec![])).unwrap());
    let out = h.engine.restore_into(&ids, &app, "other-profile", h.now).unwrap().unwrap();
    h.collect(out);
    let cmd = h.one(2, kinds::TABS_RESTORE);
    let items = cmd.payload["items"].as_array().unwrap();
    assert_eq!(items.len(), ids.len());
    assert!(items.iter().all(|i| i["windowKey"].is_null()), "a new window: {items:?}");
    assert_eq!(items.iter().filter(|i| i["active"] == true).count(), 1);
    // Once they're on their way, nothing is unreachable any more.
    assert_eq!(h.engine.unreachable_home(&[], h.now).unwrap(), None);
}

/// P2.8: a popup's restore puts each tab back in the profile it came from: its own
/// window for its own tabs, the other profile's window when that's open, and a refusal
/// naming the profile when it isn't. Nothing lands in the wrong profile.
#[test]
fn a_popup_restores_into_the_tabs_own_profile() {
    let (mut h, sweep) = swept();
    let ids = archived_ids(&h, &sweep);
    h.connect(2, "s2");
    h.msg(2, kinds::SNAPSHOT, serde_json::to_value(snapshot(vec![])).unwrap());
    // Profile 1 still open: from profile 2's popup they go back to profile 1.
    let req = h.request(2, kinds::ARCHIVE_RESTORE, json!({"itemIds": ids, "intoWindowId": 5}));
    assert_eq!(h.take(2, kinds::TABS_RESTORE).len(), 0);
    let cmd = h.one(1, kinds::TABS_RESTORE);
    assert!(
        cmd.payload["items"].as_array().unwrap().iter().all(|i| i["windowKey"] != "profile2|s2:5")
    );
    let _ = req;
    // Profile 1 closed: refused, and the message names it.
    let (mut h, sweep) = swept();
    let ids = archived_ids(&h, &sweep);
    let out = h.engine.handle(Input::Disconnected(1), h.now);
    h.collect(out);
    h.connect(2, "s2");
    h.msg(2, kinds::SNAPSHOT, serde_json::to_value(snapshot(vec![])).unwrap());
    let req = h.request(2, kinds::ARCHIVE_RESTORE, json!({"itemIds": ids, "intoWindowId": 5}));
    let reply = h.reply_to(2, &req);
    assert_eq!(reply.payload["ok"], false);
    let msg = reply.payload["error"]["message"].as_str().unwrap();
    assert!(msg.starts_with("Open Google Chrome · Profile 1 to restore this"), "{msg}");
    assert_eq!(h.take(2, kinds::TABS_RESTORE).len(), 0);
    let req = h.request(2, kinds::ARCHIVE_RESTORE_BATCH, json!({"itemId": ids[0]}));
    assert_eq!(h.reply_to(2, &req).payload["ok"], false);
    assert_eq!(h.take(2, kinds::TABS_RESTORE).len(), 0);
    // Its own popup puts them in its window.
    h.connect_as(3, "profile1", "s3", "0.1.0");
    h.msg(3, kinds::SNAPSHOT, serde_json::to_value(snapshot(vec![])).unwrap());
    h.request(3, kinds::ARCHIVE_RESTORE, json!({"itemIds": ids, "intoWindowId": 7}));
    let cmd = h.one(3, kinds::TABS_RESTORE);
    let items = cmd.payload["items"].as_array().unwrap();
    assert_eq!(items.len(), ids.len());
    assert!(
        items.iter().all(|i| i["windowKey"].as_str().is_some_and(|k| k.ends_with(":7"))),
        "{items:?}"
    );
}

/// P2.8: Settings' "Forget" drops a profile that isn't connected (never a connected one).
#[test]
fn forgetting_a_profile() {
    let mut h = Harness::new();
    h.connect(1, "s1");
    h.connect(2, "s2");
    let app = h.engine.profiles(h.now).unwrap()[0].app_id.clone();
    assert_eq!(
        h.engine.forget_profile(&app, "profile1").unwrap().unwrap_err(),
        decay_engine::ForgetRefused::Connected
    );
    let out = h.engine.handle(Input::Disconnected(1), h.now);
    h.collect(out);
    let out = h.engine.forget_profile(&app, "profile1").unwrap().unwrap();
    h.collect(out);
    assert!(h.events.contains(&EngineEvent::ProfilesChanged));
    let left: Vec<_> =
        h.engine.profiles(h.now).unwrap().into_iter().map(|p| p.profile_id).collect();
    assert_eq!(left, ["profile2"]);
    assert_eq!(
        h.engine.forget_profile(&app, "profile1").unwrap().unwrap_err(),
        decay_engine::ForgetRefused::UnknownProfile
    );
}

#[test]
fn restoring_needs_a_connected_browser() {
    let (mut h, sweep) = swept();
    let ids = archived_ids(&h, &sweep);
    let out = h.engine.handle(Input::Disconnected(1), h.now);
    h.collect(out);
    assert_eq!(
        h.engine.restore(&ids, false, h.now).unwrap(),
        Err(decay_engine::RestoreRefused::NoBrowser)
    );
    // Another profile being connected is no reason to open them there (other accounts).
    h.connect_as(2, "other-profile", "s2", "0.1.0");
    h.msg(
        2,
        kinds::SNAPSHOT,
        serde_json::to_value(snapshot(vec![tab(9, 7, "https://x.example/")])).unwrap(),
    );
    assert_eq!(
        h.engine.restore(&ids, false, h.now).unwrap(),
        Err(decay_engine::RestoreRefused::NoBrowser)
    );
    assert_eq!(h.take(2, kinds::TABS_RESTORE).len(), 0);
    // Undo doesn't send them there either: the undo is refused, the sweep stays.
    assert_eq!(
        h.engine.undo(Some(&sweep), h.now).unwrap(),
        Err(decay_engine::UndoRefused::NoBrowser)
    );
    assert_eq!(h.take(2, kinds::TABS_RESTORE).len(), 0);
    // The same profile back in a new browser session: restores go there.
    h.connect_as(3, "profile1", "s3", "0.1.0");
    h.msg(
        3,
        kinds::SNAPSHOT,
        serde_json::to_value(snapshot(vec![tab(9, 7, "https://y.example/")])).unwrap(),
    );
    let out = h.engine.restore(&ids[..1], false, h.now).unwrap().unwrap();
    h.collect(out);
    assert_eq!(h.take(3, kinds::TABS_RESTORE).len(), 1);
}

#[test]
fn the_first_tab_sent_gets_the_focus_even_if_an_earlier_one_is_skipped() {
    let (mut h, sweep) = swept();
    let ids = archived_ids(&h, &sweep);
    // The first one is already on its way back.
    let out = h.engine.restore(&ids[..1], false, h.now).unwrap().unwrap();
    h.collect(out);
    h.take(1, kinds::TABS_RESTORE);
    let out = h.engine.restore(&ids, false, h.now).unwrap().unwrap();
    h.collect(out);
    let restore = h.one(1, kinds::TABS_RESTORE);
    let items = restore.payload["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["active"], true);
}

#[test]
fn protect_site_adds_one_quick_action_rule() {
    let (mut h, sweep) = swept();
    let web = h
        .store()
        .archived_in_sweep(&sweep)
        .unwrap()
        .into_iter()
        .find(|i| i.domain.is_some())
        .expect("a web page");
    let rule = h.engine.protect_site(&web.id, h.now).unwrap().expect("a rule");
    assert_eq!(rule.pattern, web.domain.clone().unwrap());
    assert_eq!(rule.action, decay_core::RuleAction::Protect);
    assert_eq!(rule.source, decay_core::RuleSource::QuickAction);
    // Again: the same rule, not a duplicate.
    let again = h.engine.protect_site(&web.id, h.now).unwrap().unwrap();
    assert_eq!(again.id, rule.id);
    let same = h.store().rules().unwrap().into_iter().filter(|r| r.pattern == rule.pattern).count();
    assert_eq!(same, 1);
    assert_eq!(h.engine.protect_site("missing", h.now).unwrap(), None);
    // A rule the user turned off is turned back on by "Always keep".
    let mut off = rule.clone();
    off.enabled = false;
    h.store().update_rule(&off).unwrap();
    assert!(h.engine.protect_site(&web.id, h.now).unwrap().unwrap().enabled);
    let stored = h.store().rules().unwrap().into_iter().find(|r| r.id == rule.id).unwrap();
    assert!(stored.enabled);
    assert_eq!(h.store().archived_count().unwrap(), 2);
}

#[test]
fn completing_onboarding_sets_the_pace_and_starts_a_fresh_shadow_week() {
    let mut h = Harness::new();
    assert!(!h.engine.onboarded().unwrap());
    // Whatever happened before onboarding (an import, a mode set elsewhere, a hold) is reset.
    h.set_mode(Mode::Automatic);
    h.store().set_setting("hold_until", &Some(T0 + 3_600_000), h.now).unwrap();
    h.now += 60_000;
    let now = h.now;
    let out = h
        .engine
        .complete_onboarding(Preset::Aggressive, WindowManagement::Off, false, now)
        .unwrap();
    h.collect(out);
    let s = h.store();
    assert_eq!(s.mode().unwrap(), Mode::Shadow);
    assert_eq!(s.thresholds().unwrap(), Thresholds::AGGRESSIVE);
    assert_eq!(s.setting::<Preset>("preset").unwrap(), Some(Preset::Aggressive));
    assert_eq!(
        s.setting::<WindowManagement>("window_management").unwrap(),
        Some(WindowManagement::Off)
    );
    assert_eq!(s.setting::<i64>("shadow_started_at").unwrap(), Some(now));
    assert_eq!(s.hold_until().unwrap(), None);
    assert_eq!(s.setting::<bool>("launch_at_login").unwrap(), Some(false));
    assert!(h.engine.onboarded().unwrap());
    assert_eq!(h.engine.status(now).mode, Mode::Shadow);
}

#[test]
fn kept_live_lists_aged_tabs_a_protection_keeps() {
    let mut h = ready_to_sweep(Mode::Shadow);
    let kept: Vec<(String, ProtectedBy)> = h
        .engine
        .kept_live(h.now)
        .into_iter()
        .map(|(i, by)| (i.url.unwrap_or_default(), by))
        .collect();
    // The old page would be archived (not kept); the active and pinned tabs are kept.
    assert_eq!(kept.len(), 2, "{kept:?}");
    assert!(kept.contains(&("https://pinned.example/".into(), ProtectedBy::Pinned)));
    assert!(kept.iter().any(|(u, _)| u == "https://active.example/"));
    // A hold covers every tab: not a reason to list them.
    h.store().set_setting("hold_until", &(h.now + 3_600_000), h.now).unwrap();
    let out = h.engine.set_mode(Mode::Shadow, h.now).unwrap(); // reloads the policy
    h.collect(out);
    let held = h.engine.kept_live(h.now);
    assert!(held.iter().all(|(_, by)| *by != ProtectedBy::Hold), "{held:?}");
    // Young tabs aren't listed: nothing would have touched them anyway.
    let fresh = Harness::new();
    assert_eq!(fresh.engine.kept_live(fresh.now), Vec::new());
}

#[test]
fn rules_from_the_app_apply_at_once_and_excludes_purge() {
    use decay_core::{RuleAction, RuleScope};
    use decay_engine::{RuleInput, RuleRefused};
    let input = |scope: RuleScope, pattern: &str, action: RuleAction| RuleInput {
        scope,
        pattern: pattern.into(),
        action,
        threshold_hours: None,
        enabled: None,
    };
    let mut h = ready_to_sweep(Mode::Shadow);
    let old = h.item_for_url("https://old.example/page");

    // A rule test sees the open tab; nothing archived matches.
    let probe = input(RuleScope::Domain, "old.example", RuleAction::Protect);
    let test = h.engine.test_rule(&probe, h.now).unwrap();
    assert_eq!((test.matches_live, test.matches_archived), (1, 0));
    assert_eq!(test.sample[0].id, old.id);

    // Protect: saved, and the policy keeps the tab (it's listed as kept now).
    let (rule, purged, out) = h.engine.create_rule(&probe, h.now).unwrap();
    h.collect(out);
    assert_eq!((rule.pattern.as_str(), purged), ("old.example", 0));
    assert!(h.engine.kept_live(h.now).iter().any(|(i, _)| i.id == old.id));
    assert!(matches!(h.engine.create_rule(&probe, h.now), Err(RuleRefused::Duplicate)));
    assert!(matches!(
        h.engine.create_rule(&input(RuleScope::Domain, "com", RuleAction::Protect), h.now),
        Err(RuleRefused::Invalid(_))
    ));

    // Exclude: confirmed by a preview count, then everything stored for it goes, and the
    // extension is told at once.
    h.take(1, kinds::CONFIG);
    let exclude = input(RuleScope::Domain, "old.example", RuleAction::Exclude);
    assert_eq!(h.engine.preview_exclude(&exclude).unwrap(), 1);
    let (x, purged, out) = h.engine.create_rule(&exclude, h.now).unwrap();
    h.collect(out);
    assert_eq!(purged, 1);
    assert!(h.store().item(&old.id).unwrap().is_none());
    let config: Config = h.one(1, kinds::CONFIG).payload_as().unwrap();
    assert_eq!(config.excluded_domains, vec!["old.example".to_owned()]);
    // The open tab's record is gone too: the next change doesn't bring it back.
    h.advance(timing::TICK_MS, &[1]);
    assert!(
        h.store()
            .live_items()
            .unwrap()
            .iter()
            .all(|i| i.url.as_deref() != Some("https://old.example/page"))
    );

    // Turn the exclude off and delete the protect rule.
    let mut off = exclude.clone();
    off.enabled = Some(false);
    let (x2, _, out) = h.engine.update_rule(&x.id, &off, h.now).unwrap();
    h.collect(out);
    assert!(!x2.enabled);
    let out = h.engine.delete_rule(&rule.id, h.now).unwrap();
    h.collect(out);
    let ids: Vec<String> = h.engine.rules().unwrap().into_iter().map(|r| r.id).collect();
    assert!(ids.contains(&x.id) && !ids.contains(&rule.id));
    assert!(matches!(h.engine.delete_rule(&rule.id, h.now), Err(RuleRefused::NotFound)));
    assert!(matches!(h.engine.update_rule("nope", &off, h.now), Err(RuleRefused::NotFound)));
}

fn rule_input(
    scope: decay_core::RuleScope,
    pattern: &str,
    action: decay_core::RuleAction,
) -> decay_engine::RuleInput {
    decay_engine::RuleInput {
        scope,
        pattern: pattern.into(),
        action,
        threshold_hours: None,
        enabled: None,
    }
}

#[test]
fn an_exclude_added_mid_archive_deletes_the_item_once_the_close_settles() {
    use decay_core::{RuleAction, RuleScope};
    for closed in [true, false] {
        let mut h = ready_to_sweep(Mode::Automatic);
        h.poll();
        let probe = h.one(1, kinds::PROBE);
        h.ack_ok(1, &probe, json!({"results": [{"tabId": 1, "dirtyForm": false, "beforeUnload": false, "uploading": false}], "missing": []}));
        let close = h.one(1, kinds::TABS_CLOSE);
        let item = h.item_for_url("https://old.example/page");
        assert_eq!(item.state, ItemState::Archiving);

        let exclude = rule_input(RuleScope::Domain, "old.example", RuleAction::Exclude);
        // The confirm count includes the item being closed.
        assert_eq!(h.engine.preview_exclude(&exclude).unwrap(), 1);
        let (_, purged, out) = h.engine.create_rule(&exclude, h.now).unwrap();
        h.collect(out);
        assert_eq!(purged, 0, "the sweep owns it until the close settles");
        assert!(h.store().item(&item.id).unwrap().is_some());

        let result = if closed {
            json!({"closed": [1], "skipped": [], "unchecked": []})
        } else {
            json!({"closed": [], "skipped": [{"tabId": 1, "reason": "active"}], "unchecked": []})
        };
        h.ack_ok(1, &close, result);
        assert!(h.store().item(&item.id).unwrap().is_none(), "closed: {closed}");
        assert!(archived(h.store()).iter().all(|i| i.id != item.id));
    }
}

#[test]
fn turning_off_a_rule_works_even_when_it_fails_todays_checks() {
    use decay_core::{Rule, RuleAction, RuleScope, RuleSource};
    let mut h = Harness::new();
    // Saved by an older build: "com" is a public suffix, refused today.
    let old = Rule {
        id: "old-rule".into(),
        scope: RuleScope::Domain,
        pattern: "com".into(),
        action: RuleAction::Protect,
        threshold_hours: None,
        enabled: true,
        source: RuleSource::User,
        created_at: h.now,
        hit_count: 0,
    };
    h.store().insert_rule(&old).unwrap();
    let mut off = rule_input(RuleScope::Domain, "com", RuleAction::Protect);
    off.enabled = Some(false);
    let (r, _, _) = h.engine.update_rule("old-rule", &off, h.now).unwrap();
    assert!(!r.enabled);
    // Turning it back on (or changing it) is checked as usual.
    off.enabled = Some(true);
    assert!(matches!(
        h.engine.update_rule("old-rule", &off, h.now),
        Err(decay_engine::RuleRefused::Invalid(_))
    ));
}

#[test]
fn only_exclude_rules_preview_a_purge_and_only_domain_ones_reach_the_extension() {
    use decay_core::{RuleAction, RuleScope};
    let mut h = ready_to_sweep(Mode::Shadow);
    h.take(1, kinds::CONFIG);
    let protect = rule_input(RuleScope::Domain, "old.example", RuleAction::Protect);
    assert_eq!(h.engine.preview_exclude(&protect).unwrap(), 0);

    // A title exclude purges here, without telling the extension.
    let title = rule_input(RuleScope::TitleRegex, "Old", RuleAction::Exclude);
    let n = h.engine.preview_exclude(&title).unwrap();
    let (_, purged, out) = h.engine.create_rule(&title, h.now).unwrap();
    h.collect(out);
    assert_eq!(purged, n);
    assert_eq!(h.take(1, kinds::CONFIG).len(), 0);

    // Adding then deleting a domain exclude: the extension hears both.
    let domain = rule_input(RuleScope::Domain, "news.example", RuleAction::Exclude);
    let (d, _, out) = h.engine.create_rule(&domain, h.now).unwrap();
    h.collect(out);
    let config: Config = h.one(1, kinds::CONFIG).payload_as().unwrap();
    assert_eq!(config.excluded_domains, vec!["news.example".to_owned()]);
    let out = h.engine.delete_rule(&d.id, h.now).unwrap();
    h.collect(out);
    let config: Config = h.one(1, kinds::CONFIG).payload_as().unwrap();
    assert_eq!(config.excluded_domains, Vec::<String>::new());
}

#[test]
fn an_imported_archive_restores_into_a_connected_browser_of_its_app() {
    let mut h = Harness::new();
    h.connect(1, "s");
    h.msg(1, kinds::SNAPSHOT, serde_json::to_value(snapshot(three_tabs())).unwrap());
    // Bookmarks from a file, as the app imports them (no window or profile known).
    let html = r#"<DL><DT><A HREF="https://imported.example/page" ADD_DATE="1700000000">Imported</A></DL>"#;
    let app = h.engine.import_browser_app(h.now);
    assert_eq!(app, "com.google.Chrome");
    let (items, _) = decay_store::bookmarks_to_items(html, &app, h.now);
    let (result, out) = h
        .engine
        .import_from_app(
            json!({ "items": items }),
            h.now,
            decay_engine::ImportOptions { bookmarks: true, allow_purge: 0 },
        )
        .unwrap();
    h.collect(out);
    assert_eq!(result.report.items_imported, 1);
    let id = items[0]["id"].as_str().unwrap().to_owned();
    // Restoring it from the app finds the connected Chrome (it used to be refused).
    let out = h.engine.restore(&[id], false, h.now).unwrap().unwrap();
    h.collect(out);
    let restore = h.one(1, kinds::TABS_RESTORE);
    assert_eq!(restore.payload["items"][0]["url"], "https://imported.example/page");
}

#[test]
fn an_imported_item_without_a_profile_is_refused_when_two_profiles_could_take_it() {
    let mut h = Harness::new();
    h.connect(1, "s");
    h.connect(2, "s2"); // a second Chrome profile: another account
    for c in [1, 2] {
        h.msg(c, kinds::SNAPSHOT, serde_json::to_value(snapshot(three_tabs())).unwrap());
    }
    let html = r#"<DT><A HREF="https://imported.example/page">Imported</A>"#;
    let (items, _) = decay_store::bookmarks_to_items(html, "com.google.Chrome", h.now);
    let (_, out) = h
        .engine
        .import_from_app(
            json!({ "items": items }),
            h.now,
            decay_engine::ImportOptions { bookmarks: true, allow_purge: 0 },
        )
        .unwrap();
    h.collect(out);
    let id = items[0]["id"].as_str().unwrap().to_owned();
    let refused = h.engine.restore(std::slice::from_ref(&id), false, h.now).unwrap();
    assert!(matches!(refused, Err(decay_engine::RestoreRefused::NoBrowser)), "{refused:?}");
    assert!(h.take(1, kinds::TABS_RESTORE).is_empty() && h.take(2, kinds::TABS_RESTORE).is_empty());
    assert_eq!(h.store().item(&id).unwrap().unwrap().state, ItemState::Archived);
}

#[test]
fn a_file_import_keeps_never_recorded_sites_unrecorded_and_the_mode_as_is() {
    let mut h = Harness::new();
    h.connect(1, "s");
    h.msg(1, kinds::SNAPSHOT, serde_json::to_value(snapshot(three_tabs())).unwrap());
    // Already here: never record bank.example.
    let input = decay_engine::RuleInput {
        scope: decay_core::RuleScope::Domain,
        pattern: "bank.example".into(),
        action: decay_core::RuleAction::Exclude,
        threshold_hours: None,
        enabled: None,
    };
    let (_, _, out) = h.engine.create_rule(&input, h.now).unwrap();
    h.collect(out);
    // Stored before the import: a page the file's new exclude rule covers.
    let html = r#"<DT><A HREF="https://secret.example/old">Old</A>"#;
    let (old, _) = decay_store::bookmarks_to_items(html, "com.google.Chrome", h.now);
    let (_, out) = h
        .engine
        .import_from_app(
            json!({ "items": old }),
            h.now,
            decay_engine::ImportOptions { bookmarks: true, allow_purge: 0 },
        )
        .unwrap();
    h.collect(out);
    let recent = h.now - 1000;
    let item = |id: &str, url: &str| {
        json!({
            "id": id, "kind": "tab", "appId": "com.google.Chrome", "title": id, "url": url,
            "domain": "lies.example", "state": "archived", "firstSeenAt": 1, "lastUsedAt": 1,
            "stateChangedAt": recent, "archivedAt": recent
        })
    };
    let mut expired = item("expired", "https://ok.example/old");
    expired["archivedAt"] = json!(1);
    let file = json!({
        "format": "decay-archive", "version": 1, "exportedAt": 1,
        "items": [
            item("keep", "https://ok.example/a"),
            item("bank", "https://www.bank.example/statement"),
            item("secret", "https://secret.example/new"),
            expired,
        ],
        "rules": [{"id": "r-secret", "scope": "domain", "match": "secret.example",
                   "action": "exclude", "enabled": true, "source": "user", "createdAt": 1}],
        "settings": {"mode": "automatic", "retentionDays": 30}
    });
    // Its new rule would delete a stored item: refused until that's confirmed.
    assert_eq!(h.engine.import_purge_count(&file).unwrap(), 1);
    let backups = h.store().backups().unwrap().len();
    let refused =
        h.engine.import_from_app(file.clone(), h.now, decay_engine::ImportOptions::default());
    assert!(matches!(refused, Err(decay_engine::ImportRefused::NeedsConfirm(1))), "{refused:?}");
    assert_eq!(h.store().backups().unwrap().len(), backups, "nothing written");
    let confirmed = decay_engine::ImportOptions { bookmarks: false, allow_purge: 1 };
    let (result, out) = h.engine.import_from_app(file, h.now, confirmed).unwrap();
    h.collect(out);
    assert_eq!(
        (
            result.report.items_imported,
            result.items_excluded,
            result.report.items_purged,
            result.report.items_expired
        ),
        (1, 2, 1, 1)
    );
    let keep = h.store().item("keep").unwrap().unwrap();
    assert_eq!(keep.domain.as_deref(), Some("ok.example"), "the site comes from the address");
    assert_eq!(result.report.rules_imported, 1);
    assert!(h.store().item("keep").unwrap().is_some());
    assert!(
        h.store().item("bank").unwrap().is_none() && h.store().item("secret").unwrap().is_none()
    );
    let old_id = old[0]["id"].as_str().unwrap();
    assert!(h.store().item(old_id).unwrap().is_none(), "purged by the imported rule");
    assert_eq!(h.store().mode().unwrap(), Mode::Shadow, "a file never switches the mode");
    // The extension hears about the new exclusion.
    let config: Config = h.take(1, kinds::CONFIG).last().unwrap().payload_as().unwrap();
    assert!(config.excluded_domains.contains(&"secret.example".to_owned()));
    // Today's backup was written first.
    assert_ne!(h.store().backups().unwrap().len(), 0);
}

#[test]
fn a_file_that_isnt_an_archive_changes_nothing() {
    let mut h = Harness::new();
    let rules = h.store().rules().unwrap().len();
    for odd in [
        json!({"format": "decay-beta-metrics", "version": 1, "rules": [{"id": "x"}]}),
        json!({"format": "decay-archive", "version": 9, "items": []}),
    ] {
        assert!(
            h.engine.import_from_app(odd, h.now, decay_engine::ImportOptions::default()).is_err()
        );
    }
    assert_eq!(h.store().rules().unwrap().len(), rules);
    assert_eq!(h.store().backups().unwrap().len(), 0, "refused before the backup");
}

#[test]
fn imported_bookmarks_go_to_the_browser_last_used_when_none_is_connected() {
    let h = Harness::new();
    assert_eq!(h.engine.import_browser_app(h.now), "com.google.Chrome", "nothing known");
    let mut tab = decay_store::Item::new_live(
        "e1".into(),
        decay_core::ItemKind::Tab,
        "com.microsoft.edgemac",
        h.now,
    );
    tab.state = ItemState::Archived;
    tab.archived_at = Some(h.now);
    h.store().insert_item(&tab).unwrap();
    assert_eq!(h.engine.import_browser_app(h.now), "com.microsoft.edgemac");
}

#[test]
fn insights_count_what_happened_today() {
    let (mut h, _sweep) = swept();
    h.engine.set_time_zone(jiff::tz::TimeZone::UTC);
    let archived = archived(h.store());
    assert_eq!(archived.len(), 2);
    // The popup's search, then a restore picked from it 2.5 s after the first keystroke.
    h.msg(1, kinds::METRICS_SEARCHED, json!({}));
    let req = h.request(
        1,
        kinds::ARCHIVE_RESTORE,
        json!({"itemIds": [archived[0].id], "searchMs": 2500}),
    );
    let restore = h.one(1, kinds::TABS_RESTORE);
    h.ack_ok(
        1,
        &restore,
        json!({"restored": [{"itemId": archived[0].id, "tabId": 30}], "failed": []}),
    );
    let _ = h.reply_to(1, &req);
    // A tick with active use samples the open tabs for this hour (none without use).
    h.advance(decay_engine::timing::TICK_MS + 1, &[1]);
    let idle = h.engine.metrics(0, h.now).unwrap().days.pop().unwrap();
    h.msg(
        1,
        kinds::TAB_USED,
        json!({"tabId": 2, "windowId": 1, "at": h.now, "signal": "activated"}),
    );
    h.advance(decay_engine::timing::TICK_MS + 1, &[1]);
    let report = h.engine.metrics(0, h.now).unwrap();
    let today = report.days.last().expect("a day");
    assert_eq!(
        (today.archived_count, today.restored_24h_count, today.search_count),
        (2, 1, 1),
        "{today:?}"
    );
    assert_eq!(today.search_to_restore_samples, [2500]);
    assert!(idle.sampled_hours.is_empty(), "no sample without use: {idle:?}");
    assert_eq!(today.sampled_hours.len(), 1, "sampled hourly while in use");
    assert!(today.open_tabs_p50 > 0);
    // A restore not picked from a search adds no time; a failed one adds none either.
    let req = h.request(1, kinds::ARCHIVE_RESTORE, json!({"itemIds": [archived[1].id]}));
    let restore = h.one(1, kinds::TABS_RESTORE);
    h.ack_ok(1, &restore, json!({"restored": [], "failed": [archived[1].id]}));
    let _ = h.reply_to(1, &req);
    let report = h.engine.metrics(0, h.now).unwrap();
    assert_eq!(report.days.last().unwrap().search_to_restore_samples, [2500]);
}

#[test]
fn insights_keep_counts_on_the_day_they_happened_after_travelling() {
    let (mut h, _sweep) = swept();
    // 26 hours apart: always different calendar days.
    h.engine.set_time_zone(jiff::tz::TimeZone::get("Etc/GMT+12").unwrap());
    let before = h.engine.metrics(0, h.now).unwrap().days;
    let archived_on = |days: &[decay_store::MetricsDay]| {
        days.iter().filter(|d| d.archived_count > 0).map(|d| d.day.clone()).collect::<Vec<_>>()
    };
    assert_eq!(archived_on(&before).len(), 1, "{before:?}");
    h.engine.set_time_zone(jiff::tz::TimeZone::get("Pacific/Kiritimati").unwrap());
    h.advance(decay_engine::timing::TICK_MS + 1, &[1]);
    let after = h.engine.metrics(0, h.now).unwrap().days;
    assert_eq!(archived_on(&after), archived_on(&before));
}

#[test]
fn insights_count_an_undo_once_and_its_tabs_as_regret() {
    let (mut h, _sweep) = swept();
    h.engine.set_time_zone(jiff::tz::TimeZone::UTC);
    let req = h.request(1, kinds::ARCHIVE_UNDO, json!({}));
    let restore = h.one(1, kinds::TABS_RESTORE);
    let restored: Vec<Value> = restore.payload["items"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(i, it)| json!({"itemId": it["itemId"], "tabId": 40 + i}))
        .collect();
    h.ack_ok(1, &restore, json!({"restored": restored, "failed": []}));
    let _ = h.reply_to(1, &req);
    let today = h.engine.metrics(0, h.now).unwrap().days.pop().unwrap();
    assert_eq!((today.undo_count, today.restored_24h_count), (1, 2), "{today:?}");
}
