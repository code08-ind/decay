//! App windows (P2.1): tracking from the platform's window lists, aging in active work
//! time, focus as use, matching across app and Decay restarts, and the cases where
//! nothing may happen (window handling off, no Accessibility, browsers, Decay itself).

#![allow(clippy::needless_pass_by_value)]

mod support;

use decay_core::{ItemKind, ItemState, Mode, SaveState};
use decay_engine::{Engine, EngineConfig, Input};
use decay_platform::{AppWindow, AppWindows, FrontApp, WindowList};
use decay_store::{Item, Platform};
#[allow(clippy::wildcard_imports)]
use support::*;

const TEXTEDIT: &str = "com.apple.TextEdit";

fn window(id: u32, title: &str, doc: Option<&str>) -> AppWindow {
    AppWindow {
        window_id: Some(id),
        title: title.into(),
        document: doc.map(Into::into),
        edited: Some(false),
        can_minimize: true,
        ..AppWindow::default()
    }
}

fn app(pid: i32, bundle: &str, windows: Vec<AppWindow>) -> AppWindows {
    // By default the window server knows exactly the windows Accessibility listed.
    let all_window_ids = windows.iter().filter_map(|w| w.window_id).collect();
    AppWindows {
        pid,
        bundle_id: bundle.into(),
        name: bundle.rsplit('.').next().unwrap().into(),
        windows,
        all_window_ids,
    }
}

impl Harness {
    fn windows(&mut self, apps: Vec<AppWindows>) {
        let out = self.engine.handle(
            Input::Windows(Box::new(WindowList { apps, started_at: Some(self.now) })),
            self.now,
        );
        self.collect(out);
    }

    /// An OS sample: active (idle 1 s), `front` in front.
    fn front(&mut self, pid: i32) {
        let mut s = os(1, 120);
        s.frontmost = Some(FrontApp { pid, name: "front".into() });
        self.os(s);
    }

