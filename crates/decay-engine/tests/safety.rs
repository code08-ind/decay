//! The "never lose work" safety suite (P1.14; PRD §3 principle 3, NFR Safety; Software
//! Design §9; product spec "Safety, privacy and edge cases"). Decay must fail by doing
//! nothing: each scenario below runs a real sweep through the engine with a fake
//! extension and checks that the protected tab is **never** asked to close, while an
//! ordinary old tab beside it is (so "nothing closed" can't pass by accident).
//!
//! | Scenario | Test |
//! | --- | --- |
//! | Audible tab | `audible_tabs_are_never_closed` |
//! | Becomes audible / active after going stale | `a_tab_that_turns_audible_or_active_after_going_stale_is_kept` |
//! | Typed form input / "leave site?" / upload | `pages_with_unsaved_input_are_never_closed` |
//! | Page state unknown (2× threshold, with its control) | `pages_that_cant_be_checked_wait_twice_as_long` |
//! | Probe fails or goes unanswered | `a_probe_that_fails_or_goes_unanswered_means_no_close` |
//! | A page that checked clean gets a last check right before closing | `requireChecked`, checked on every close ([`sweep`]) |
//! | Pinned / active tab | `pinned_and_active_tabs_are_never_closed` |
//! | Incognito | `private_windows_are_never_recorded_or_closed` |
//! | Can't be reopened exactly (file:, data:, blob:, view-source:) | `pages_that_cant_be_reopened_are_never_closed` |
//! | "Always keep" rule | `protect_rules_win` |
//! | Restored by Undo (48 h grace, with its control) | `undone_tabs_are_left_alone_for_48_hours` |
//! | Undo for the last 20 sweeps | `the_last_20_sweeps_can_be_undone` |
//! | Shadow mode / paused / on hold | `shadow_pause_and_hold_never_close` |
//! | Screen locked / display asleep (with its control) | `nothing_is_closed_while_the_screen_is_locked_or_asleep` |
//! | Browser goes quiet / a stale answer / disconnect after the close | `a_browser_that_stops_answering_is_not_swept`, `a_probe_answer_from_a_dropped_link_closes_nothing`, `a_disconnect_after_the_close_keeps_the_record_until_the_browser_says` |
//! | Extension refuses or can't check at close time | `a_close_the_extension_refuses_is_not_archived` |
//! | Write before close | checked on every close, in every test ([`closes`]) |
//! | Unsaved / unknown document (app windows) | `unsaved_or_unknown_documents_are_never_closed` (policy; windows arrive in Phase 2) |
//! | Any mix of the above (random sessions, incl. timeouts, reconnects with new tab ids, late closes) | `random_sessions_never_lose_work`, with `a_session_does_archive_what_it_may` showing it closes things |
//! | Machine sleeps or goes idle: the clock pauses | `engine.rs`: `os_idle_lock_and_display_sleep_drive_the_aging_clock`, `a_sleep_gap_counts_as_a_wake`, `a_tick_earns_nothing_for_time_locked_since_the_last_tick` |
//! | Decay crashes between saving the record and the close | `engine.rs`: `after_a_crash_archiving_items_settle_when_the_browser_reconnects` |
//! | The database is damaged, missing or empty | `decay-store` `backup.rs`: `a_damaged_database_is_replaced_by_the_newest_good_backup`, `a_missing_or_empty_database_is_restored_not_recreated`, `missing_database_and_unreadable_backups_is_an_error_not_a_fresh_start` |
//! | Lifecycle never archives what may not close | `decay-core` `lifecycle.rs` property tests |
//!
//! Mutation-tested (P1.14 QA): every protection above fails at least one test when it's
//! removed, except two guards that no real event sequence reaches: the sweep's own
//! audible / active re-check (a tab that turns audible or active leaves "stale" at once,
//! and the extension refuses to close it too) and the engine's hold check (the policy
//! holds as well).
//!
//! | In a call or meeting (mic or camera in use), and 2 min after | `nothing_is_swept_during_a_call_or_meeting` |
//!
//! Not covered yet:
//! - Screen sharing without a call: macOS has no public signal for another app capturing
//!   the screen (sharing nearly always comes with a call, which is covered).
//! - The *order* of writing and closing inside one engine step (the record is written in
//!   a durable transaction before the close is queued; a store-failure hook would test
//!   that a failed write sends no close).
//! - Downloads: Chrome keeps a download going after its tab closes, so they're not a tab
//!   protection.

#![allow(clippy::needless_pass_by_value)]

mod support;

use std::collections::{BTreeMap, BTreeSet};

use decay_bridge::protocol::{Snapshot, TabInfo, WindowInfo, WindowState};
use decay_bridge::{Envelope, kinds};
use decay_core::{
    Decision, ItemKind, ItemState, Mode, PolicyContext, PolicyTarget, ProtectedBy, Rule,
    RuleAction, RuleScope, RuleSource, SaveState, Signals, Thresholds, evaluate,
};
use decay_engine::{ConnId, Input, timing};
use proptest::prelude::*;
use serde_json::{Value, json};
#[allow(clippy::wildcard_imports)]
use support::*;

const OLD: &str = "https://old.example/page";

/// How the fake extension answers a probe for one tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Clean,
    Dirty,
    BeforeUnload,
    Uploading,
    Unknown,
    /// Not in the answer at all (the probe lost it).
    Missing,
}

