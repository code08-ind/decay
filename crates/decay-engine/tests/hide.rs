//! Hiding stale app windows (P2.2): which windows a hide sweep takes, the record before
//! the action, the actor's answers, the user bringing a window back, Undo, and every case
//! where nothing may be hidden.

#![allow(clippy::needless_pass_by_value)]

mod support;

use decay_core::{EventType, ItemKind, ItemState, Mode, SweepStatus};
use decay_engine::{EngineEvent, Input, WindowCommand};
use decay_platform::{AppWindow, AppWindows, FrontApp, WindowList, WindowOp};
use decay_store::Item;
use serde_json::json;
#[allow(clippy::wildcard_imports)]
use support::*;

const APP: &str = "com.apple.TextEdit";
const PID: i32 = 50;

fn window(id: u32, title: &str) -> AppWindow {
    AppWindow {
        window_id: Some(id),
        title: title.into(),
        document: Some(format!("file:///{title}")),
        edited: Some(false),
        can_minimize: true,
        ..AppWindow::default()
    }
}

fn list(windows: Vec<AppWindow>) -> Vec<AppWindows> {
    let all_window_ids = windows.iter().filter_map(|w| w.window_id).collect();
    vec![AppWindows {
        pid: PID,
        bundle_id: APP.into(),
        name: "TextEdit".into(),
        windows,
        all_window_ids,
    }]
}

impl Harness {
    fn windows(&mut self, apps: Vec<AppWindows>) {
        let out = self.engine.handle(
            Input::Windows(Box::new(WindowList { apps, started_at: Some(self.now) })),
            self.now,
        );
        self.collect(out);
    }

    /// One second: a quiet OS sample (idle 10 min, another app in front), a window list
    /// every 3 s, the timers.
    fn second(&mut self, apps: &dyn Fn() -> Vec<AppWindows>) {
        self.now += 1_000;
        let mut s = os(600, 600);
        s.frontmost = Some(FrontApp { pid: 99, name: "Other".into() });
        self.os(s);
        if self.now % 3_000 < 1_000 {
            self.windows(apps());
        }
        self.poll();
    }

    /// Runs for `ms`, stopping early when the engine asks for a window command (so the
    /// test answers it within its 10 s, as the actor would).
    fn run_for(&mut self, ms: i64, apps: &dyn Fn() -> Vec<AppWindows>) {
        let end = self.now + ms;
        while self.now < end {
            self.second(apps);
            if !self.window_cmds.is_empty() {
                break;
            }
        }
    }

    fn acted(&mut self, cmd: &WindowCommand, result: Result<(), String>) {
        let input = Input::WindowActed { item_id: cmd.item_id.clone(), result };
        let out = self.engine.handle(input, self.now);
        self.collect(out);
    }

    fn take_cmds(&mut self) -> Vec<WindowCommand> {
        std::mem::take(&mut self.window_cmds)
    }

    fn windows_items(&self) -> Vec<Item> {
        let mut v: Vec<Item> = self
            .store()
            .live_items()
            .unwrap()
            .into_iter()
            .filter(|i| i.kind == ItemKind::Window)
            .collect();
        v.sort_by(|a, b| a.title.cmp(&b.title));
        v
    }

    fn item(&self, title: &str) -> Item {
        self.windows_items().into_iter().find(|i| i.title == title).unwrap()
    }
}

/// Window handling on, `mode`, the windows of `apps` listed and `hours` old.
fn scene(mode: Mode, apps: &dyn Fn() -> Vec<AppWindows>, hours: u64) -> Harness {
    let mut h = Harness::new();
    h.store().set_setting("window_management", &"hide", h.now).unwrap();
    h.set_mode(mode);
    let out = h.engine.settings_changed(h.now).unwrap();
    h.collect(out);
    h.windows(apps());
    h.engine.dev_add_age(hours * H, h.now);
    h
}

fn one() -> Vec<AppWindows> {
    list(vec![window(1, "old.txt")])
}

