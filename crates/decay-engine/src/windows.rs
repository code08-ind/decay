//! App windows (P2.1, PRD F1 / F4, Docs/Spikes.md): the platform lists every app's
//! standard windows every few seconds (`Input::Windows`); each becomes a `window` item
//! that ages in active work time like a tab and is used (age 0) while it's the focused
//! window of the app in front. Hiding stale windows is P2.2: here they only become stale.
//!
//! - Off unless `window_management` is `hide` (onboarding's "Window access").
//! - Browsers' windows are left to their extension (their tabs are tracked); Decay's own
//!   windows aren't tracked.
//! - Matching: the same window number; else, for a window that just appeared (the app was
//!   restarted, or Decay was), an untracked record of the same app with the same document
//!   (or, without one, the same title), so ages survive restarts.
//! - A window that's gone is forgotten (its record deleted: closed by the user) after a
//!   minute, so one that comes straight back keeps its age.
//! - No list (no Accessibility access, or the lister stalled) never means "closed": the
//!   records stay, frozen (no aging) until lists come again.
//! - Unsaved state: the close button's edited flag → unsaved; not edited with a document
//!   → saved; anything else → unknown (treated like unsaved: never closed).

use std::collections::{BTreeMap, HashMap, HashSet};

use decay_core::{
    Decision, EventType, FormInput, ItemKind, ItemState, SaveState, Signals, evaluate,
};
use decay_platform::{AppWindow, AppWindows, WindowList};
use decay_store::{App, Item, Platform, RestoreKind, Store, new_id};

use crate::control::WindowManagement;
use crate::engine::{Engine, PolicySettings, Res, timing};
use crate::sweep::{excluded, stale_reason, target};
use crate::tabs::mark_used;

/// Decay's own bundle id: its windows are never tracked.
const SELF_BUNDLE: &str = "com.decay.app";

/// A window list older than this is stale: windows don't age (Accessibility lost, or the
/// lister is stuck).
pub(crate) const WINDOWS_FRESH_MS: i64 = 30_000;

/// One tracked window.
#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools, reason = "independent facts the OS reports")]
pub(crate) struct TrackedWindow {
    pub item: Item,
    pub window_id: Option<u32>,
    pub pid: i32,
    pub focused: bool,
    /// In the latest Accessibility list (not just known to the window server).
    #[allow(dead_code, reason = "read by the hide executor (P2.2)")]
    pub visible: bool,
    pub minimized: bool,
    /// For hiding (P2.2): only windows that can be minimised and aren't full screen.
    #[allow(dead_code, reason = "read by the hide executor (P2.2)")]
    pub full_screen: bool,
    #[allow(dead_code, reason = "read by the hide executor (P2.2)")]
    pub can_minimize: bool,
}

impl TrackedWindow {
    /// A stored record, before any list has shown its window.
    fn loaded(item: Item, window_id: Option<u32>) -> Self {
        Self {
            item,
            window_id,
            pid: UNKNOWN_PID,
            focused: false,
            visible: false,
            minimized: false,
            full_screen: false,
            can_minimize: false,
        }
    }
}

/// The windows Decay tracks (by item id), and when each missing one went missing.
#[derive(Debug, Default, Clone)]
pub(crate) struct AppWindowsState {
    pub tracked: BTreeMap<String, TrackedWindow>,
    pub missing: HashMap<String, i64>,
    pub last_list_at: Option<i64>,
    /// Apps already written to the `apps` table this run.
    pub known_apps: HashSet<String>,
    /// The stored records were loaded (on the first list of the run).
    pub loaded: bool,
}

/// Browsers whose windows hold tabs the extension tracks.
pub(crate) fn is_browser(bundle_id: &str) -> bool {
    matches!(
        bundle_id,
        "com.google.Chrome"
            | "com.google.Chrome.beta"
            | "com.google.Chrome.dev"
            | "com.google.Chrome.canary"
            | "com.microsoft.edgemac"
            | "com.microsoft.edgemac.Beta"
            | "com.microsoft.edgemac.Dev"
            | "com.brave.Browser"
            | "com.brave.Browser.beta"
            | "company.thebrowser.Browser"
            | "org.chromium.Chromium"
            | "com.vivaldi.Vivaldi"
            // Windows (P3.1): the executable's lower-cased file name.
            | "chrome.exe"
            | "msedge.exe"
            | "brave.exe"
            | "arc.exe"
            | "chromium.exe"
            | "vivaldi.exe"
    )
}