fn answer(id: i64, page: Page) -> Value {
    json!({
        "tabId": id,
        "dirtyForm": page == Page::Dirty,
        "beforeUnload": page == Page::BeforeUnload,
        "uploading": page == Page::Uploading,
        "unknown": page == Page::Unknown,
    })
}

/// Tabs in window 1 (focused) and, for incognito tabs, a private window 2.
fn snapshot_with(tabs: Vec<TabInfo>, private: Vec<TabInfo>) -> Snapshot {
    let mut windows = vec![WindowInfo {
        id: 1,
        focused: true,
        state: WindowState::Normal,
        incognito: false,
        tabs,
    }];
    if !private.is_empty() {
        windows.push(WindowInfo {
            id: 2,
            focused: false,
            state: WindowState::Normal,
            incognito: true,
            tabs: private,
        });
    }
    Snapshot { windows, groups: Vec::new() }
}

/// The old control tab (id 1) plus `others`, all `hours` old, in `mode`.
fn scene(mode: Mode, others: Vec<TabInfo>, hours: u64) -> Harness {
    scene_with(mode, others, Vec::new(), hours)
}

fn scene_with(mode: Mode, others: Vec<TabInfo>, private: Vec<TabInfo>, hours: u64) -> Harness {
    let mut h = Harness::new();
    h.set_mode(mode);
    h.connect(1, "s");
    let mut tabs = vec![tab(1, 1, OLD)];
    tabs.extend(others);
    h.msg(1, kinds::SNAPSHOT, serde_json::to_value(snapshot_with(tabs, private)).unwrap());
    h.engine.dev_add_age(hours * H, h.now);
    h
}

/// Every close command sent so far (taken), after checking write-before-close: each tab
/// in it has its record saved as archiving, in a sweep that exists, before the close.
fn closes(h: &mut Harness, conn: ConnId) -> Vec<Envelope> {
    let all = h.take(conn, kinds::TABS_CLOSE);
    for close in &all {
        let sweep = close.payload["sweepId"].as_str().expect("a sweep id");
        assert!(h.store().sweep(sweep).unwrap().is_some(), "the sweep is written first");
        for id in close.payload["tabIds"].as_array().unwrap() {
            let url = close.payload["urls"][id.to_string()]
                .as_str()
                .unwrap_or_else(|| panic!("tab {id} is closed without its URL"));
            let saved = h.store().live_items().unwrap().into_iter().any(|i| {
                i.url.as_deref() == Some(url)
                    && i.state == ItemState::Archiving
                    && i.last_sweep_id.as_deref() == Some(sweep)
                    && i.archive_reason.is_some()
            });
            assert!(saved, "{url}: record saved (archiving, in {sweep}) before the close");
        }
    }
    all
}

fn require_checked(close: &Envelope) -> BTreeSet<i64> {
    close.payload["requireChecked"]
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_i64).collect())
        .unwrap_or_default()
}

fn is_web(close: &Envelope, id: i64) -> bool {
    close.payload["urls"][id.to_string()]
        .as_str()
        .is_some_and(|u| u.starts_with("http:") || u.starts_with("https:"))
}

fn tab_ids(closes: &[Envelope]) -> BTreeSet<i64> {
    closes
        .iter()
        .flat_map(|c| c.payload["tabIds"].as_array().unwrap().iter().filter_map(Value::as_i64))
        .collect()
}

