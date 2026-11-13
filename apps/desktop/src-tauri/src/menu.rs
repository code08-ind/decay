//! What the menu bar item shows (F16): the stale-count badge, the tooltip and the menu.
//! A pure function of the engine's status, so it's tested without a tray.
//!
//! An item that wouldn't do anything is disabled rather than hidden (the menu keeps its
//! shape): "Run sweep now" without a connected browser, "Undo last sweep" with nothing
//! to undo, and so on.

use decay_core::Mode;
use decay_engine::EngineStatus;

/// Menu item ids (also the actions).
pub mod id {
    pub const STATUS: &str = "status";
    pub const STATUS_DETAIL: &str = "status-detail";
    pub const SEARCH: &str = "search";
    pub const HOLD: &str = "hold";
    pub const RELEASE_HOLD: &str = "release-hold";
    pub const PAUSE: &str = "pause";
    pub const RESUME: &str = "resume";
    pub const SWEEP_NOW: &str = "sweep-now";
    pub const UNDO: &str = "undo";
    pub const LIBRARY: &str = "library";
    pub const SETTINGS: &str = "settings";
    pub const QUIT: &str = "quit";
}

/// A hold this far ahead is "indefinite" (API Design `hold { duration: 'indefinite' }`).
pub const HOLD_INDEFINITE_MS: i64 = 253_402_300_799_000; // 9999-12-31

/// One line of the menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    Item { id: &'static str, label: String, enabled: bool, accelerator: Option<String> },
    Separator,
}

fn item(id: &'static str, label: impl Into<String>, enabled: bool) -> Entry {
    Entry::Item { id, label: label.into(), enabled, accelerator: None }
}

fn plural(n: u32, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The local UTC offset in minutes at a moment (it changes across daylight saving).
pub type OffsetAt<'a> = &'a dyn Fn(i64) -> i32;

/// `HH:MM` in local time at `at_ms` for "On hold until …".
fn clock(at_ms: i64, offset_at: OffsetAt<'_>) -> String {
    let mins = (at_ms.div_euclid(60_000) + i64::from(offset_at(at_ms))).rem_euclid(24 * 60);
    format!("{:02}:{:02}", mins / 60, mins % 60)
}

/// The first line: what Decay is doing.
fn headline(s: &EngineStatus, offset_at: OffsetAt<'_>) -> String {
    if s.mode == Mode::Paused {
        return "Paused".into();
    }
    if let Some(until) = s.hold_until {
        if until >= HOLD_INDEFINITE_MS {
            return "On hold".into();
        }
        return format!("On hold until {}", clock(until, offset_at));
    }
    if s.browsers.is_empty() {
        return "No browser connected".into();
    }
    if s.sweeping {
        return "Sweeping…".into();
    }
    let mode = if s.mode == Mode::Shadow { " · shadow mode" } else { "" };
    format!("{} open, {} stale{mode}", plural(s.live_tabs, "tab", "tabs"), s.stale_count)
}

/// The menu. `status` is `None` until the engine reported (start-up); `hotkey` is shown
/// next to Search.
pub fn entries(
    status: Option<&EngineStatus>,
    hotkey: Option<&str>,
    offset_at: OffsetAt<'_>,
) -> Vec<Entry> {
    let Some(s) = status else {
        return vec![
            item(id::STATUS, "Starting…", false),
            Entry::Separator,
            item(id::QUIT, "Quit Decay", true),
        ];
    };
    let paused = s.mode == Mode::Paused;
    let held = s.hold_until.is_some();
    let connected = !s.browsers.is_empty();
    let mut out = vec![
        item(id::STATUS, headline(s, offset_at), false),
        Entry::Separator,
        Entry::Item {
            id: id::SEARCH,
            label: "Search archive…".into(),
            enabled: true,
            accelerator: hotkey.map(str::to_owned),
        },
        Entry::Separator,
    ];
    out.push(if held {
        item(id::RELEASE_HOLD, "Release hold", true)
    } else {
        item(id::HOLD, "Hold for 1 hour", !paused)
    });
    out.push(if paused {
        item(id::RESUME, "Resume", true)
    } else {
        item(id::PAUSE, "Pause", true)
    });
    let sweep_label =
        if s.mode == Mode::Shadow { "Run sweep now (shadow)" } else { "Run sweep now" };
    out.push(item(id::SWEEP_NOW, sweep_label, connected && !paused && !held && !s.sweeping));
    out.push(item(id::UNDO, "Undo last sweep", s.undoable_sweep.is_some() && connected));
    out.extend([
        Entry::Separator,
        item(id::LIBRARY, "Open Library", true),
        item(id::SETTINGS, "Settings…", true),
        Entry::Separator,
        item(id::QUIT, "Quit Decay", true),
    ]);
    out
}

/// The menu when the engine couldn't start (`why`: shown, so the user can act on it).
pub fn failed_entries(why: &str) -> Vec<Entry> {
    let short: String = why.chars().take(80).collect();
    vec![
        item(id::STATUS, "Decay couldn't start", false),
        item(id::STATUS_DETAIL, short, false),
        Entry::Separator,
        item(id::QUIT, "Quit Decay", true),
    ]
}

/// Two menus differ only in the first line's text: the status item can be relabelled
/// instead of rebuilding the menu (tab counts change often).
pub fn same_shape(a: &[Entry], b: &[Entry]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).enumerate().all(|(n, pair)| match pair {
            (
                Entry::Item { id: ia, label: la, enabled: ea, accelerator: aa },
                Entry::Item { id: ib, label: lb, enabled: eb, accelerator: ab },
            ) => ia == ib && ea == eb && aa == ab && (n == 0 || la == lb),
            (Entry::Separator, Entry::Separator) => true,
            _ => false,
        })
}