/// Windows Decay never tracks: browsers' (their tabs are) and its own, by bundle id or by
/// process (Windows has no bundle ids).
fn not_tracked(app: &AppWindows) -> bool {
    app.bundle_id == SELF_BUNDLE
        || u32::try_from(app.pid).is_ok_and(|p| p == std::process::id())
        || is_browser(&app.bundle_id)
}

/// The save state a window reports (Docs/Spikes.md §2).
pub(crate) fn save_state_of(w: &AppWindow) -> SaveState {
    match (w.edited, &w.document) {
        (Some(true), _) => SaveState::Unsaved,
        (Some(false), Some(_)) => SaveState::Saved,
        _ => SaveState::Unknown,
    }
}

/// A `file://` URL as a path (percent-decoded); other URLs as they are.
pub(crate) fn document_path(url: &str) -> String {
    let Some(rest) = url.strip_prefix("file://") else { return url.to_owned() };
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push(u8::try_from(h * 16 + l).unwrap_or(b'?'));
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| rest.to_owned())
}

/// Whether `item` is the record of `w` in its app: same document, or (without one on
/// either side) the same title (untitled windows match each other).
fn same_window(item: &Item, w: &AppWindow) -> bool {
    match (&item.path, &w.document) {
        (Some(p), Some(d)) => *p == document_path(d),
        (None, None) => item.title == w.title,
        _ => false,
    }
}

/// Decay just started: every stored live window record joins the tracker (process
/// unknown until a list shows its window), so the same matching and expiry apply to it.
fn load_stored(state: &mut AppWindowsState, store: &Store) -> Res {
    for item in store.live_items()? {
        let live = matches!(item.state, ItemState::Active | ItemState::Stale | ItemState::Hidden);
        if item.kind == ItemKind::Window && live && !state.tracked.contains_key(&item.id) {
            let window_id = item.external_id.as_deref().and_then(|n| n.parse().ok());
            state.tracked.insert(item.id.clone(), TrackedWindow::loaded(item, window_id));
        }
    }
    state.loaded = true;
    Ok(())
}

/// Copies what the window shows into its record; whether anything changed.
fn refresh(item: &mut Item, w: &AppWindow) -> bool {
    let path = w.document.as_deref().map(document_path);
    let save_state = save_state_of(w);
    let external = w.window_id.map(|n| n.to_string());
    let changed = item.title != w.title
        || item.path != path
        || item.save_state != save_state
        || item.external_id != external;
    item.title.clone_from(&w.title);
    item.path = path;
    item.save_state = save_state;
    item.external_id = external;
    changed
}

/// The `apps` row for an app whose windows are tracked.
fn app_row(bundle: &str, name: &str) -> App {
    App {
        id: bundle.to_owned(),
        name: name.to_owned(),
        platform: Platform::MacOs,
        is_browser: false,
        close_allowed: false,
        restore_kind: RestoreKind::Document,
        icon_png: None,
    }
}

/// A process id no app has: records loaded from the store (Decay just started) until a
/// list shows their window.
pub(crate) const UNKNOWN_PID: i32 = -1;

/// What the latest list says about each app: the window numbers it still has on any
/// Space, and whether Accessibility answered for it at all.
#[derive(Debug, Default)]
struct Present {
    /// bundle → window numbers (window server, every Space).
    numbers: HashMap<String, HashSet<u32>>,
    /// Apps listed but whose windows Accessibility didn't return (a slow app, or only
    /// windows on other Spaces): their tracked windows are kept as they are.
    unanswered: HashSet<String>,
}