/// Runs the sweep for 5 minutes in 5-s steps, answering every probe at once (a probe
/// times out after ~8 s) with `page` and acking every close as closed. Checks on every
/// close that a page answered clean is listed for a last check (`requireChecked`).
/// Returns the tab ids Decay asked to close.
fn sweep(h: &mut Harness, page: impl Fn(i64) -> Page) -> BTreeSet<i64> {
    let mut closed = BTreeSet::new();
    // Tabs whose probe was answered (before it could time out).
    let mut answered = BTreeSet::new();
    for step in 0..60 {
        if step > 0 {
            h.now += 5_000;
            h.msg(1, kinds::HEARTBEAT, json!({"at": h.now}));
        }
        h.poll();
        for probe in h.take(1, kinds::PROBE) {
            let ids: Vec<i64> = probe.payload["tabIds"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(Value::as_i64)
                .collect();
            let results: Vec<Value> = ids
                .iter()
                .filter(|id| page(**id) != Page::Missing)
                .map(|id| answer(*id, page(*id)))
                .collect();
            let missing: Vec<i64> =
                ids.iter().copied().filter(|id| page(*id) == Page::Missing).collect();
            h.ack_ok(1, &probe, json!({"results": results, "missing": missing}));
            answered.extend(ids);
        }
        for close in closes(h, 1) {
            let checked = require_checked(&close);
            for id in tab_ids(std::slice::from_ref(&close)) {
                // Planned on a clean check: the extension must check again right before
                // closing (typing may have started since).
                if answered.contains(&id) && page(id) == Page::Clean && is_web(&close, id) {
                    assert!(checked.contains(&id), "tab {id} closed without a last check");
                }
            }
            let ids = close.payload["tabIds"].clone();
            closed.extend(tab_ids(std::slice::from_ref(&close)));
            h.ack_ok(1, &close, json!({"closed": ids, "skipped": [], "unchecked": []}));
        }
    }
    closed
}

fn clean(_: i64) -> Page {
    Page::Clean
}

fn ids(list: &[i64]) -> BTreeSet<i64> {
    list.iter().copied().collect()
}

#[test]
fn an_ordinary_old_tab_is_archived() {
    // The control: without it, every "never closed" below could pass on a dead engine.
    let mut h = scene(Mode::Automatic, vec![], 61);
    assert_eq!(sweep(&mut h, clean), ids(&[1]));
    assert_eq!(h.item_for_url(OLD).state, ItemState::Archived);
}

#[test]
fn audible_tabs_are_never_closed() {
    let mut music = tab(2, 1, "https://music.example/");
    music.audible = true;
    let mut h = scene(Mode::Automatic, vec![music], 500);
    assert_eq!(sweep(&mut h, clean), ids(&[1]));
}

#[test]
fn pages_with_unsaved_input_are_never_closed() {
    for page in [Page::Dirty, Page::BeforeUnload, Page::Uploading] {
        let form = tab(2, 1, "https://form.example/");
        let mut h = scene(Mode::Automatic, vec![form], 500);
        let closed = sweep(&mut h, |id| if id == 2 { page } else { Page::Clean });
        assert_eq!(closed, ids(&[1]), "{page:?}");
        assert_ne!(h.item_for_url("https://form.example/").state, ItemState::Archived);
    }
}

#[test]
fn pages_that_cant_be_checked_wait_twice_as_long() {
    for page in [Page::Unknown, Page::Missing] {
        let pick = move |id| if id == 2 { page } else { Page::Clean };
        let mut h = scene(Mode::Automatic, vec![tab(2, 1, "https://opaque.example/")], 100);
        assert_eq!(sweep(&mut h, pick), ids(&[1]), "{page:?} at 100 h (< 2 × 60 h)");
        // The control: past 2× it does go (so it was waiting, not stuck).
        let mut h = scene(Mode::Automatic, vec![tab(2, 1, "https://opaque.example/")], 125);
        assert_eq!(sweep(&mut h, pick), ids(&[1, 2]), "{page:?} at 125 h");
    }
}

#[test]
fn pinned_and_active_tabs_are_never_closed() {
    let mut pinned = tab(2, 1, "https://pinned.example/");
    pinned.pinned = true;
    let mut active = tab(3, 1, "https://active.example/");
    active.active = true;
    let mut h = scene(Mode::Automatic, vec![pinned, active], 500);
    assert_eq!(sweep(&mut h, clean), ids(&[1]));
}

#[test]
fn private_windows_are_never_recorded_or_closed() {
    let secret = tab(7, 2, "https://private.example/");
    let mut h = scene_with(Mode::Automatic, vec![], vec![secret], 500);
    assert_eq!(sweep(&mut h, clean), ids(&[1]));
    let stored = h.store().live_items().unwrap().into_iter().chain(archived(h.store()));
    assert!(stored.into_iter().all(|i| i.url.as_deref() != Some("https://private.example/")));
}

#[test]
fn pages_that_cant_be_reopened_are_never_closed() {
    let odd = vec![
        tab(2, 1, "file:///Users/me/notes.html"),
        tab(3, 1, "data:text/html,<p>draft</p>"),
        tab(4, 1, "blob:https://app.example/1b2c"),
        tab(5, 1, "view-source:https://example.com/"),
    ];
    let mut h = scene(Mode::Automatic, odd, 500);
    assert_eq!(sweep(&mut h, clean), ids(&[1]));
}

#[test]
fn protect_rules_win() {
    let mut h = scene(Mode::Automatic, vec![tab(2, 1, "https://mail.example/inbox")], 500);
    let keep = decay_engine::RuleInput {
        scope: RuleScope::Domain,
        pattern: "mail.example".into(),
        action: RuleAction::Protect,
        threshold_hours: None,
        enabled: None,
    };
    let (_, _, out) = h.engine.create_rule(&keep, h.now).unwrap();
    h.collect(out);
    assert_eq!(sweep(&mut h, clean), ids(&[1]));
}

#[test]
fn undone_tabs_are_left_alone_for_48_hours() {
    let mut h = scene(Mode::Automatic, vec![], 61);
    assert_eq!(sweep(&mut h, clean), ids(&[1]));
    // Undo: the tab comes back (a new tab id), and is as old as ever on the clock.
    h.request(1, kinds::ARCHIVE_UNDO, json!({}));
    let restore = h.one(1, kinds::TABS_RESTORE);
    let item = restore.payload["items"][0]["itemId"].as_str().unwrap().to_owned();
    h.ack_ok(1, &restore, json!({"restored": [{"itemId": item, "tabId": 10}], "failed": []}));
    let back = tab(10, 1, OLD);
    h.msg(1, kinds::SNAPSHOT, serde_json::to_value(snapshot_with(vec![back], vec![])).unwrap());
    h.engine.dev_add_age(500 * H, h.now);
    h.advance(20 * 60_000, &[1]);
    assert_eq!(sweep(&mut h, clean), BTreeSet::new(), "within 48 h of the Undo");
    // The control: once the 48 h are over, the same tab goes. (A jump, not `advance`:
    // probes sent meanwhile would go unanswered and count as unknown.)
    h.now += 49 * 3_600_000;
    h.msg(1, kinds::HEARTBEAT, json!({"at": h.now}));
    h.msg(
        1,
        kinds::SNAPSHOT,
        serde_json::to_value(snapshot_with(vec![tab(10, 1, OLD)], vec![])).unwrap(),
    );
    assert_eq!(sweep(&mut h, clean), ids(&[10]), "after 48 h");
}

#[test]
fn shadow_pause_and_hold_never_close() {
    let mut shadow = scene(Mode::Shadow, vec![], 500);
    assert_eq!(sweep(&mut shadow, clean), BTreeSet::new());
    assert_ne!(shadow.item_for_url(OLD).state, ItemState::Archived);

    let mut paused = scene(Mode::Automatic, vec![], 500);
    let out = paused.engine.set_mode(Mode::Paused, paused.now).unwrap();
    paused.collect(out);
    assert_eq!(sweep(&mut paused, clean), BTreeSet::new());
    assert_eq!(paused.take(1, kinds::PROBE).len(), 0);

    let mut held = scene(Mode::Automatic, vec![], 500);
    let out = held.engine.hold(Some(held.now + 3_600_000), held.now).unwrap();
    held.collect(out);
    assert_eq!(sweep(&mut held, clean), BTreeSet::new());
}

#[test]
fn nothing_is_closed_while_the_screen_is_locked_or_asleep() {
    // Idle for 10 minutes (no typing): only the lock / sleep differs from the control.
    let sample = |locked: bool, asleep: bool| {
        let mut s = os(600, 600);
        s.locked = Some(locked);
        s.display_asleep = Some(asleep);
        s
    };
    for (locked, asleep, sweeps) in
        [(false, false, true), (true, false, false), (false, true, false)]
    {
        let mut h = scene(Mode::Automatic, vec![], 500);
        h.os(sample(locked, asleep));
        h.poll();
        let probed = h.take(1, kinds::PROBE).len() == 1;
        assert_eq!(probed, sweeps, "locked {locked}, display asleep {asleep}");
    }
}

#[test]
fn a_browser_that_stops_answering_is_not_swept() {
    // No `Disconnected`: the extension just goes quiet (hung, or the machine is busy).
    let mut h = Harness::new();
    h.set_mode(Mode::Automatic);
    h.connect(1, "s");
    let snap = snapshot_with(vec![tab(1, 1, OLD)], vec![]);
    h.msg(1, kinds::SNAPSHOT, serde_json::to_value(snap).unwrap());
    h.advance(timing::STALE_LINK_MS + 10_000, &[]); // no heartbeats
    h.engine.dev_add_age(500 * H, h.now);
    h.poll();
    assert_eq!(h.take(1, kinds::PROBE).len() + h.take(1, kinds::TABS_CLOSE).len(), 0);
    // The control: the same browser speaking again is swept.
    h.msg(1, kinds::HEARTBEAT, json!({"at": h.now}));
    assert_eq!(sweep(&mut h, clean), ids(&[1]));
}

#[test]
fn a_probe_answer_from_a_dropped_link_closes_nothing() {
    let mut h = scene(Mode::Automatic, vec![], 61);
    h.poll();
    let probe = h.one(1, kinds::PROBE);
    h.profile = Some("profile1".into()); // the same browser comes back
    let out = h.engine.handle(Input::Disconnected(1), h.now);
    h.collect(out);
    reconnect_with(&mut h, 2, vec![tab(1, 1, OLD)]);
    // The old probe's answer arrives late, on the new link: it doesn't count.
    h.ack_ok(2, &probe, json!({"results": [answer(1, Page::Clean)], "missing": []}));
    assert_eq!(closes(&mut h, 2).len(), 0, "no close on a stale answer");
    assert_ne!(h.item_for_url(OLD).state, ItemState::Archiving);
}

fn reconnect_with(h: &mut Harness, conn: ConnId, tabs: Vec<TabInfo>) {
    h.connect(conn, "s");
    let snap = serde_json::to_value(snapshot_with(tabs, vec![])).unwrap();
    h.msg(conn, kinds::SNAPSHOT, snap);
}

#[test]
fn a_disconnect_after_the_close_keeps_the_record_until_the_browser_says() {
    for still_open in [true, false] {
        let mut h = scene(Mode::Automatic, vec![], 61);
        h.poll();
        let probe = h.one(1, kinds::PROBE);
        h.ack_ok(1, &probe, json!({"results": [answer(1, Page::Clean)], "missing": []}));
        assert_eq!(closes(&mut h, 1).len(), 1);
        h.profile = Some("profile1".into()); // the same browser comes back
        let out = h.engine.handle(Input::Disconnected(1), h.now);
        h.collect(out);
        // Unknown outcome: the record stays (never deleted, never closed again).
        h.advance(10 * 60_000, &[]);
        assert_eq!(h.item_for_url(OLD).state, ItemState::Archiving);
        // The browser comes back and shows where the tab is.
        h.connect(2, "s");
        let tabs =
            if still_open { vec![tab(1, 1, OLD)] } else { vec![tab(5, 1, "https://x.example/")] };
        h.msg(2, kinds::SNAPSHOT, serde_json::to_value(snapshot_with(tabs, vec![])).unwrap());
        let want = if still_open { ItemState::Stale } else { ItemState::Archived };
        assert_eq!(h.item_for_url(OLD).state, want, "still open: {still_open}");
        assert_eq!(closes(&mut h, 2).len(), 0, "settled, not closed again");
    }
}

#[test]
fn a_close_the_extension_refuses_is_not_archived() {
    for reason in ["formInput", "unchecked", "changed", "inUse", "error"] {
        let mut h = scene(Mode::Automatic, vec![], 61);
        h.poll();
        let probe = h.one(1, kinds::PROBE);
        h.ack_ok(1, &probe, json!({"results": [answer(1, Page::Clean)], "missing": []}));
        let close = closes(&mut h, 1).pop().unwrap();
        h.ack_ok(
            1,
            &close,
            json!({"closed": [], "skipped": [{"tabId": 1, "reason": reason}], "unchecked": []}),
        );
        let item = h.item_for_url(OLD);
        assert_ne!(item.state, ItemState::Archived, "{reason}");
        assert_ne!(item.state, ItemState::Archiving, "{reason}: settled");
    }
}

#[test]
fn a_tab_that_turns_audible_or_active_after_going_stale_is_kept() {
    // What the extension sends (background.ts): audio starting, and the tab activated.
    let audible = (kinds::TAB_UPDATED, json!({"tabId": 2, "changes": {"audible": true}}));
    let active =
        (kinds::TAB_USED, json!({"tabId": 2, "windowId": 1, "at": T0, "signal": "activated"}));
    for (kind, msg) in [audible, active] {
        // Stale first (13 h: past "stale", far from "archive")…
        let mut h = scene(Mode::Automatic, vec![tab(2, 1, "https://late.example/")], 13);
        h.poll();
        h.advance(16 * 60_000, &[1]);
        // …then it starts playing / becomes the active tab, and both get very old (past
        // 2×, so even an unchecked page would qualify): only the live signal keeps it.
        h.msg(1, kind, msg.clone());
        h.engine.dev_add_age(130 * H, h.now);
        assert_eq!(sweep(&mut h, clean), ids(&[1]), "{kind} {msg}");
    }
}

#[test]
fn a_probe_that_fails_or_goes_unanswered_means_no_close() {
    for failed in [true, false] {
        let mut h = scene(Mode::Automatic, vec![tab(2, 1, "https://quiet.example/")], 100);
        h.poll();
        let probe = h.one(1, kinds::PROBE);
        if failed {
            let err = json!({"ok": false, "error": {"code": "INTERNAL", "message": "boom"}});
            h.ack(1, &probe, err, "profile1");
        } else {
            h.advance(30_000, &[1]); // never answered: it times out
        }
        // Both pages are unknown now: at 100 h (< 2 × 60 h) neither may go.
        h.advance(60_000, &[1]);
        assert_eq!(closes(&mut h, 1).len(), 0, "failed: {failed}");
        // The control: a later sweep that hears back closes the clean page only.
        h.advance(16 * 60_000, &[1]);
        h.take(1, kinds::PROBE);
        let pick = |id| if id == 2 { Page::Unknown } else { Page::Clean };
        assert_eq!(sweep(&mut h, pick), ids(&[1]), "failed: {failed}");
    }
}

#[test]
fn the_last_20_sweeps_can_be_undone() {
    let mut h = Harness::new();
    h.set_mode(Mode::Automatic);
    h.connect(1, "s");
    for k in 1..=21 {
        let url = format!("https://page{k}.example/");
        let snap = snapshot_with(vec![tab(k, 1, &url)], vec![]);
        h.msg(1, kinds::SNAPSHOT, serde_json::to_value(snap).unwrap());
        h.engine.dev_add_age(61 * H, h.now);
        assert_eq!(sweep(&mut h, clean), ids(&[k]), "sweep {k}");
        h.advance(16 * 60_000, &[1]);
    }
    let all: Vec<String> = h
        .store()
        .recent_sweeps(100)
        .unwrap()
        .into_iter()
        .filter(|s| s.item_count > 0)
        .map(|s| s.id)
        .collect();
    assert_eq!(all.len(), 21);
    let undoable = h.engine.undoable_sweep_ids().unwrap();
    assert_eq!(undoable.len(), 20);
    assert!(all[..20].iter().all(|id| undoable.contains(id)), "the newest 20");
    assert!(!undoable.contains(&all[20]), "not the 21st back");
    // The 20th back still really reopens its tab.
    h.request(1, kinds::ARCHIVE_UNDO, json!({"sweepId": all[19]}));
    assert_eq!(h.one(1, kinds::TABS_RESTORE).payload["items"].as_array().unwrap().len(), 1);
}

// --- App windows (Phase 2): the policy never closes an unsaved or unknown document -----

#[test]
fn unsaved_or_unknown_documents_are_never_closed() {
    let allow = Rule {
        id: "r".into(),
        scope: RuleScope::App,
        pattern: "com.apple.Preview".into(),
        action: RuleAction::CloseAllowed,
        threshold_hours: None,
        enabled: true,
        source: RuleSource::User,
        created_at: 0,
        hit_count: 0,
    };
    let rules = [allow];
    let ctx = PolicyContext {
        now: 1,
        mode: Mode::Automatic,
        hold_until: None,
        thresholds: Thresholds::BALANCED,
        rules: &rules,
        pro: true,
    };
    let window = PolicyTarget {
        kind: ItemKind::Window,
        app_id: "com.apple.Preview",
        url: None,
        title: "report.pdf",
        pinned: false,
        active_age_s: 1000 * H,
        grace_until: None,
    };
    let at = |save_state| evaluate(&window, &Signals { save_state, ..Signals::default() }, &ctx);
    assert!(matches!(at(SaveState::Saved), Decision::Archive { .. }), "the control");
    for (state, by) in [
        (SaveState::Unsaved, ProtectedBy::Unsaved),
        (SaveState::Unknown, ProtectedBy::UnknownSaveState),
        (SaveState::NotApplicable, ProtectedBy::UnknownSaveState),
    ] {
        assert!(
            matches!(at(state), Decision::Hide { kept_by: Some(b), .. } if b == by),
            "{state:?}: hidden at most, never closed"
        );
    }
}

// --- Property test: any session ------------------------------------------------------
//
// The fake browser behaves like the real extension: it answers probes (or lets them time
// out), and before closing it checks each page again and skips one with unsaved input
// (P0.7's pre-close check). The engine may *ask* to close a page it couldn't check once
// it's past 2× the threshold; the checks below are about what it may ask and what may
// be lost.

/// One tab of a random session.
#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools)]
struct Spec {
    url: usize,
    pinned: bool,
    audible: bool,
    active: bool,
    private: bool,
    page: Page,
}

const URLS: [&str; 8] = [
    "https://a.example/1",
    "https://b.example/2",
    "https://c.example/3",
    "http://d.example/4",
    "file:///tmp/e.html",
    "data:text/plain,f",
    "chrome://settings/",
    "https://g.example/7",
];

fn page_strategy() -> impl Strategy<Value = Page> {
    prop_oneof![
        4 => Just(Page::Clean),
        1 => Just(Page::Dirty),
        1 => Just(Page::BeforeUnload),
        1 => Just(Page::Uploading),
        1 => Just(Page::Unknown),
        1 => Just(Page::Missing),
    ]
}

fn spec_strategy() -> impl Strategy<Value = Spec> {
    (
        0..URLS.len(),
        prop::bool::weighted(0.15),
        prop::bool::weighted(0.15),
        prop::bool::weighted(0.15),
        prop::bool::weighted(0.15),
        page_strategy(),
    )
        .prop_map(|(url, pinned, audible, active, private, page)| Spec {
            url,
            pinned,
            audible,
            active,
            private,
            page,
        })
}

#[derive(Debug, Clone)]
enum Step {
    /// Hours of active use pass.
    Age(u64),
    /// Minutes pass; probes are answered within `answer_after` seconds (> ~8 s: too late).
    Wait { minutes: u8, answer_after: u8 },
    /// The extension answers the oldest pending close: per tab, closed / refused / no
    /// answer at all.
    AnswerClose(Vec<u8>),
    /// The browser drops the link and comes back, maybe with new tab ids.
    Reconnect { renumber: bool },
}

fn step_strategy() -> impl Strategy<Value = Step> {
    prop_oneof![
        2 => (1u64..200).prop_map(Step::Age),
        3 => (1u8..30, prop_oneof![4 => Just(0u8), 1 => Just(20u8)])
            .prop_map(|(minutes, answer_after)| Step::Wait { minutes, answer_after }),
        3 => prop::collection::vec(0u8..3, 8).prop_map(Step::AnswerClose),
        1 => any::<bool>().prop_map(|renumber| Step::Reconnect { renumber }),
    ]
}

/// Each tab's own URL (distinct, and kept across renumbering).
fn url_of(s: &Spec, spec_index: usize) -> String {
    format!("{}#{spec_index}", URLS[s.url])
}

fn unsaved(page: Page) -> bool {
    matches!(page, Page::Dirty | Page::BeforeUnload | Page::Uploading)
}

struct Session {
    h: Harness,
    conn: ConnId,
    specs: Vec<Spec>,
    /// Tabs open in the fake browser (tab id → spec index).
    open: BTreeMap<i64, usize>,
    /// Closes sent and not answered yet.
    pending: Vec<Envelope>,
    /// URLs the browser closed for Decay: each must stay in the archive.
    closed_urls: BTreeSet<String>,
    /// What the engine heard for each tab on this link, and when.
    heard: BTreeMap<i64, (Page, i64)>,
    /// An upper bound on any tab's active age, in hours.
    max_age_h: u64,
    /// How many closes were asked for (to show the checks ran).
    asked: usize,
}

impl Session {
    fn tab_info(&self, id: i64) -> TabInfo {
        let i = self.open[&id];
        let s = &self.specs[i];
        let mut t = tab(id, if s.private { 2 } else { 1 }, &url_of(s, i));
        t.pinned = s.pinned;
        t.audible = s.audible;
        t.active = s.active;
        t
    }

    fn snapshot(&self) -> Value {
        let (mut tabs, mut private) = (Vec::new(), Vec::new());
        for id in self.open.keys() {
            let t = self.tab_info(*id);
            if t.window_id == 2 {
                private.push(t);
            } else {
                tabs.push(t);
            }
        }
        serde_json::to_value(snapshot_with(tabs, private)).unwrap()
    }

    fn send_snapshot(&mut self) {
        let snap = self.snapshot();
        self.h.msg(self.conn, kinds::SNAPSHOT, snap);
    }

    /// Answers probes (if `answer_probes`) and resyncs, then checks every new close.
    fn pump(&mut self, mode: Mode, answer_probes: bool) {
        for resync in self.h.take(self.conn, kinds::RESYNC) {
            self.h.ack_ok(self.conn, &resync, json!(null));
            self.send_snapshot();
        }
        if !answer_probes {
            // Left unanswered: they time out, and the engine counts those pages as unknown.
            for probe in self.h.take(self.conn, kinds::PROBE) {
                for id in
                    probe.payload["tabIds"].as_array().unwrap().iter().filter_map(Value::as_i64)
                {
                    self.heard.insert(id, (Page::Unknown, self.h.now));
                }
            }
        }
        if answer_probes {
            for probe in self.h.take(self.conn, kinds::PROBE) {
                let mut results = Vec::new();
                let mut missing = Vec::new();
                for id in
                    probe.payload["tabIds"].as_array().unwrap().iter().filter_map(Value::as_i64)
                {
                    let page = self.open.get(&id).map_or(Page::Missing, |i| self.specs[*i].page);
                    self.heard.insert(id, (page, self.h.now));
                    if page == Page::Missing {
                        missing.push(id);
                    } else {
                        results.push(answer(id, page));
                    }
                }
                self.h.ack_ok(self.conn, &probe, json!({"results": results, "missing": missing}));
            }
        }
        for close in closes(&mut self.h, self.conn) {
            assert_ne!(mode, Mode::Shadow, "shadow mode never closes");
            let checked = require_checked(&close);
            for id in tab_ids(std::slice::from_ref(&close)) {
                let Some(&i) = self.open.get(&id) else { continue };
                self.asked += 1;
                let s = &self.specs[i];
                let url = url_of(s, i);
                assert!(!s.pinned && !s.audible && !s.active && !s.private, "protected: {s:?}");
                assert!(!url.starts_with("file:") && !url.starts_with("data:"), "{url}");
                let now = self.h.now;
                let heard = self.heard.get(&id).copied();
                if let Some((page, at)) = heard {
                    // A fresh "unsaved" verdict blocks the close outright.
                    let fresh_other = now - at < timing::PROBE_OTHER_TTL_MS;
                    assert!(!(unsaved(page) && fresh_other), "{url}: closed despite {page:?}");
                    // A fresh clean verdict: closed only with a last check.
                    let fresh_clean = now - at < timing::PROBE_CLEAN_TTL_MS;
                    if page == Page::Clean && fresh_clean && url.starts_with("http") {
                        assert!(checked.contains(&id), "{url}: clean but no last check");
                    }
                }
                let checked_clean = heard.is_some_and(|(p, at)| {
                    p == Page::Clean && now - at < timing::PROBE_CLEAN_TTL_MS
                });
                if url.starts_with("http") && !checked_clean {
                    // Couldn't check it: only past 2× the 60-h threshold.
                    assert!(self.max_age_h >= 120, "{url}: unchecked at ≤ {} h", self.max_age_h);
                }
            }
            self.pending.push(close);
        }
    }

    /// The extension's answer to the oldest pending close. Like the real one, it checks
    /// each page right before closing and skips one with unsaved input.
    fn answer_close(&mut self, how: &[u8]) {
        if self.pending.is_empty() || how.first() == Some(&2) {
            return; // nothing pending, or the answer never comes
        }
        let close = self.pending.remove(0);
        let (mut closed, mut skipped) = (Vec::new(), Vec::new());
        for (n, id) in tab_ids(std::slice::from_ref(&close)).into_iter().enumerate() {
            // A tab that's already gone is reported closed (as the extension does).
            let page = self.open.get(&id).map(|i| self.specs[*i].page);
            if page.is_some() && (page.is_some_and(unsaved) || how.get(n) == Some(&1)) {
                skipped.push(json!({"tabId": id, "reason": "formInput"}));
                continue;
            }
            if let Some(i) = self.open.remove(&id) {
                self.closed_urls.insert(url_of(&self.specs[i], i));
            }
            closed.push(id);
        }
        // Like Chrome, the extension reports each tab it closed (not by the user).
        for id in &closed {
            self.h.msg(self.conn, kinds::TAB_REMOVED, json!({"tabId": id, "byUser": false}));
        }
        self.h.ack_ok(
            self.conn,
            &close,
            json!({"closed": closed, "skipped": skipped, "unchecked": []}),
        );
    }

    /// Nothing the browser closed for Decay was lost: it's archived (or still settling).
    fn check_nothing_lost(&self) {
        let items: Vec<_> = self
            .h
            .store()
            .live_items()
            .unwrap()
            .into_iter()
            .chain(archived(self.h.store()))
            .collect();
        for url in &self.closed_urls {
            let kept = items.iter().any(|i| {
                i.url.as_deref() == Some(url.as_str())
                    && matches!(
                        i.state,
                        ItemState::Archived | ItemState::Archiving | ItemState::Restored
                    )
                    && i.archive_reason.is_some()
                    && i.last_sweep_id.is_some()
            });
            assert!(kept, "{url} was closed by Decay and isn't in the archive");
        }
    }
}

/// Runs a session; returns how many closes Decay asked for (all checked on the way).
fn run_session(mode: Mode, specs: Vec<Spec>, steps: Vec<Step>) -> usize {
    let mut h = Harness::new();
    h.set_mode(mode);
    h.profile = Some("profile1".into()); // one browser, whatever the connection
    h.connect(1, "s");
    let open = (0..specs.len()).map(|i| (i64::try_from(i).unwrap() + 1, i)).collect();
    let mut s = Session {
        h,
        conn: 1,
        specs,
        open,
        pending: Vec::new(),
        closed_urls: BTreeSet::new(),
        heard: BTreeMap::new(),
        max_age_h: 0,
        asked: 0,
    };
    s.send_snapshot();
    for step in steps {
        match step {
            Step::Age(hours) => {
                s.max_age_h += hours;
                s.h.engine.dev_add_age(hours * H, s.h.now);
                s.h.poll();
                s.pump(mode, true);
            }
            Step::Wait { minutes, answer_after } => {
                // Wall time can add a little active age too (bounded generously).
                s.max_age_h += u64::from(minutes) / 60 + 1;
                let end = s.h.now + i64::from(minutes) * 60_000;
                let conn = s.conn;
                while s.h.now < end {
                    s.h.advance(5_000, &[conn]);
                    s.pump(mode, answer_after == 0);
                }
            }
            Step::AnswerClose(how) => s.answer_close(&how),
            Step::Reconnect { renumber } => {
                let out = s.h.engine.handle(Input::Disconnected(s.conn), s.h.now);
                s.h.collect(out);
                s.pending.clear();
                s.heard.clear(); // verdicts belong to the old link
                if renumber {
                    s.open = s.open.iter().map(|(id, i)| (id + 100, *i)).collect();
                }
                s.conn += 1;
                let conn = s.conn;
                s.h.connect(conn, "s");
                s.send_snapshot();
            }
        }
        s.pump(mode, true);
        s.check_nothing_lost();
    }
    s.asked
}

#[test]
fn a_session_does_archive_what_it_may() {
    // The property test's checks only bite when sessions close things: these must.
    let spec = |url, page| Spec {
        url,
        pinned: false,
        audible: false,
        active: false,
        private: false,
        page,
    };
    let specs = vec![spec(0, Page::Clean), spec(1, Page::Dirty), spec(4, Page::Clean)];
    let steps = vec![
        Step::Age(100),
        Step::Wait { minutes: 2, answer_after: 0 },
        Step::AnswerClose(vec![0, 0, 0]),
    ];
    assert_eq!(run_session(Mode::Automatic, specs, steps), 1, "only the clean web page");
    // Past 2× with the probes unanswered: the engine may ask; the extension's own check
    // still keeps the page with unsaved input.
    let specs = vec![spec(0, Page::Clean), spec(1, Page::Dirty)];
    let steps = vec![
        Step::Age(200),
        Step::Wait { minutes: 3, answer_after: 20 },
        Step::AnswerClose(vec![0, 0]),
        Step::Reconnect { renumber: true },
        Step::Wait { minutes: 20, answer_after: 0 },
    ];
    assert!(run_session(Mode::Automatic, specs, steps) >= 1);
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 48, ..ProptestConfig::default() })]

    #[test]
    fn random_sessions_never_lose_work(
        automatic in prop::bool::weighted(0.8),
        specs in prop::collection::vec(spec_strategy(), 1..7),
        steps in prop::collection::vec(step_strategy(), 1..14),
    ) {
        let mode = if automatic { Mode::Automatic } else { Mode::Shadow };
        run_session(mode, specs, steps);
    }
}