    fn window_items(&self) -> Vec<Item> {
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

    /// Ticks for `ms` with fresh OS samples and window lists (`apps` each time).
    fn run_for(&mut self, ms: i64, front: i32, apps: &dyn Fn() -> Vec<AppWindows>) {
        let end = self.now + ms;
        while self.now < end {
            self.now = (self.now + 1_000).min(end);
            self.front(front);
            if self.now % 3_000 < 1_000 {
                self.windows(apps());
            }
            self.poll();
        }
    }
}

fn tracking() -> Harness {
    let mut h = Harness::new();
    h.store().set_setting("window_management", &"hide", h.now).unwrap();
    h.set_mode(Mode::Automatic);
    let out = h.engine.settings_changed(h.now).unwrap();
    h.collect(out);
    h
}

#[test]
fn windows_become_items_with_their_document_and_save_state() {
    let mut h = tracking();
    let mut edited = window(2, "Draft", None);
    edited.edited = Some(true);
    h.windows(vec![app(
        50,
        TEXTEDIT,
        vec![window(1, "Report.txt", Some("file:///Users/me/My%20Report.txt")), edited],
    )]);
    let items = h.window_items();
    assert_eq!(items.len(), 2);
    let draft = &items[0];
    let report = &items[1];
    assert_eq!((report.app_id.as_str(), report.kind), (TEXTEDIT, ItemKind::Window));
    assert_eq!(report.path.as_deref(), Some("/Users/me/My Report.txt"));
    assert_eq!(report.save_state, SaveState::Saved);
    assert_eq!(report.external_id.as_deref(), Some("1"));
    assert_eq!(draft.save_state, SaveState::Unsaved);
    assert_eq!(h.store().app(TEXTEDIT).unwrap().unwrap().platform, Platform::MacOs);
    assert_eq!(h.engine.tracked_windows(), 2);
}

#[test]
fn browsers_decay_itself_and_window_handling_off_are_never_tracked() {
    let mut h = tracking();
    h.windows(vec![
        app(10, "com.google.Chrome", vec![window(1, "Inbox", None)]),
        app(11, "com.decay.app", vec![window(2, "Library", None)]),
    ]);
    assert_eq!(h.window_items(), []);
    // Off: nothing tracked, and live window records already kept are dropped.
    h.windows(vec![app(50, TEXTEDIT, vec![window(3, "Notes", None)])]);
    assert_eq!(h.window_items().len(), 1);
    h.store().set_setting("window_management", &"off", h.now).unwrap();
    let out = h.engine.settings_changed(h.now).unwrap();
    h.collect(out);
    h.windows(vec![app(50, TEXTEDIT, vec![window(3, "Notes", None)])]);
    assert_eq!(h.window_items(), []);
    assert_eq!(h.engine.tracked_windows(), 0);
}

#[test]
fn unfocused_windows_age_the_focused_one_in_front_does_not_and_old_ones_go_stale() {
    let mut h = tracking();
    let apps = || {
        let mut focused = window(1, "Working", None);
        focused.focused = true;
        vec![app(50, TEXTEDIT, vec![focused, window(2, "Forgotten", None)])]
    };
    h.windows(apps());
    h.run_for(5 * 60_000, 50, &apps);
    let items = h.window_items();
    let (forgotten, working) = (&items[0], &items[1]);
    assert!(forgotten.active_age_s >= 4 * 60, "aged: {}", forgotten.active_age_s);
    assert_eq!(working.active_age_s, 0, "the focused window of the app in front is in use");
    // Another app in front: the focused TextEdit window ages too.
    h.run_for(2 * 60_000, 99, &apps);
    assert!(h.window_items()[1].active_age_s > 0);
    // Past the stale threshold (12 h): stale, never closed or hidden by P2.1.
    h.engine.dev_add_age(13 * H, h.now);
    h.run_for(60_000, 99, &apps);
    assert!(h.window_items().iter().all(|i| i.state == ItemState::Stale));
    assert!(h.sent.iter().all(|(_, e)| e.kind != "tabs.close"));
}

#[test]
fn a_minimised_focused_window_is_not_in_use() {
    let mut h = tracking();
    let apps = || {
        let mut w = window(1, "Down", None);
        w.focused = true;
        w.minimized = true;
        vec![app(50, TEXTEDIT, vec![w])]
    };
    h.windows(apps());
    h.run_for(2 * 60_000, 50, &apps);
    assert!(h.window_items()[0].active_age_s > 0);
}

#[test]
fn a_closed_window_is_forgotten_after_a_minute_unless_it_comes_back() {
    let mut h = tracking();
    let both = || vec![app(50, TEXTEDIT, vec![window(1, "A", None), window(2, "B", None)])];
    let one = || vec![app(50, TEXTEDIT, vec![window(1, "A", None)])];
    h.windows(both());
    h.engine.dev_add_age(2 * H, h.now);
    // B disappears for 30 s and comes back: same record, same age.
    h.run_for(30_000, 99, &one);
    h.run_for(3_000, 99, &both);
    let b = h.window_items().into_iter().find(|i| i.title == "B").unwrap();
    assert!(b.active_age_s >= 2 * H);
    // Gone for over a minute: the record is deleted (the user closed it).
    h.run_for(70_000, 99, &one);
    assert_eq!(h.window_items().iter().map(|i| i.title.as_str()).collect::<Vec<_>>(), ["A"]);
}

#[test]
fn an_app_restart_keeps_ages_by_document_then_title() {
    let mut h = tracking();
    h.windows(vec![app(
        50,
        TEXTEDIT,
        vec![window(1, "a.txt", Some("file:///a.txt")), window(2, "Untitled", None)],
    )]);
    h.engine.dev_add_age(5 * H, h.now);
    let before: Vec<String> = h.window_items().into_iter().map(|i| i.id).collect();
    // Relaunched: new pid, new window numbers, same documents / titles.
    h.windows(vec![app(
        60,
        TEXTEDIT,
        vec![window(7, "a.txt", Some("file:///a.txt")), window(8, "Untitled", None)],
    )]);
    let after = h.window_items();
    assert_eq!(after.iter().map(|i| i.id.clone()).collect::<Vec<_>>(), before);
    assert!(after.iter().all(|i| i.active_age_s >= 5 * H));
    let doc = after.iter().find(|i| i.title == "a.txt").unwrap();
    assert_eq!(doc.external_id.as_deref(), Some("7"));
}

#[test]
fn a_decay_restart_reuses_the_stored_records() {
    let mut h = tracking();
    let list = || vec![app(50, TEXTEDIT, vec![window(1, "a.txt", Some("file:///a.txt"))])];
    h.windows(list());
    h.engine.dev_add_age(3 * H, h.now);
    let id = h.window_items()[0].id.clone();
    // A new engine over the same database (Decay restarted).
    let config = || EngineConfig { platform: Platform::MacOs, pro: false };
    let spare_dir = tempfile::tempdir().unwrap();
    let spare = Engine::new(open_store(spare_dir.path(), h.now), config(), h.now).unwrap();
    let store = std::mem::replace(&mut h.engine, spare).into_store();
    h.engine = Engine::new(store, config(), h.now).unwrap();
    h.windows(list());
    let items = h.window_items();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].id, id);
    assert!(items[0].active_age_s >= 3 * H);
}