/// Windows not in the latest list: those that still exist (their number is still on the
/// window server, or their app didn't answer) are kept, out of sight; the others are
/// missing since now and forgotten (returned, to delete) after a minute, so one that
/// comes straight back keeps its record.
fn expire_missing(
    state: &mut AppWindowsState,
    seen: &HashSet<String>,
    present: &Present,
    now: i64,
) -> Vec<String> {
    let mut gone: Vec<String> = Vec::new();
    for (id, t) in &mut state.tracked {
        if seen.contains(id) {
            continue;
        }
        t.focused = false;
        t.visible = false;
        let app = &t.item.app_id;
        let still_there = present.unanswered.contains(app)
            || t.window_id
                .is_some_and(|n| present.numbers.get(app).is_some_and(|ns| ns.contains(&n)));
        if still_there {
            state.missing.remove(id);
            continue;
        }
        let since = *state.missing.entry(id.clone()).or_insert(now);
        if now - since >= timing::MISSING_GRACE_MS
            && matches!(t.item.state, ItemState::Active | ItemState::Stale | ItemState::Hidden)
        {
            gone.push(id.clone());
        }
    }
    for id in &gone {
        state.tracked.remove(id);
        state.missing.remove(id);
    }
    gone
}

/// The record of `w`, taken out of `state.tracked`, or `None` for a new window:
/// 1. the tracked window with its number, if it's the same process (or shows the same
///    thing: a relaunched app, or a reboot, may reuse a number for another window);
/// 2. a tracked window of the app whose number isn't in this list or belongs to another
///    process (the app or Decay was restarted), or that has none, showing the same
///    document / title: the most recently used.
fn find_record(
    state: &mut AppWindowsState,
    by_number: &HashMap<(String, u32), String>,
    seen: &HashSet<String>,
    current: &HashSet<u32>,
    app: (&str, i32),
    w: &AppWindow,
) -> Option<Item> {
    let (bundle, pid) = app;
    let numbered = w
        .window_id
        .and_then(|n| by_number.get(&(bundle.to_owned(), n)))
        .filter(|id| !seen.contains(*id))
        .filter(|id| {
            state.tracked.get(*id).is_some_and(|t| t.pid == pid || same_window(&t.item, w))
        })
        .cloned();
    let id = numbered.or_else(|| {
        state
            .tracked
            .iter()
            .filter(|(id, t)| {
                !seen.contains(*id)
                    && t.item.app_id == bundle
                    && (t.window_id.is_none_or(|n| !current.contains(&n)) || t.pid != pid)
                    && same_window(&t.item, w)
            })
            .max_by_key(|(_, t)| t.item.last_used_at)
            .map(|(id, _)| id.clone())
    })?;
    state.tracked.remove(&id).map(|t| t.item)
}

/// A number now shown by another window (an app reused it) no longer belongs to the
/// record that had it: that record loses it (written, freeing the number) and goes
/// missing unless it's seen again.
fn release_reused_numbers(
    state: &mut AppWindowsState,
    seen: &HashSet<String>,
    writes: &mut BTreeMap<String, (Item, bool)>,
) {
    let claimed: HashSet<(String, u32)> = seen
        .iter()
        .filter_map(|id| state.tracked.get(id))
        .filter_map(|t| t.window_id.map(|n| (t.item.app_id.clone(), n)))
        .collect();
    for (id, t) in &mut state.tracked {
        let reused = !seen.contains(id)
            && t.window_id.is_some_and(|n| claimed.contains(&(t.item.app_id.clone(), n)));
        if reused {
            t.window_id = None;
            t.item.external_id = None;
            writes.insert(id.clone(), (t.item.clone(), false));
        }
    }
}