#[test]
fn a_stale_window_past_the_hidden_threshold_is_minimised_and_recorded() {
    let mut h = scene(Mode::Automatic, &one, 40); // Balanced: hidden at 36 h
    h.run_for(40_000, &one);
    let cmds = h.take_cmds();
    assert_eq!(cmds.len(), 1);
    assert_eq!((cmds[0].op, cmds[0].pid, cmds[0].window_id), (WindowOp::Hide, PID, 1));
    // Written before the action: a running sweep with the item pending.
    let sweep = h.store().recent_sweeps(5).unwrap().remove(0);
    assert_eq!(sweep.status, SweepStatus::Running);
    h.acted(&cmds[0], Ok(()));
    let item = h.item("old.txt");
    assert_eq!(item.state, ItemState::Hidden);
    assert_eq!(item.last_sweep_id.as_deref(), Some(sweep.id.as_str()));
    assert!(item.archive_reason.is_some());
    let done = h.store().sweep(&sweep.id).unwrap().unwrap();
    assert_eq!((done.status, done.item_count, done.failed_count), (SweepStatus::Done, 1, 0));
    assert!(h.events.iter().any(|e| matches!(e, EngineEvent::WindowsHidden { hidden: 1, .. })));
    let events: Vec<EventType> =
        h.store().events_for_item(&item.id, 20).unwrap().into_iter().map(|e| e.kind).collect();
    assert!(events.contains(&EventType::Hidden));
    // A hidden window doesn't age and isn't hidden again.
    h.run_for(20 * 60_000, &|| {
        let mut w = window(1, "old.txt");
        w.minimized = true;
        list(vec![w])
    });
    assert_eq!(h.take_cmds(), []);
    assert_eq!(h.item("old.txt").state, ItemState::Hidden);
}

#[test]
fn windows_that_may_not_be_hidden_never_are() {
    type Tweak = fn(&mut AppWindow);
    let cases: [(&str, Tweak); 5] = [
        ("minimised already", |w| w.minimized = true),
        ("full screen", |w| w.full_screen = true),
        ("no minimise button", |w| w.can_minimize = false),
        ("no window number", |w| w.window_id = None),
        ("unsaved", |w| w.edited = Some(true)),
    ];
    for (what, tweak) in cases {
        let apps = move || {
            let mut w = window(1, "x.txt");
            tweak(&mut w);
            list(vec![w])
        };
        let mut h = scene(Mode::Automatic, &apps, 100);
        h.run_for(40_000, &apps);
        let hides = h.take_cmds().into_iter().filter(|c| c.op == WindowOp::Hide).count();
        // Unsaved windows may still be *hidden* (minimising loses nothing); only closing
        // needs a saved document.
        let want = usize::from(what == "unsaved");
        assert_eq!(hides, want, "{what}");
    }
}

#[test]
fn the_focused_window_of_the_app_in_front_is_never_hidden() {
    let apps = || {
        let mut w = window(1, "busy.txt");
        w.focused = true;
        list(vec![w])
    };
    let mut h = scene(Mode::Automatic, &apps, 100);
    // TextEdit in front.
    let end = h.now + 40_000;
    while h.now < end {
        h.now += 1_000;
        let mut s = os(600, 600);
        s.frontmost = Some(FrontApp { pid: PID, name: "TextEdit".into() });
        h.os(s);
        if h.now % 3_000 < 1_000 {
            h.windows(apps());
        }
        h.poll();
    }
    assert_eq!(h.take_cmds(), []);
}

#[test]
fn a_window_on_another_space_is_not_hidden() {
    let apps = || {
        let mut a = list(vec![window(1, "here.txt")]);
        a[0].all_window_ids.push(2); // there.txt: only the window server sees it now
        a
    };
    let both = || list(vec![window(1, "here.txt"), window(2, "there.txt")]);
    let mut h = scene(Mode::Automatic, &both, 0);
    h.run_for(4_000, &apps); // switched Space
    h.engine.dev_add_age(100 * H, h.now);
    h.run_for(40_000, &apps);
    let cmds = h.take_cmds();
    assert!(cmds.iter().all(|c| c.window_id != 2), "{cmds:?}");
}