#[test]
fn without_window_lists_nothing_ages_and_nothing_is_forgotten() {
    let mut h = tracking();
    let list = || vec![app(50, TEXTEDIT, vec![window(1, "a", None)])];
    h.windows(list());
    // Accessibility revoked: lists stop coming. Ten minutes later the record is still
    // there and hasn't aged.
    let end = h.now + 10 * 60_000;
    while h.now < end {
        h.now += 1_000;
        h.front(99);
        h.poll();
    }
    let item = &h.window_items()[0];
    assert_eq!(item.active_age_s, 0);
    assert_eq!(h.engine.tracked_windows(), 1);
}

#[test]
fn title_exclusion_rules_keep_windows_out() {
    let mut h = tracking();
    let rule = decay_engine::RuleInput {
        scope: decay_core::RuleScope::TitleRegex,
        pattern: "Secret".into(),
        action: decay_core::RuleAction::Exclude,
        threshold_hours: None,
        enabled: None,
    };
    let (_, _, out) = h.engine.create_rule(&rule, h.now).unwrap();
    h.collect(out);
    h.windows(vec![app(
        50,
        TEXTEDIT,
        vec![window(1, "Secret plan", None), window(2, "Groceries", None)],
    )]);
    assert_eq!(
        h.window_items().iter().map(|i| i.title.as_str()).collect::<Vec<_>>(),
        ["Groceries"]
    );
}

// --- Regressions from the P2.1 QA pass -------------------------------------------------

#[test]
fn windows_on_another_space_or_of_a_slow_app_are_kept() {
    let mut h = tracking();
    let both = || vec![app(50, TEXTEDIT, vec![window(1, "Here", None), window(2, "There", None)])];
    h.windows(both());
    h.engine.dev_add_age(H, h.now);
    // Accessibility now lists only the current Space; the window server still has #2.
    let here_only = || {
        let mut a = app(50, TEXTEDIT, vec![window(1, "Here", None)]);
        a.all_window_ids = vec![1, 2];
        vec![a]
    };
    h.run_for(3 * 60_000, 99, &here_only);
    let titles: Vec<String> = h.window_items().into_iter().map(|i| i.title).collect();
    assert_eq!(titles, ["Here", "There"], "a window on another Space isn't closed");
    // The app doesn't answer Accessibility at all (busy) for 3 minutes: all kept.
    let busy = || {
        let mut a = app(50, TEXTEDIT, vec![]);
        a.all_window_ids = vec![1, 2];
        vec![a]
    };
    h.run_for(3 * 60_000, 99, &busy);
    assert_eq!(h.window_items().len(), 2);
    // The window server drops #2 too: really closed, forgotten after a minute.
    h.run_for(70_000, 99, &|| vec![app(50, TEXTEDIT, vec![window(1, "Here", None)])]);
    assert_eq!(h.window_items().len(), 1);
}