/// Saves one list's changes in one transaction, in an order the "one live record per
/// window number" rule can't trip over: every changed record first gives up its number,
/// then the deletes, then the records get their numbers, then the new records.
fn write_changes(
    store: &mut Store,
    apps: &[App],
    writes: &BTreeMap<String, (Item, bool)>,
    deleted: &[String],
    used: &[String],
    now: i64,
) -> Res {
    store.atomically(false, |s| {
        for app in apps {
            s.upsert_app(app)?;
        }
        let updates: Vec<&Item> = writes
            .values()
            .filter(|(i, new)| !new && !deleted.contains(&i.id))
            .map(|(i, _)| i)
            .collect();
        for item in &updates {
            let mut freed = (*item).clone();
            freed.external_id = None;
            s.update_item(&freed)?;
        }
        for id in deleted {
            s.delete_item(id)?;
        }
        for item in &updates {
            s.update_item(item)?;
        }
        for (item, _) in writes.values().filter(|(_, new)| *new) {
            s.insert_item(item)?;
        }
        for id in used.iter().filter(|id| !deleted.contains(id)) {
            s.append_event(id, EventType::Used, None, None, now)?;
        }
        Ok(())
    })?;
    Ok(())
}

impl Engine {
    /// A fresh window list from the platform.
    pub(crate) fn on_windows(&mut self, list: WindowList, now: i64) {
        if let Err(e) = self.sync_windows(list, now) {
            self.problem("windows", &e);
        }
    }

    /// Whether window lists are wanted (the lister skips asking apps otherwise).
    pub fn wants_windows(&self) -> bool {
        self.policy.window_management == WindowManagement::Hide
    }

    /// Applies a list. Works on a copy of the tracker that replaces it only once the
    /// changes are saved: a failed write leaves memory and the database agreeing, and
    /// the next list tries again.
    fn sync_windows(&mut self, list: WindowList, now: i64) -> Res {
        if !self.wants_windows() {
            return self.forget_windows(now);
        }
        let policy = std::sync::Arc::clone(&self.policy);
        let mut state = self.app_windows.clone();
        let first_list = !state.loaded;
        if first_list {
            load_stored(&mut state, &self.store)?;
        }
        state.last_list_at = Some(now);
        let started = list.started_at.unwrap_or(now);
        let by_number: HashMap<(String, u32), String> = state
            .tracked
            .iter()
            .filter_map(|(id, t)| t.window_id.map(|n| ((t.item.app_id.clone(), n), id.clone())))
            .collect();
        let mut seen: HashSet<String> = HashSet::new();
        let mut present = Present::default();
        let mut apps: Vec<App> = Vec::new();
        let mut writes: BTreeMap<String, (Item, bool)> = BTreeMap::new(); // id → (item, new)
        let mut deleted: Vec<String> = Vec::new();
        let mut used: Vec<String> = Vec::new();
        for app in list.apps {
            if not_tracked(&app) {
                continue;
            }
            present
                .numbers
                .entry(app.bundle_id.clone())
                .or_default()
                .extend(app.all_window_ids.iter().copied());
            if app.windows.is_empty() {
                // The window server has windows Accessibility didn't return: keep them.
                if !app.all_window_ids.is_empty() {
                    present.unanswered.insert(app.bundle_id.clone());
                }
                continue;
            }
            if state.known_apps.insert(app.bundle_id.clone()) {
                apps.push(app_row(&app.bundle_id, &app.name));
            }
            let current: HashSet<u32> = app.windows.iter().filter_map(|w| w.window_id).collect();
            for w in &app.windows {
                let found = find_record(
                    &mut state,
                    &by_number,
                    &seen,
                    &current,
                    (&app.bundle_id, app.pid),
                    w,
                );
                let new = found.is_none();
                let mut item = found.unwrap_or_else(|| {
                    Item::new_live(new_id(now), ItemKind::Window, app.bundle_id.clone(), now)
                });
                let mut changed = refresh(&mut item, w) || new;
                // Brought back: only a list taken after it was hidden says so (a list
                // started before the hide can still arrive after its answer).
                if item.state == ItemState::Hidden
                    && !w.minimized
                    && started > item.state_changed_at
                {
                    // Brought back by the user (or by Undo): used.
                    item.state = ItemState::Active;
                    item.state_changed_at = now;
                    item.active_age_s = 0;
                    item.last_used_at = now;
                    used.push(item.id.clone());
                    changed = true;
                }
                if excluded(&item, &policy) {
                    // Excluded by a title rule: not stored, not tracked.
                    if !new {
                        deleted.push(item.id.clone());
                    }
                    continue;
                }
                if changed {
                    writes.insert(item.id.clone(), (item.clone(), new));
                }
                state.missing.remove(&item.id);
                seen.insert(item.id.clone());
                state.tracked.insert(
                    item.id.clone(),
                    TrackedWindow {
                        item,
                        window_id: w.window_id,
                        pid: app.pid,
                        focused: w.focused,
                        visible: true,
                        minimized: w.minimized,
                        full_screen: w.full_screen,
                        can_minimize: w.can_minimize,
                    },
                );
            }
        }
        release_reused_numbers(&mut state, &seen, &mut writes);
        deleted.extend(expire_missing(&mut state, &seen, &present, now));
        write_changes(&mut self.store, &apps, &writes, &deleted, &used, now)?;
        self.app_windows = state;
        if first_list {
            // Hide sweeps a quit or crash left running: settled against this list.
            self.settle_hide_sweeps(now)?;
        }
        Ok(())
    }