#[test]
fn shadow_mode_pause_hold_rules_young_windows_and_window_handling_off_hide_nothing() {
    let quiet = |mut h: Harness| {
        h.run_for(40_000, &one);
        h.take_cmds().len()
    };
    assert_eq!(quiet(scene(Mode::Shadow, &one, 100)), 0, "shadow");
    assert_eq!(quiet(scene(Mode::Automatic, &one, 20)), 0, "stale but under 36 h");
    let mut paused = scene(Mode::Automatic, &one, 100);
    let out = paused.engine.set_mode(Mode::Paused, paused.now).unwrap();
    paused.collect(out);
    assert_eq!(quiet(paused), 0, "paused");
    let mut held = scene(Mode::Automatic, &one, 100);
    let out = held.engine.hold(Some(held.now + 3_600_000), held.now).unwrap();
    held.collect(out);
    assert_eq!(quiet(held), 0, "on hold");
    let mut ruled = scene(Mode::Automatic, &one, 100);
    let keep = decay_engine::RuleInput {
        scope: decay_core::RuleScope::App,
        pattern: APP.into(),
        action: decay_core::RuleAction::Protect,
        threshold_hours: None,
        enabled: None,
    };
    let (_, _, out) = ruled.engine.create_rule(&keep, ruled.now).unwrap();
    ruled.collect(out);
    assert_eq!(quiet(ruled), 0, "protect rule");
    let mut off = scene(Mode::Automatic, &one, 100);
    off.store().set_setting("window_management", &"off", off.now).unwrap();
    let out = off.engine.settings_changed(off.now).unwrap();
    off.collect(out);
    assert_eq!(quiet(off), 0, "window handling off");
}

#[test]
fn nothing_is_hidden_while_the_user_types() {
    let mut h = scene(Mode::Automatic, &one, 100);
    let end = h.now + 40_000;
    while h.now < end {
        h.now += 1_000;
        let mut s = os(1, 1); // typing right now
        s.frontmost = Some(FrontApp { pid: 99, name: "Other".into() });
        h.os(s);
        if h.now % 3_000 < 1_000 {
            h.windows(one());
        }
        h.poll();
    }
    assert_eq!(h.take_cmds(), []);
}

#[test]
fn a_failed_or_unanswered_hide_leaves_the_window_stale() {
    for answer in [Some(Err("the window isn't reachable".to_owned())), None] {
        let mut h = scene(Mode::Automatic, &one, 100);
        h.run_for(40_000, &one);
        let cmd = h.take_cmds().remove(0);
        match &answer {
            Some(result) => h.acted(&cmd, result.clone()),
            None => h.run_for(15_000, &one), // no answer within 10 s
        }
        assert_eq!(h.item("old.txt").state, ItemState::Stale, "{answer:?}");
        let sweep = h.store().recent_sweeps(5).unwrap().remove(0);
        assert_eq!((sweep.status, sweep.item_count, sweep.failed_count), (SweepStatus::Done, 0, 1));
        h.acted(&cmd, Ok(()));
        if answer.is_some() {
            // It said it failed: a stray success afterwards changes nothing.
            assert_eq!(h.item("old.txt").state, ItemState::Stale);
        } else {
            // It was only late: the window was minimised after all, so it's recorded
            // hidden (and can be undone), and the sweep's counts are corrected.
            assert_eq!(h.item("old.txt").state, ItemState::Hidden);
            let sweep = h.store().sweep(&sweep.id).unwrap().unwrap();
            assert_eq!((sweep.item_count, sweep.failed_count), (1, 0));
        }
    }
}

#[test]
fn at_most_five_windows_per_sweep_then_the_next_sweep_a_quarter_hour_later() {
    let many = || list((1..=8).map(|n| window(n, &format!("w{n}.txt"))).collect());
    let mut h = scene(Mode::Automatic, &many, 100);
    h.run_for(40_000, &many);
    let first = h.take_cmds();
    assert_eq!(first.len(), 5);
    for c in &first {
        h.acted(c, Ok(()));
    }
    let hidden: Vec<u32> = first.iter().map(|c| c.window_id).collect();
    let after = move || {
        list(
            (1..=8)
                .map(|n| {
                    let mut w = window(n, &format!("w{n}.txt"));
                    w.minimized = hidden.contains(&n);
                    w
                })
                .collect(),
        )
    };
    h.run_for(10 * 60_000, &after);
    assert_eq!(h.take_cmds(), [], "not again within 15 minutes");
    h.run_for(6 * 60_000, &after);
    assert_eq!(h.take_cmds().len(), 3);
}