#[test]
fn an_untitled_window_without_a_number_keeps_one_record() {
    let mut h = tracking();
    let list = || {
        let mut w = window(1, "", None);
        w.window_id = None;
        vec![app(50, TEXTEDIT, vec![w])]
    };
    h.windows(list());
    h.run_for(2 * 60_000, 99, &list);
    let items = h.window_items();
    assert_eq!(items.len(), 1, "one window, one record");
    assert!(items[0].active_age_s >= 60);
}

#[test]
fn turning_window_handling_off_drops_every_live_window_record() {
    // Off while no lists come (no Accessibility): dropped at once.
    let mut h = tracking();
    h.windows(vec![app(50, TEXTEDIT, vec![window(1, "a", None)])]);
    h.store().set_setting("window_management", &"off", h.now).unwrap();
    let out = h.engine.settings_changed(h.now).unwrap();
    h.collect(out);
    assert_eq!(h.window_items(), []);
    // Off while Decay wasn't running: dropped at start-up.
    let mut h = tracking();
    h.windows(vec![app(50, TEXTEDIT, vec![window(1, "a", None)])]);
    h.store().set_setting("window_management", &"off", h.now).unwrap();
    let config = || EngineConfig { platform: Platform::MacOs, pro: false };
    let spare_dir = tempfile::tempdir().unwrap();
    let spare = Engine::new(open_store(spare_dir.path(), h.now), config(), h.now).unwrap();
    let store = std::mem::replace(&mut h.engine, spare).into_store();
    h.engine = Engine::new(store, config(), h.now).unwrap();
    assert_eq!(h.window_items(), []);
    assert!(!h.engine.wants_windows());
}

#[test]
fn a_reused_window_number_after_a_relaunch_is_a_new_window() {
    let mut h = tracking();
    h.windows(vec![app(50, TEXTEDIT, vec![window(5, "old.txt", Some("file:///old.txt"))])]);
    h.engine.dev_add_age(10 * H, h.now);
    let old = h.window_items()[0].id.clone();
    h.now += 3_000;
    h.windows(vec![app(60, TEXTEDIT, vec![window(5, "new.txt", Some("file:///new.txt"))])]);
    let new = h.window_items().into_iter().find(|i| i.title == "new.txt").unwrap();
    assert_ne!(new.id, old);
    assert_eq!(new.active_age_s, 0);
}

#[test]
fn a_window_that_went_missing_does_not_age_meanwhile() {
    let mut h = tracking();
    let both = || vec![app(50, TEXTEDIT, vec![window(1, "A", None), window(2, "B", None)])];
    let one = || vec![app(50, TEXTEDIT, vec![window(1, "A", None)])];
    h.windows(both());
    h.run_for(50_000, 99, &one);
    let b = h.window_items().into_iter().find(|i| i.title == "B").unwrap();
    assert_eq!(b.active_age_s, 0);
}

#[test]
fn thousands_of_windows_are_handled_quickly() {
    let mut h = tracking();
    let list = || {
        (0..30)
            .map(|a| {
                let windows =
                    (0..100).map(|w| window(a * 1000 + w, &format!("w{a}-{w}"), None)).collect();
                app(i32::try_from(a).unwrap() + 100, &format!("com.example.app{a}"), windows)
            })
            .collect::<Vec<_>>()
    };
    h.windows(list());
    let start = std::time::Instant::now();
    h.now += 3_000;
    h.windows(list());
    let steady = start.elapsed();
    assert_eq!(h.engine.tracked_windows(), 3_000);
    // Generous for debug builds and busy CI machines; the point is "not quadratic".
    assert!(steady < std::time::Duration::from_secs(2), "steady list took {steady:?}");
}

// --- Regressions from the P2.1 code review ---------------------------------------------