    /// Window handling is off: no window records are kept. Every live, untouched window
    /// record goes (tracked or not: also after a restart or without Accessibility);
    /// nothing archived is touched.
    pub(crate) fn forget_windows(&mut self, now: i64) -> Res {
        // Windows Decay hid come back: it stops managing them, so they aren't left in
        // the Dock where nothing would bring them back.
        for (id, t) in &self.app_windows.tracked {
            if t.item.state == ItemState::Hidden
                && t.pid != UNKNOWN_PID
                && let Some(window_id) = t.window_id
            {
                self.out.push(crate::engine::Output::Window(crate::hide::WindowCommand {
                    op: decay_platform::WindowOp::Restore,
                    item_id: id.clone(),
                    pid: t.pid,
                    window_id,
                    deadline: now + crate::hide::HIDE_ANSWER_MS,
                }));
            }
        }
        let ids: Vec<String> = self
            .store
            .live_items()?
            .into_iter()
            .filter(|i| {
                i.kind == ItemKind::Window
                    && matches!(i.state, ItemState::Active | ItemState::Stale | ItemState::Hidden)
            })
            .map(|i| i.id)
            .collect();
        self.store.atomically(false, |s| {
            for id in &ids {
                s.delete_item(id)?;
            }
            Ok(())
        })?;
        self.app_windows = AppWindowsState::default();
        Ok(())
    }

    /// The tick for windows: ages unfocused ones by `credit`, resets the focused window of
    /// the app in front, and marks the old ones stale. Nothing ages without a fresh list.
    pub(crate) fn age_windows(&mut self, credit: u64, now: i64) -> Res {
        let fresh = self.app_windows.last_list_at.is_some_and(|t| now - t < WINDOWS_FRESH_MS);
        if !fresh || self.app_windows.tracked.is_empty() {
            return Ok(());
        }
        let front = self.fresh_platform(now).and_then(|p| p.frontmost.as_ref()).map(|f| f.pid);
        let policy = std::sync::Arc::clone(&self.policy);
        let pro = self.config.pro;
        // On a copy: a failed write leaves memory as the database has it.
        let mut tracked = self.app_windows.tracked.clone();
        let missing = &self.app_windows.missing;
        self.store.atomically(false, |s| {
            age_tracked(s, &mut tracked, missing, front, &policy, pro, credit, now)
        })?;
        self.app_windows.tracked = tracked;
        Ok(())
    }

    /// Live windows tracked now.
    pub fn tracked_windows(&self) -> usize {
        self.app_windows.tracked.len()
    }
}