#[test]
fn nothing_is_swept_during_a_call_or_meeting() {
    for (mic, camera) in [(true, false), (false, true)] {
        let mut h = scene(Mode::Automatic, vec![], 61);
        let call = |h: &mut Harness, on: bool| {
            let mut sample = os(600, 600);
            sample.mic_in_use = Some(on && mic);
            sample.camera_in_use = Some(on && camera);
            h.os(sample);
        };
        // Twenty minutes in a call: quiet otherwise, an old tab, automatic mode.
        for _ in 0..(20 * 60) {
            h.now += 1_000;
            call(&mut h, true);
            h.msg(1, kinds::HEARTBEAT, json!({"at": h.now}));
            h.poll();
        }
        assert_eq!(h.take(1, kinds::PROBE).len() + h.take(1, kinds::TABS_CLOSE).len(), 0);
        assert!(h.engine.status(h.now).in_meeting);
        // The call ends: still nothing for two minutes…
        for _ in 0..100 {
            h.now += 1_000;
            call(&mut h, false);
            h.msg(1, kinds::HEARTBEAT, json!({"at": h.now}));
            h.poll();
        }
        assert_eq!(h.take(1, kinds::PROBE).len(), 0, "mic {mic} camera {camera}: cool-down");
        // …then the sweep goes ahead.
        for _ in 0..60 {
            h.now += 1_000;
            call(&mut h, false);
            h.msg(1, kinds::HEARTBEAT, json!({"at": h.now}));
            h.poll();
        }
        assert!(!h.engine.status(h.now).in_meeting);
        assert_eq!(h.take(1, kinds::PROBE).len(), 1, "after the meeting");
    }
}

#[test]
fn sweep_now_is_still_the_users_call_during_a_meeting() {
    let mut h = scene(Mode::Automatic, vec![], 61);
    let mut sample = os(600, 600);
    sample.mic_in_use = Some(true);
    h.os(sample);
    h.poll(); // the tick: the tab goes stale, the scheduled sweep waits for the meeting
    assert_eq!(h.take(1, kinds::PROBE).len(), 0);
    let out = h.engine.sweep_now(h.now).unwrap();
    h.collect(out);
    assert_eq!(h.take(1, kinds::PROBE).len(), 1);
}