#[test]
fn a_window_the_user_brings_back_is_used() {
    let mut h = scene(Mode::Automatic, &one, 100);
    h.run_for(40_000, &one);
    let cmd = h.take_cmds().remove(0);
    h.acted(&cmd, Ok(()));
    assert_eq!(h.item("old.txt").state, ItemState::Hidden);
    // The user clicks it in the Dock: no longer minimised.
    h.run_for(4_000, &one);
    let item = h.item("old.txt");
    assert_eq!((item.state, item.active_age_s), (ItemState::Active, 0));
    let events: Vec<EventType> =
        h.store().events_for_item(&item.id, 20).unwrap().into_iter().map(|e| e.kind).collect();
    assert!(events.contains(&EventType::Used));
}

#[test]
fn undo_brings_the_windows_back_and_keeps_them_for_48_hours() {
    let mut h = scene(Mode::Automatic, &one, 100);
    h.connect(1, "s"); // Undo is asked from the extension or the app; any path works
    h.run_for(40_000, &one);
    let cmd = h.take_cmds().remove(0);
    h.acted(&cmd, Ok(()));
    let minimised = || {
        let mut w = window(1, "old.txt");
        w.minimized = true;
        list(vec![w])
    };
    h.run_for(4_000, &minimised);
    let sweep = h.item("old.txt").last_sweep_id.unwrap();
    assert!(h.engine.undoable_sweep_ids().unwrap().contains(&sweep));
    h.request(1, decay_bridge::kinds::ARCHIVE_UNDO, json!({"sweepId": sweep}));
    let restore = h.take_cmds();
    assert_eq!(restore.len(), 1);
    assert_eq!((restore[0].op, restore[0].window_id), (WindowOp::Restore, 1));
    h.acted(&restore[0], Ok(()));
    let item = h.item("old.txt");
    assert_eq!(item.state, ItemState::Active);
    assert!(item.grace_until.is_some_and(|g| g > h.now + 47 * 3_600_000));
    assert_eq!(h.store().sweep(&sweep).unwrap().unwrap().status, SweepStatus::Undone);
    // Old, but within its 48 h: not hidden again.
    h.engine.dev_add_age(100 * H, h.now);
    h.run_for(20 * 60_000, &one);
    assert_eq!(h.take_cmds(), []);
}

#[test]
fn a_hidden_window_that_is_closed_is_forgotten() {
    let mut h = scene(Mode::Automatic, &one, 100);
    h.run_for(40_000, &one);
    let cmd = h.take_cmds().remove(0);
    h.acted(&cmd, Ok(()));
    h.run_for(70_000, &Vec::new); // the window (and app) are gone
    assert_eq!(h.windows_items(), []);
}

// --- Regressions from the P2.2 review and QA ---------------------------------------------

fn minimised() -> Vec<AppWindows> {
    let mut w = window(1, "old.txt");
    w.minimized = true;
    list(vec![w])
}

/// A hidden window and its sweep id.
fn hidden(h: &mut Harness) -> String {
    h.run_for(40_000, &one);
    let cmd = h.take_cmds().remove(0);
    h.acted(&cmd, Ok(()));
    h.item("old.txt").last_sweep_id.unwrap()
}

fn restart(h: &mut Harness) {
    use decay_engine::{Engine, EngineConfig};
    let config = || EngineConfig { platform: decay_store::Platform::MacOs, pro: false };
    let spare_dir = tempfile::tempdir().unwrap();
    let spare = Engine::new(open_store(spare_dir.path(), h.now), config(), h.now).unwrap();
    let store = std::mem::replace(&mut h.engine, spare).into_store();
    h.engine = Engine::new(store, config(), h.now).unwrap();
}

#[test]
fn a_list_taken_before_the_hide_does_not_bring_the_window_back() {
    let mut h = scene(Mode::Automatic, &one, 100);
    let before = h.now;
    hidden(&mut h);
    // A list the lister started before the window was minimised arrives late.
    let out = h.engine.handle(
        Input::Windows(Box::new(WindowList { apps: one(), started_at: Some(before) })),
        h.now,
    );
    h.collect(out);
    assert_eq!(h.item("old.txt").state, ItemState::Hidden);
}