/// The text beside the menu bar icon: the stale count, if any.
pub fn badge(status: Option<&EngineStatus>) -> Option<String> {
    status
        .map(|s| s.stale_count)
        .filter(|n| *n > 0)
        .map(|n| if n > 99 { "99+".into() } else { n.to_string() })
}

/// The icon's tooltip.
pub fn tooltip(status: Option<&EngineStatus>, offset_at: OffsetAt<'_>) -> String {
    match status {
        Some(s) => format!("Decay — {}", headline(s, offset_at)),
        None => "Decay".into(),
    }
}

#[cfg(test)]
mod tests {
    use decay_bridge::protocol::BrowserKind;
    use decay_engine::BrowserStatus;

    use super::*;

    fn status() -> EngineStatus {
        EngineStatus {
            mode: Mode::Automatic,
            hold_until: None,
            stale_count: 3,
            live_tabs: 12,
            browsers: vec![BrowserStatus {
                browser: BrowserKind::Chrome,
                profile_id: "p".into(),
                tabs: 12,
            }],
            sweeping: false,
            undoable_sweep: Some("w1".into()),
            accessibility: Some(true),
        }
    }

    fn enabled(entries: &[Entry], want: &str) -> bool {
        entries
            .iter()
            .find_map(|e| match e {
                Entry::Item { id, enabled, .. } if *id == want => Some(*enabled),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no {want}"))
    }

    fn label(entries: &[Entry], want: &str) -> String {
        entries
            .iter()
            .find_map(|e| match e {
                Entry::Item { id, label, .. } if *id == want => Some(label.clone()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no {want}"))
    }

    #[test]
    fn the_full_menu_while_running() {
        let e = entries(Some(&status()), Some("Alt+Super+D"), &|_| 0);
        let ids: Vec<&str> = e
            .iter()
            .filter_map(|e| match e {
                Entry::Item { id, .. } => Some(*id),
                Entry::Separator => None,
            })
            .collect();
        assert_eq!(
            ids,
            [
                id::STATUS,
                id::SEARCH,
                id::HOLD,
                id::PAUSE,
                id::SWEEP_NOW,
                id::UNDO,
                id::LIBRARY,
                id::SETTINGS,
                id::QUIT
            ]
        );
        assert_eq!(label(&e, id::STATUS), "12 tabs open, 3 stale");
        assert!(!enabled(&e, id::STATUS));
        assert!(enabled(&e, id::SWEEP_NOW) && enabled(&e, id::UNDO) && enabled(&e, id::HOLD));
        assert!(e.contains(&Entry::Item {
            id: id::SEARCH,
            label: "Search archive…".into(),
            enabled: true,
            accelerator: Some("Alt+Super+D".into())
        }));
    }

    #[test]
    fn paused_held_and_disconnected_menus() {
        let paused = EngineStatus { mode: Mode::Paused, ..status() };
        let e = entries(Some(&paused), None, &|_| 0);
        assert_eq!(label(&e, id::STATUS), "Paused");
        assert!(enabled(&e, id::RESUME));
        assert!(!enabled(&e, id::HOLD) && !enabled(&e, id::SWEEP_NOW));

        // 13:30 UTC, shown at UTC+2.
        let held = EngineStatus { hold_until: Some(1_793_799_000_000), ..status() };
        let e = entries(Some(&held), None, &|_| 120);
        assert_eq!(label(&e, id::STATUS), "On hold until 15:30");
        assert!(enabled(&e, id::RELEASE_HOLD) && !enabled(&e, id::SWEEP_NOW));

        let alone = EngineStatus { browsers: Vec::new(), ..status() };
        let e = entries(Some(&alone), None, &|_| 0);
        assert_eq!(label(&e, id::STATUS), "No browser connected");
        assert!(!enabled(&e, id::SWEEP_NOW) && !enabled(&e, id::UNDO));

        let shadow =
            EngineStatus { mode: Mode::Shadow, undoable_sweep: None, live_tabs: 1, ..status() };
        let e = entries(Some(&shadow), None, &|_| 0);
        assert_eq!(label(&e, id::STATUS), "1 tab open, 3 stale · shadow mode");
        assert_eq!(label(&e, id::SWEEP_NOW), "Run sweep now (shadow)");
        assert!(!enabled(&e, id::UNDO));

        let busy = EngineStatus { sweeping: true, ..status() };
        let e = entries(Some(&busy), None, &|_| 0);
        assert_eq!(label(&e, id::STATUS), "Sweeping…");
        assert!(!enabled(&e, id::SWEEP_NOW));
    }

    #[test]
    fn before_the_engine_reports() {
        let e = entries(None, None, &|_| 0);
        assert_eq!(label(&e, id::STATUS), "Starting…");
        assert!(enabled(&e, id::QUIT));
        assert_eq!(badge(None), None);
        assert_eq!(tooltip(None, &|_| 0), "Decay");
    }

    #[test]
    fn the_badge_counts_stale_tabs() {
        assert_eq!(badge(Some(&status())), Some("3".into()));
        assert_eq!(badge(Some(&EngineStatus { stale_count: 0, ..status() })), None);
        assert_eq!(badge(Some(&EngineStatus { stale_count: 250, ..status() })), Some("99+".into()));
        assert_eq!(tooltip(Some(&status()), &|_| 0), "Decay — 12 tabs open, 3 stale");
    }

    #[test]
    fn an_indefinite_hold_and_a_failed_start() {
        let held = EngineStatus { hold_until: Some(HOLD_INDEFINITE_MS), ..status() };
        assert_eq!(label(&entries(Some(&held), None, &|_| 0), id::STATUS), "On hold");
        let e = failed_entries(&"x".repeat(200));
        assert_eq!(label(&e, id::STATUS), "Decay couldn't start");
        assert!(e.iter().all(|e| match e {
            Entry::Item { label, .. } => label.chars().count() <= 80,
            Entry::Separator => true,
        }));
        assert!(enabled(&e, id::QUIT));
    }

    #[test]
    fn the_hold_end_uses_the_offset_at_that_moment() {
        // Summer time ends between now and the end of the hold: the end is shown in winter time.
        let until = 1_793_000_000_000;
        let offset_at = |ms: i64| if ms >= until { 60 } else { 120 };
        let held = EngineStatus { hold_until: Some(until), ..status() };
        let e = entries(Some(&held), None, &offset_at);
        assert_eq!(label(&e, id::STATUS), format!("On hold until {}", clock(until, &|_| 60)));
    }

    #[test]
    fn a_new_tab_count_only_relabels_the_status_line() {
        let a = entries(Some(&status()), None, &|_| 0);
        let more_tabs = entries(Some(&EngineStatus { live_tabs: 13, ..status() }), None, &|_| 0);
        assert!(same_shape(&a, &more_tabs));
        let paused = entries(Some(&EngineStatus { mode: Mode::Paused, ..status() }), None, &|_| 0);
        assert!(!same_shape(&a, &paused));
        let no_undo =
            entries(Some(&EngineStatus { undoable_sweep: None, ..status() }), None, &|_| 0);
        assert!(!same_shape(&a, &no_undo));
        assert!(!same_shape(&a, &entries(None, None, &|_| 0)));
    }

    #[test]
    fn clock_wraps_around_midnight() {
        assert_eq!(clock(0, &|_| -60), "23:00");
        assert_eq!(clock(23 * 3_600_000 + 59 * 60_000, &|_| 0), "23:59");
    }
}
