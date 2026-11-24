//! Closing hidden folder windows (P4.1, PRD F11): only with Pro, only allowlisted apps'
//! saved windows Decay already hid; the record is written before the close; only a
//! confirmed close (or a window list showing it gone) archives; restoring and Undo
//! reopen the folder and count only once it opened.

#![allow(clippy::needless_pass_by_value)]

mod support;

use std::cell::{Cell, RefCell};

use decay_core::{EventType, ItemKind, ItemState, Mode, SweepStatus};
use decay_engine::{EngineEvent, Input, WindowCommand};
use decay_platform::{AppWindow, AppWindows, FrontApp, WindowList, WindowOp};
use decay_store::Item;
#[allow(clippy::wildcard_imports)]
use support::*;

const FINDER: &str = "com.apple.finder";
const PID: i32 = 60;

thread_local! {
    /// What the next lists show: the window exists, and is minimised.
    static SHOWN: Cell<bool> = const { Cell::new(true) };
    static MINIMISED: Cell<bool> = const { Cell::new(false) };
    /// The window shows its folder, and has unsaved changes.
    static HAS_FOLDER: Cell<bool> = const { Cell::new(true) };
    static EDITED: Cell<bool> = const { Cell::new(false) };
    /// The folder window's record.
    static ITEM: RefCell<String> = const { RefCell::new(String::new()) };
}

fn folder_url() -> String {
    format!("file://{}/", std::env::temp_dir().display())
}

fn finder_windows() -> Vec<AppWindows> {
    let mut windows = Vec::new();
    if SHOWN.get() {
        windows.push(AppWindow {
            window_id: Some(7),
            title: "Downloads".into(),
            document: HAS_FOLDER.get().then(folder_url),
            edited: Some(EDITED.get()),
            can_minimize: true,
            minimized: MINIMISED.get(),
            ..AppWindow::default()
        });
    }
    let all_window_ids = windows.iter().filter_map(|w| w.window_id).collect();
    vec![AppWindows {
        pid: PID,
        bundle_id: FINDER.into(),
        name: "Finder".into(),
        windows,
        all_window_ids,
    }]
}

impl Harness {
    fn list_windows(&mut self) {
        let list = WindowList { apps: finder_windows(), started_at: Some(self.now) };
        let out = self.engine.handle(Input::Windows(Box::new(list)), self.now);
        self.collect(out);
    }

    /// Quiet seconds (another app in front) with a list every 3 s, until the engine asks
    /// for a window command or `ms` pass.
    fn run_until_cmd(&mut self, ms: i64) -> Vec<WindowCommand> {
        let end = self.now + ms;
        while self.now < end && self.window_cmds.is_empty() {
            self.now += 1_000;
            let mut s = os(600, 600);
            s.frontmost = Some(FrontApp { pid: 99, name: "Other".into() });
            self.os(s);
            if self.now % 3_000 < 1_000 {
                self.list_windows();
            }
            self.poll();
        }
        std::mem::take(&mut self.window_cmds)
    }

    fn answer(&mut self, cmd: &WindowCommand, result: Result<(), String>) {
        let input = Input::WindowActed { item_id: cmd.item_id.clone(), result };
        let out = self.engine.handle(input, self.now);
        self.collect(out);
    }

    fn folder_item(&self) -> Item {
        let id = ITEM.with_borrow(Clone::clone);
        self.store().item(&id).unwrap().expect("the folder window's record")
    }
}

/// Pro or not, window handling on, automatic mode, the Finder window listed, 70 h old
/// (past Balanced's archive threshold of 60 h), then hidden by a hide sweep.
fn hidden_folder(pro: bool) -> Harness {
    SHOWN.set(true);
    MINIMISED.set(false);
    HAS_FOLDER.set(true);
    EDITED.set(false);
    let mut h = Harness::with_pro(pro);
    h.store().set_setting("window_management", &"hide", h.now).unwrap();
    h.set_mode(Mode::Automatic);
    let out = h.engine.settings_changed(h.now).unwrap();
    h.collect(out);
    h.list_windows();
    let id = h
        .store()
        .live_items()
        .unwrap()
        .into_iter()
        .find(|i| i.kind == ItemKind::Window && i.app_id == FINDER)
        .unwrap()
        .id;
    ITEM.set(id);
    h.engine.dev_add_age(70 * H, h.now);
    let cmds = h.run_until_cmd(60_000);
    assert_eq!(cmds.iter().map(|c| c.op).collect::<Vec<_>>(), [WindowOp::Hide]);
    h.answer(&cmds[0], Ok(()));
    MINIMISED.set(true);
    assert_eq!(h.folder_item().state, ItemState::Hidden);
    h
}