#[test]
fn undo_through_the_app_and_a_failed_restore() {
    let mut h = scene(Mode::Automatic, &one, 100);
    let sweep = hidden(&mut h);
    h.run_for(4_000, &minimised);
    let out = h.engine.undo(Some(&sweep), h.now).unwrap().unwrap();
    h.collect(out);
    let restore = h.take_cmds().remove(0);
    assert_eq!(restore.op, WindowOp::Restore);
    assert!(
        h.events
            .iter()
            .any(|e| matches!(e, EngineEvent::SweepUndone { restored: 1, failed: 0, .. }))
    );
    // The actor couldn't un-minimise it: still hidden (not "active" in the Dock).
    h.acted(&restore, Err("the app isn't answering".into()));
    assert_eq!(h.item("old.txt").state, ItemState::Hidden);
}

#[test]
fn undo_after_a_restart_waits_for_the_windows_to_be_seen() {
    let mut h = scene(Mode::Automatic, &one, 100);
    let sweep = hidden(&mut h);
    restart(&mut h);
    let refused = h.engine.undo(Some(&sweep), h.now).unwrap();
    assert!(matches!(refused, Err(decay_engine::UndoRefused::WindowsUnknown)), "{refused:?}");
    assert_eq!(h.item("old.txt").state, ItemState::Hidden, "nothing changed");
    // Once the windows are listed (a new process id after the restart), Undo works.
    let relisted = || {
        let mut a = minimised();
        a[0].pid = 77;
        a
    };
    h.run_for(4_000, &relisted);
    let out = h.engine.undo(Some(&sweep), h.now).unwrap().unwrap();
    h.collect(out);
    let cmds = h.take_cmds();
    assert_eq!((cmds.len(), cmds[0].pid), (1, 77));
}

#[test]
fn a_hide_sweep_cut_short_by_a_restart_is_settled_by_the_first_list() {
    for minimised_after in [true, false] {
        let mut h = scene(Mode::Automatic, &one, 100);
        h.run_for(40_000, &one);
        let cmd = h.take_cmds().remove(0); // Decay quits before the answer
        let sweep = h.store().recent_sweeps(5).unwrap().remove(0).id;
        restart(&mut h);
        h.run_for(4_000, &|| if minimised_after { minimised() } else { one() });
        let s = h.store().sweep(&sweep).unwrap().unwrap();
        assert_eq!(s.status, SweepStatus::Done, "minimised: {minimised_after}");
        let want = if minimised_after { ItemState::Hidden } else { ItemState::Stale };
        assert_eq!(h.item("old.txt").state, want);
        assert_eq!(
            (s.item_count, s.failed_count),
            (u32::from(minimised_after), u32::from(!minimised_after))
        );
        drop(cmd);
    }
}

#[test]
fn nothing_is_hidden_without_knowing_the_app_in_front() {
    let mut h = scene(Mode::Automatic, &one, 100);
    let end = h.now + 40_000;
    while h.now < end {
        h.now += 1_000;
        let mut s = os(600, 600);
        s.frontmost = None;
        h.os(s);
        if h.now % 3_000 < 1_000 {
            h.windows(one());
        }
        h.poll();
    }
    assert_eq!(h.take_cmds(), []);
}

#[test]
fn turning_window_handling_off_brings_hidden_windows_back() {
    let mut h = scene(Mode::Automatic, &one, 100);
    hidden(&mut h);
    h.run_for(4_000, &minimised);
    h.store().set_setting("window_management", &"off", h.now).unwrap();
    let out = h.engine.settings_changed(h.now).unwrap();
    h.collect(out);
    let cmds = h.take_cmds();
    assert_eq!(cmds.len(), 1);
    assert_eq!((cmds[0].op, cmds[0].window_id), (WindowOp::Restore, 1));
    assert_eq!(h.windows_items(), []);
}

#[test]
fn commands_carry_a_deadline_for_the_actor() {
    let mut h = scene(Mode::Automatic, &one, 100);
    h.run_for(40_000, &one);
    let cmd = h.take_cmds().remove(0);
    assert!(cmd.deadline > h.now && cmd.deadline <= h.now + 10_000);
}