/// The same database under a new engine (Decay restarted).
fn restart(h: &mut Harness) {
    let config = || EngineConfig { platform: Platform::MacOs, pro: false };
    let spare_dir = tempfile::tempdir().unwrap();
    let spare = Engine::new(open_store(spare_dir.path(), h.now), config(), h.now).unwrap();
    let store = std::mem::replace(&mut h.engine, spare).into_store();
    h.engine = Engine::new(store, config(), h.now).unwrap();
}

#[test]
fn after_a_restart_a_retitled_window_and_a_new_app_are_both_stored_and_age() {
    let mut h = tracking();
    h.windows(vec![app(50, TEXTEDIT, vec![window(5, "Untitled", None)])]);
    restart(&mut h);
    // While Decay was off: "Untitled" was saved as x.txt (same window number), and a
    // new app opened a window.
    let list = || {
        vec![
            app(50, TEXTEDIT, vec![window(5, "x.txt", Some("file:///x.txt"))]),
            app(70, "com.apple.Preview", vec![window(9, "scan.pdf", Some("file:///scan.pdf"))]),
        ]
    };
    h.windows(list());
    let titles: Vec<String> = h.window_items().into_iter().map(|i| i.title).collect();
    // Both stored (the old `Untitled` record lost its number and is in its grace minute).
    assert!(titles.contains(&"scan.pdf".to_owned()) && titles.contains(&"x.txt".to_owned()));
    h.run_for(2 * 60_000, 99, &list);
    assert!(h.window_items().iter().all(|i| i.active_age_s >= 60), "and they age");
    assert_eq!(h.window_items().len(), 2, "the stale `Untitled` record didn't stay behind");
}

#[test]
fn after_a_reboot_numbers_restart_and_old_records_are_cleaned_up() {
    let mut h = tracking();
    h.windows(vec![app(
        50,
        TEXTEDIT,
        vec![
            window(40, "a.txt", Some("file:///a.txt")),
            window(41, "b.txt", Some("file:///b.txt")),
        ],
    )]);
    h.engine.dev_add_age(3 * H, h.now);
    restart(&mut h);
    // After the reboot only a.txt is open again, under a number b.txt used to have.
    let list = || vec![app(60, TEXTEDIT, vec![window(41, "a.txt", Some("file:///a.txt"))])];
    h.windows(list());
    h.run_for(70_000, 99, &list);
    let items = h.window_items();
    assert_eq!(items.len(), 1, "b.txt (never reopened) is forgotten");
    assert_eq!(items[0].title, "a.txt");
    assert!(items[0].active_age_s >= 3 * H, "a.txt kept its age");
    assert_eq!(items[0].external_id.as_deref(), Some("41"));
}

#[test]
fn a_number_moving_between_two_records_is_saved_every_time() {
    // The write order once depended on hash order: run it many times.
    for _ in 0..30 {
        let mut h = tracking();
        h.windows(vec![app(
            50,
            TEXTEDIT,
            vec![window(5, "old.txt", Some("file:///old.txt")), window(9, "Untitled", None)],
        )]);
        h.now += 3_000;
        // Relaunched: the untitled window now has #5; old.txt is gone.
        h.windows(vec![app(60, TEXTEDIT, vec![window(5, "Untitled", None)])]);
        let untitled = h.window_items().into_iter().find(|i| i.title == "Untitled").unwrap();
        assert_eq!(untitled.external_id.as_deref(), Some("5"));
        // Swapped numbers in one list: both saved.
        h.now += 3_000;
        h.windows(vec![app(
            60,
            TEXTEDIT,
            vec![window(9, "Untitled", None), window(5, "c.txt", Some("file:///c.txt"))],
        )]);
        let items = h.window_items();
        let ext = |t: &str| items.iter().find(|i| i.title == t).and_then(|i| i.external_id.clone());
        assert_eq!((ext("Untitled").as_deref(), ext("c.txt").as_deref()), (Some("9"), Some("5")));
    }
}