#[test]
fn a_hidden_folder_window_is_closed_after_its_record_is_written() {
    let mut h = hidden_folder(true);
    let cmds = h.run_until_cmd(60_000);
    assert_eq!(cmds.len(), 1);
    assert_eq!((cmds[0].op, cmds[0].pid, cmds[0].window_id), (WindowOp::Close, PID, 7));
    // Written first: the record archiving, its sweep running.
    let item = h.folder_item();
    assert_eq!(item.state, ItemState::Archiving);
    let sweep = h.store().sweep(item.last_sweep_id.as_deref().unwrap()).unwrap().unwrap();
    assert_eq!(sweep.status, SweepStatus::Running);
    assert!(sweep.reason.as_deref().unwrap().contains("close"));

    SHOWN.set(false);
    h.answer(&cmds[0], Ok(()));
    let item = h.folder_item();
    assert_eq!(item.state, ItemState::Archived);
    let dir = std::env::temp_dir().display().to_string();
    assert_eq!(
        item.path.as_deref().map(|p| p.trim_end_matches('/')),
        Some(dir.trim_end_matches('/'))
    );
    let done = h.store().sweep(&sweep.id).unwrap().unwrap();
    assert_eq!((done.status, done.item_count, done.failed_count), (SweepStatus::Done, 1, 0));
    assert!(
        h.events
            .iter()
            .any(|e| matches!(e, EngineEvent::WindowsClosed { closed: 1, failed: 0, .. }))
    );
    assert!(events_of(h.store(), &item.id).contains(&EventType::Archived));
    // The window being gone from later lists changes nothing.
    assert_eq!(h.run_until_cmd(5 * 60_000), []);
    assert_eq!(h.folder_item().state, ItemState::Archived);
}

#[test]
fn without_pro_hidden_windows_stay_hidden() {
    let mut h = hidden_folder(false);
    assert_eq!(h.run_until_cmd(30 * 60_000), []);
    assert_eq!(h.folder_item().state, ItemState::Hidden);
}

#[test]
fn a_refused_close_proves_nothing_until_a_list_shows_the_window() {
    let mut h = hidden_folder(true);
    let cmds = h.run_until_cmd(60_000);
    h.answer(&cmds[0], Err("the window didn't close".into()));
    assert_eq!(h.folder_item().state, ItemState::Archiving, "not settled by the answer");
    // The next list still shows it: the close didn't happen.
    h.list_windows();
    let item = h.folder_item();
    assert_eq!(item.state, ItemState::Hidden);
    let sweep = h.store().sweep(item.last_sweep_id.as_deref().unwrap()).unwrap().unwrap();
    assert_eq!((sweep.status, sweep.item_count, sweep.failed_count), (SweepStatus::Done, 0, 1));
    assert!(events_of(h.store(), &item.id).contains(&EventType::CloseFailed));
}

#[test]
fn a_close_nobody_answered_is_archived_once_the_window_is_gone() {
    let mut h = hidden_folder(true);
    let cmds = h.run_until_cmd(60_000);
    assert_eq!(cmds[0].op, WindowOp::Close);
    // No answer; the window closed after all.
    SHOWN.set(false);
    h.now += 11_000;
    h.poll();
    assert_eq!(h.folder_item().state, ItemState::Archiving);
    h.list_windows();
    assert_eq!(h.folder_item().state, ItemState::Archived);
    assert!(h.events.iter().any(|e| matches!(e, EngineEvent::WindowsClosed { closed: 1, .. })));
}

#[test]
fn unsaved_or_folderless_windows_are_never_closed() {
    type Tweak = fn();
    let cases: [(&str, Tweak); 2] =
        [("unsaved", || EDITED.set(true)), ("no folder", || HAS_FOLDER.set(false))];
    for (what, tweak) in cases {
        let mut h = hidden_folder(true);
        tweak();
        let cmds = h.run_until_cmd(30 * 60_000);
        assert!(cmds.iter().all(|c| c.op != WindowOp::Close), "{what}: {cmds:?}");
        assert_eq!(h.folder_item().state, ItemState::Hidden, "{what}");
    }
}