#[allow(clippy::too_many_arguments, reason = "the tick's inputs")]
fn age_tracked(
    s: &Store,
    tracked: &mut BTreeMap<String, TrackedWindow>,
    missing: &HashMap<String, i64>,
    front: Option<i32>,
    policy: &PolicySettings,
    pro: bool,
    credit: u64,
    now: i64,
) -> Res {
    let ctx = policy.ctx(now, pro);
    let mut ages: Vec<(String, u64)> = Vec::new();
    // A new title exclusion rule applies at once (as for tabs).
    let excluded_ids: Vec<String> = tracked
        .iter()
        .filter(|(_, t)| excluded(&t.item, policy))
        .map(|(id, _)| id.clone())
        .collect();
    for id in &excluded_ids {
        s.delete_item(id)?;
        tracked.remove(id);
    }
    for (id, t) in tracked.iter_mut() {
        if missing.contains_key(id) {
            continue; // probably closed: no aging while that's settled
        }
        let in_use = t.focused && front == Some(t.pid) && !t.minimized;
        let item = &mut t.item;
        if !matches!(item.state, ItemState::Active | ItemState::Stale) {
            continue; // hidden / archiving: P2.2's executor owns it
        }
        let mut age_changed = false;
        if credit > 0 {
            if in_use {
                mark_used(s, item, now, true)?;
            } else {
                item.active_age_s = item.active_age_s.saturating_add(credit);
                age_changed = true;
            }
        }
        let signals = Signals {
            in_use,
            audible: false,
            form_input: FormInput::NotApplicable,
            save_state: item.save_state,
        };
        let decision = evaluate(&target(item), &signals, &ctx);
        let held = matches!(decision, Decision::Protect { by: decay_core::ProtectedBy::Hold, .. });
        let next = match decision {
            Decision::Archive { .. } | Decision::Stale { .. } | Decision::Hide { .. } => {
                ItemState::Stale
            }
            Decision::Keep { .. } | Decision::Protect { .. } => ItemState::Active,
        };
        if !held && next != item.state {
            item.state = next;
            item.state_changed_at = now;
            s.update_item(item)?;
            if next == ItemState::Stale {
                s.append_event(
                    &item.id,
                    EventType::Stale,
                    None,
                    stale_reason(&decision).as_ref(),
                    now,
                )?;
            }
        } else if age_changed {
            ages.push((item.id.clone(), item.active_age_s));
        }
    }
    let refs: Vec<(&str, u64)> = ages.iter().map(|(id, age)| (id.as_str(), *age)).collect();
    s.set_active_ages(&refs)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(title: &str, doc: Option<&str>, edited: Option<bool>) -> AppWindow {
        AppWindow {
            window_id: Some(1),
            title: title.into(),
            document: doc.map(Into::into),
            edited,
            ..AppWindow::default()
        }
    }

    #[test]
    fn save_state_follows_the_close_button_and_the_document() {
        let doc = Some("file:///Users/me/a.txt");
        assert_eq!(save_state_of(&win("a", doc, Some(true))), SaveState::Unsaved);
        assert_eq!(save_state_of(&win("a", doc, Some(false))), SaveState::Saved);
        assert_eq!(save_state_of(&win("a", None, Some(false))), SaveState::Unknown);
        assert_eq!(save_state_of(&win("a", doc, None)), SaveState::Unknown);
        assert_eq!(save_state_of(&win("a", None, Some(true))), SaveState::Unsaved);
    }

    #[test]
    fn document_urls_become_paths() {
        assert_eq!(document_path("file:///Users/me/My%20Report.pdf"), "/Users/me/My Report.pdf");
        assert_eq!(document_path("file://localhost/tmp/x"), "/tmp/x");
        assert_eq!(document_path("file:///Users/me/Caf%C3%A9.txt"), "/Users/me/Café.txt");
        assert_eq!(document_path("file:///bad%zz"), "/bad%zz");
        assert_eq!(document_path("https://example.com/a"), "https://example.com/a");
    }

    #[test]
    fn browsers_are_left_to_their_extension() {
        assert!(is_browser("com.google.Chrome") && is_browser("company.thebrowser.Browser"));
        assert!(!is_browser("com.apple.TextEdit"));
        assert!(is_browser("chrome.exe") && is_browser("msedge.exe"));
        assert!(!is_browser("notepad.exe"));
    }
}