fn archived_folder() -> Harness {
    let mut h = hidden_folder(true);
    let cmds = h.run_until_cmd(60_000);
    SHOWN.set(false);
    h.answer(&cmds[0], Ok(()));
    assert_eq!(h.folder_item().state, ItemState::Archived);
    h.events.clear();
    h
}

#[test]
fn restoring_an_archived_window_reopens_its_folder() {
    let mut h = archived_folder();
    let id = h.folder_item().id;
    let out = h.engine.restore(std::slice::from_ref(&id), false, h.now).unwrap().unwrap();
    h.collect(out);
    let cmds = std::mem::take(&mut h.window_cmds);
    assert_eq!(cmds.len(), 1);
    assert_eq!(cmds[0].reopen.as_deref(), h.folder_item().path.as_deref());
    assert_eq!(h.folder_item().state, ItemState::Archived, "only once it opened");
    h.answer(&cmds[0], Ok(()));
    assert_eq!(h.folder_item().state, ItemState::Restored);
    assert!(
        h.events.iter().any(|e| matches!(e, EngineEvent::Restored { restored: 1, failed: 0, .. }))
    );
    assert!(events_of(h.store(), &id).contains(&EventType::Restored));
}

#[test]
fn a_folder_that_doesnt_reopen_stays_archived() {
    let mut h = archived_folder();
    let id = h.folder_item().id;
    let out = h.engine.restore(std::slice::from_ref(&id), false, h.now).unwrap().unwrap();
    h.collect(out);
    let cmds = std::mem::take(&mut h.window_cmds);
    h.answer(&cmds[0], Err("the folder no longer exists".into()));
    assert_eq!(h.folder_item().state, ItemState::Archived);
    assert!(
        h.events.iter().any(|e| matches!(e, EngineEvent::Restored { restored: 0, failed: 1, .. }))
    );
}

#[test]
fn undo_reopens_the_sweeps_folders() {
    let mut h = archived_folder();
    let item = h.folder_item();
    let sweep = item.last_sweep_id.clone().unwrap();
    let out = h.engine.undo(Some(&sweep), h.now).unwrap().unwrap();
    h.collect(out);
    let cmds = std::mem::take(&mut h.window_cmds);
    assert_eq!(cmds.len(), 1);
    assert!(cmds[0].reopen.is_some());
    h.answer(&cmds[0], Ok(()));
    assert_eq!(h.folder_item().state, ItemState::Restored);
    assert!(events_of(h.store(), &item.id).contains(&EventType::Undo));
    let undone = h.store().sweep(&sweep).unwrap().unwrap();
    assert_eq!(undone.status, SweepStatus::Undone);
    assert!(
        h.events
            .iter()
            .any(|e| matches!(e, EngineEvent::SweepUndone { restored: 1, failed: 0, .. }))
    );
}

fn restart(h: &mut Harness) {
    use decay_engine::{Engine, EngineConfig};
    let config = || EngineConfig { platform: decay_store::Platform::MacOs, pro: true };
    let spare_dir = tempfile::tempdir().unwrap();
    let spare = Engine::new(open_store(spare_dir.path(), h.now), config(), h.now).unwrap();
    let store = std::mem::replace(&mut h.engine, spare).into_store();
    h.engine = Engine::new(store, config(), h.now).unwrap();
}

#[test]
fn a_close_cut_short_by_a_restart_is_settled_by_the_lists() {
    for closed_after in [true, false] {
        let mut h = hidden_folder(true);
        let cmds = h.run_until_cmd(60_000);
        assert_eq!(cmds[0].op, WindowOp::Close); // Decay quits before the answer
        restart(&mut h);
        SHOWN.set(!closed_after);
        h.list_windows();
        let want = if closed_after { ItemState::Archived } else { ItemState::Hidden };
        assert_eq!(h.folder_item().state, want, "closed: {closed_after}");
        let sweep = h.folder_item().last_sweep_id.unwrap();
        let s = h.store().sweep(&sweep).unwrap().unwrap();
        assert_eq!(s.status, SweepStatus::Done, "closed: {closed_after}");
    }
}
