//! Native notifications (P2.4, PRD F10 / F25, Designs/09f): the morning summary, an
//! optional notice after each sweep, and health alerts (a browser disconnected for a
//! minute, Accessibility turned off while Decay hides windows). Decay stays quiet: every
//! kind can be turned off, nothing pops up during a call or meeting (the morning summary
//! waits for it to end; sweep notices are dropped), and macOS Focus / Do Not Disturb
//! applies as for any app.
//!
//! The decisions are plain functions (tested); [`Notifier`] holds what they need, kept up
//! to date from settings broadcasts and engine events (never by asking the engine from its
//! own thread).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use decay_bridge::protocol::BrowserKind;
use decay_engine::{EngineEvent, EngineStatus};
use serde_json::{Map, Value};
use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::core::Core;
use crate::i18n::{t, tf};

/// A browser disconnected this long is worth a notice (a restart reconnects sooner).
const DISCONNECTED_NOTICE_MS: i64 = 60_000;
/// The parts of one sweep (tabs, then hidden windows) arrive as separate events: they're
/// gathered this long into one notice.
const SWEEP_GATHER_MS: i64 = 10_000;
/// Opening Decay this soon after the morning notice shows the summary.
const SUMMARY_CLICK_MS: i64 = 60 * 60_000;
/// "Only when more than 10 items are archived".
const BIG_SWEEP: u32 = 10;
const CHECK_EVERY: Duration = Duration::from_secs(30);

/// The notification settings (`notifications`, `daily_summary_time`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools, reason = "independent switches")]
pub struct Prefs {
    pub summary: bool,
    /// Local time of the morning summary.
    pub summary_at: (u8, u8),
    pub sweep: bool,
    pub sweep_big_only: bool,
    pub browser: bool,
    pub permission: bool,
    /// Window handling is on (a revoked permission matters only then).
    pub hides_windows: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            summary: true,
            summary_at: (9, 0),
            sweep: false,
            sweep_big_only: false,
            browser: true,
            permission: true,
            hides_windows: false,
        }
    }
}

/// `"HH:MM"` (24 h).
pub fn parse_time(s: &str) -> Option<(u8, u8)> {
    let (h, m) = s.split_once(':')?;
    if h.is_empty() || h.len() > 2 || m.len() != 2 {
        return None;
    }
    let (h, m) = (h.parse::<u8>().ok()?, m.parse::<u8>().ok()?);
    (h < 24 && m < 60).then_some((h, m))
}

impl Prefs {
    /// From the settings snapshot; an older `health` switch covers both health alerts.
    pub fn from_settings(s: &Map<String, Value>) -> Self {
        let d = Self::default();
        let n = s.get("notifications").and_then(Value::as_object);
        let flag = |key: &str, default: bool| {
            n.and_then(|n| n.get(key)).and_then(Value::as_bool).unwrap_or(default)
        };
        let health = flag("health", true);
        Self {
            summary: flag("summary", d.summary),
            summary_at: s
                .get("daily_summary_time")
                .and_then(Value::as_str)
                .and_then(parse_time)
                .unwrap_or(d.summary_at),
            sweep: flag("sweep", d.sweep),
            sweep_big_only: flag("sweepBigOnly", d.sweep_big_only),
            browser: flag("browser", health),
            permission: flag("permission", health),
            hides_windows: s.get("window_management").and_then(Value::as_str) == Some("hide"),
        }
    }
}

/// Whether the morning summary is due: switched on, past its time today (local), and
/// not sent today yet.
pub fn summary_due(
    prefs: &Prefs,
    local: jiff::civil::DateTime,
    sent_on: Option<jiff::civil::Date>,
) -> bool {
    let (h, m) = prefs.summary_at;
    let at = local.date().at(i8::try_from(h).unwrap_or(9), i8::try_from(m).unwrap_or(0), 0, 0);
    prefs.summary && local >= at && sent_on != Some(local.date())
}

/// The notice after a sweep, if one is wanted.
pub fn sweep_notice(prefs: &Prefs, tabs: u32, windows: u32) -> Option<String> {
    let total = tabs + windows;
    if !prefs.sweep || total == 0 || (prefs.sweep_big_only && total <= BIG_SWEEP) {
        return None;
    }
    let key = match (tabs, windows) {
        (_, 0) => "notify.sweepTabs",
        (0, _) => "notify.sweepWindows",
        _ => "notify.sweepBoth",
    };
    Some(tf(key, &[("tabs", tabs.into()), ("windows", windows.into())]))
}

fn browser_name(b: BrowserKind) -> String {
    match b {
        BrowserKind::Chrome => "Chrome".into(),
        BrowserKind::Edge => "Edge".into(),
        BrowserKind::Brave => "Brave".into(),
        BrowserKind::Arc => "Arc".into(),
        BrowserKind::Chromium | BrowserKind::Other => t("notify.yourBrowser"),
    }
}

#[derive(Debug, Default)]
struct State {
    prefs: Prefs,
    /// Settings have been read at least once (nothing is sent before).
    loaded: bool,
    status: Option<EngineStatus>,
    /// Disconnected browsers (by browser: two profiles are one notice): since when, and
    /// whether the notice went out.
    disconnected: HashMap<BrowserKind, (i64, bool)>,
    /// A sweep's tabs and windows, gathered into one notice: (tabs, windows, first at).
    sweep: Option<(u32, u32, i64)>,
    /// When the morning notice was shown (opening Decay soon after shows the summary).
    summary_shown_at: Option<i64>,
    /// Accessibility was granted at some point this run (a "revoked" notice needs that).
    accessibility_seen: bool,
    summary_sent_on: Option<jiff::civil::Date>,
}

/// What notifications need, kept current from settings and engine events.
#[derive(Debug, Default)]
pub struct Notifier {
    state: Mutex<State>,
}

impl Notifier {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn set_settings(&self, settings: &Map<String, Value>) {
        let mut s = self.lock();
        s.prefs = Prefs::from_settings(settings);
        s.loaded = true;
        s.summary_sent_on = settings
            .get("daily_summary_sent_on")
            .and_then(Value::as_str)
            .and_then(|d| d.parse().ok())
            .or(s.summary_sent_on);
    }

    fn in_meeting(&self) -> bool {
        self.lock().status.as_ref().is_some_and(|s| s.in_meeting)
    }

    /// An engine event (on the engine thread: only notes things, and shows the one
    /// immediate notice, a revoked permission).
    pub fn on_event(&self, app: &AppHandle, event: &EngineEvent, now: i64) {
        if let Some(body) = self.note(event, now) {
            show(app, &t("notify.title"), &body);
        }
    }

    /// Notes an event; returns a notice to show now, if any.
    fn note(&self, event: &EngineEvent, now: i64) -> Option<String> {
        let mut s = self.lock();
        let quiet = s.status.as_ref().is_some_and(|st| st.in_meeting);
        match event {
            EngineEvent::StatusChanged(status) => s.status = Some((**status).clone()),
            EngineEvent::SweepCompleted { shadow: false, archived, .. } if !quiet => {
                let (t, w, at) = s.sweep.unwrap_or((0, 0, now));
                s.sweep = Some((t + archived, w, at));
            }
            EngineEvent::WindowsHidden { hidden, .. } if !quiet => {
                let (t, w, at) = s.sweep.unwrap_or((0, 0, now));
                s.sweep = Some((t, w + hidden, at));
            }
            EngineEvent::AdapterDisconnected { browser, .. } => {
                s.disconnected.entry(*browser).or_insert((now, false));
            }
            EngineEvent::AdapterConnected { browser, .. } => {
                s.disconnected.remove(browser);
            }
            EngineEvent::AccessibilityChanged { trusted: true } => s.accessibility_seen = true,
            EngineEvent::AccessibilityChanged { trusted: false } => {
                let tell = s.accessibility_seen && s.prefs.permission && s.prefs.hides_windows;
                s.accessibility_seen = false;
                if tell && !quiet {
                    return Some(t("notify.axOff"));
                }
            }
            _ => {}
        }
        None
    }

    /// The gathered sweep notice, once its parts have had time to arrive.
    fn due_sweep_notice(&self, now: i64) -> Option<String> {
        let mut s = self.lock();
        let (tabs, windows, at) = s.sweep?;
        if now - at < SWEEP_GATHER_MS {
            return None;
        }
        s.sweep = None;
        if s.status.as_ref().is_some_and(|st| st.in_meeting) {
            return None;
        }
        sweep_notice(&s.prefs, tabs, windows)
    }

    /// Browsers disconnected for a minute while still running (quitting a browser isn't
    /// news): one notice each, until they reconnect. `running` says whether a browser
    /// runs (`None`: can't tell, then nothing is said).
    fn due_disconnects(
        &self,
        now: i64,
        running: impl Fn(BrowserKind) -> Option<bool>,
    ) -> Vec<String> {
        let mut s = self.lock();
        if !s.prefs.browser || s.status.as_ref().is_some_and(|st| st.in_meeting) {
            return Vec::new();
        }
        let mut out = Vec::new();
        for (browser, (since, told)) in &mut s.disconnected {
            if !*told && now - *since >= DISCONNECTED_NOTICE_MS {
                *told = true; // decided once: told, or quit (not told)
                if running(*browser) == Some(true) {
                    out.push(tf(
                        "notify.disconnected",
                        &[("browser", browser_name(*browser).into())],
                    ));
                }
            }
        }
        out
    }

    /// Whether opening Decay now should show the summary (the morning notice was clicked,
    /// most likely). Asked once.
    pub fn take_summary_click(&self, now: i64) -> bool {
        let mut s = self.lock();
        let recent = s.summary_shown_at.is_some_and(|t| now - t < SUMMARY_CLICK_MS);
        s.summary_shown_at = None;
        recent
    }
}

fn show(app: &AppHandle, title: &str, body: &str) {
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        tracing::warn!("notification not shown: {e}");
    }
}

/// The background check (every 30 s): the morning summary when due, disconnect notices.
pub fn start(app: &AppHandle) {
    let handle = app.clone();
    let spawned = std::thread::Builder::new().name("decay-notify".into()).spawn(move || {
        loop {
            std::thread::sleep(CHECK_EVERY);
            let now = jiff::Timestamp::now();
            let notifier = handle.state::<Notifier>();
            let ms = now.as_millisecond();
            let running = |b| decay_platform::app_running(decay_engine::browser_bundle_id(b));
            for body in notifier.due_disconnects(ms, running) {
                show(&handle, &t("notify.title"), &body);
            }
            if let Some(body) = notifier.due_sweep_notice(ms) {
                show(&handle, &t("notify.title"), &body);
            }
            morning_summary(&handle, now);
        }
    });
    if let Err(e) = spawned {
        tracing::warn!("notifications not started: {e}");
    }
}

fn morning_summary(app: &AppHandle, now: jiff::Timestamp) {
    let notifier = app.state::<Notifier>();
    let tz = jiff::tz::TimeZone::system();
    let local = now.to_zoned(tz.clone()).datetime();
    let (prefs, sent_on, loaded) = {
        let s = notifier.lock();
        (s.prefs.clone(), s.summary_sent_on, s.loaded)
    };
    if !loaded || !summary_due(&prefs, local, sent_on) || notifier.in_meeting() {
        return; // a meeting: it waits until the meeting ends
    }
    let Some((from, to)) = crate::summary::yesterday(now.as_millisecond(), &tz) else { return };
    let today = local.date();
    let summary = app.state::<Core>().ask(move |e, at| {
        // Marked sent only once it was really made and saved (else the next check tries
        // again); whether or not anything happened (one look a day).
        let made = crate::summary::summary_for(e.store(), from, to, at).ok().filter(|_| {
            e.store().set_setting("daily_summary_sent_on", &today.to_string(), at).is_ok()
        });
        (made, Vec::new())
    });
    let Ok(Some(summary)) = summary else { return };
    notifier.lock().summary_sent_on = Some(today);
    if !summary.is_empty() {
        show(app, &t("notify.morning"), &summary.sentence());
        notifier.lock().summary_shown_at = Some(now.as_millisecond());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn times_parse_strictly() {
        assert_eq!(parse_time("09:00"), Some((9, 0)));
        assert_eq!(parse_time("7:30"), Some((7, 30)));
        assert_eq!(parse_time("23:59"), Some((23, 59)));
        for bad in ["24:00", "9:60", "9", "", "09:0", "aa:bb", "-1:00", "123:00"] {
            assert_eq!(parse_time(bad), None, "{bad}");
        }
    }

    #[test]
    fn prefs_come_from_settings_with_the_old_health_switch() {
        let s = json!({"notifications": {"summary": false, "sweep": true, "health": false}, "daily_summary_time": "08:15"});
        let p = Prefs::from_settings(s.as_object().unwrap());
        assert_eq!(
            p,
            Prefs {
                summary: false,
                summary_at: (8, 15),
                sweep: true,
                sweep_big_only: false,
                browser: false,
                permission: false,
                hides_windows: false
            }
        );
        let s = json!({"notifications": {"browser": true, "permission": false, "sweepBigOnly": true}, "daily_summary_time": "nonsense"});
        let p = Prefs::from_settings(s.as_object().unwrap());
        assert_eq!(
            (p.summary, p.summary_at, p.browser, p.permission, p.sweep_big_only),
            (true, (9, 0), true, false, true)
        );
    }

    #[test]
    fn the_summary_is_due_once_a_day_after_its_time() {
        let p = Prefs::default();
        let day = jiff::civil::date(2026, 11, 14);
        assert!(!summary_due(&p, day.at(8, 59, 0, 0), None));
        assert!(summary_due(&p, day.at(9, 0, 0, 0), None));
        assert!(summary_due(&p, day.at(17, 0, 0, 0), Some(day.yesterday().unwrap())));
        assert!(!summary_due(&p, day.at(17, 0, 0, 0), Some(day)));
        assert!(!summary_due(&Prefs { summary: false, ..p }, day.at(10, 0, 0, 0), None));
    }

    #[test]
    fn sweep_notices_follow_the_switches() {
        let off = Prefs::default();
        assert_eq!(sweep_notice(&off, 5, 0), None, "off by default");
        let on = Prefs { sweep: true, ..off };
        assert_eq!(
            sweep_notice(&on, 1, 0).as_deref(),
            Some("Archived 1 tab. Undo from the menu bar.")
        );
        assert_eq!(
            sweep_notice(&on, 0, 2).as_deref(),
            Some("Hid 2 windows in the Dock. Undo from the menu bar.")
        );
        assert_eq!(sweep_notice(&on, 0, 0), None);
        let big = Prefs { sweep_big_only: true, ..on };
        assert_eq!(sweep_notice(&big, 10, 0), None);
        assert!(sweep_notice(&big, 11, 0).is_some());
    }

    fn status(in_meeting: bool) -> EngineStatus {
        EngineStatus {
            mode: decay_core::Mode::Automatic,
            hold_until: None,
            stale_count: 0,
            live_tabs: 0,
            browsers: Vec::new(),
            sweeping: false,
            undoable_sweep: None,
            accessibility: None,
            in_meeting,
        }
    }

    fn disconnected(n: &Notifier, browser: BrowserKind, profile: &str, at: i64) {
        let e = EngineEvent::AdapterDisconnected { browser, profile_id: profile.into() };
        assert_eq!(n.note(&e, at), None);
    }

    #[test]
    fn a_disconnect_is_told_once_only_while_the_browser_still_runs() {
        let n = Notifier::default();
        let running = |_| Some(true);
        // Two profiles of Chrome: one notice.
        disconnected(&n, BrowserKind::Chrome, "p", 0);
        disconnected(&n, BrowserKind::Chrome, "q", 5_000);
        assert_eq!(n.due_disconnects(30_000, running), Vec::<String>::new());
        assert_eq!(n.due_disconnects(61_000, running).len(), 1);
        assert_eq!(n.due_disconnects(120_000, running), Vec::<String>::new(), "once");
        // Arc was quit (not running, or can't tell): nothing.
        disconnected(&n, BrowserKind::Arc, "a", 0);
        assert_eq!(n.due_disconnects(200_000, |_| Some(false)), Vec::<String>::new());
        disconnected(&n, BrowserKind::Edge, "e", 0);
        assert_eq!(n.due_disconnects(200_000, |_| None), Vec::<String>::new());
        // Reconnecting clears it; a meeting or the switch keeps quiet.
        let back =
            EngineEvent::AdapterConnected { browser: BrowserKind::Brave, profile_id: "b".into() };
        disconnected(&n, BrowserKind::Brave, "b", 0);
        n.note(&back, 1_000);
        assert_eq!(n.due_disconnects(300_000, running), Vec::<String>::new(), "reconnected");
        disconnected(&n, BrowserKind::Brave, "b", 400_000);
        n.note(&EngineEvent::StatusChanged(Box::new(status(true))), 400_000);
        assert_eq!(n.due_disconnects(500_000, running), Vec::<String>::new(), "in a meeting");
        n.note(&EngineEvent::StatusChanged(Box::new(status(false))), 500_000);
        n.lock().prefs.browser = false;
        assert_eq!(n.due_disconnects(600_000, running), Vec::<String>::new(), "switched off");
    }

    #[test]
    fn a_sweeps_tabs_and_windows_make_one_notice() {
        let n = Notifier::default();
        n.lock().prefs = Prefs { sweep: true, sweep_big_only: true, ..Prefs::default() };
        let tabs = EngineEvent::SweepCompleted {
            sweep_id: "s".into(),
            shadow: false,
            archived: 6,
            failed: 0,
        };
        let windows = EngineEvent::WindowsHidden { sweep_id: "h".into(), hidden: 5, failed: 0 };
        n.note(&tabs, 0);
        n.note(&windows, 1_000);
        assert_eq!(n.due_sweep_notice(5_000), None, "still gathering");
        assert_eq!(
            n.due_sweep_notice(11_000).as_deref(),
            Some("Archived 6 tabs and hid 5 windows. Undo from the menu bar."),
            "11 in all: over 10"
        );
        assert_eq!(n.due_sweep_notice(30_000), None, "once");
        // Shadow sweeps say nothing.
        let shadow = EngineEvent::SweepCompleted {
            sweep_id: "x".into(),
            shadow: true,
            archived: 40,
            failed: 0,
        };
        n.note(&shadow, 40_000);
        assert_eq!(n.due_sweep_notice(60_000), None);
    }

    #[test]
    fn a_revoked_permission_is_told_only_if_it_was_granted_and_windows_are_hidden() {
        let n = Notifier::default();
        let off = EngineEvent::AccessibilityChanged { trusted: false };
        let on = EngineEvent::AccessibilityChanged { trusted: true };
        assert_eq!(n.note(&off, 0), None, "never granted (first sample)");
        n.note(&on, 1);
        assert_eq!(n.note(&off, 2), None, "window handling off");
        n.lock().prefs.hides_windows = true;
        n.note(&on, 3);
        assert!(n.note(&off, 4).is_some());
        n.note(&on, 5);
        n.lock().prefs.permission = false;
        assert_eq!(n.note(&off, 6), None, "switched off");
    }

    #[test]
    fn the_summary_click_is_taken_once_and_only_soon_after() {
        let n = Notifier::default();
        assert!(!n.take_summary_click(0));
        n.lock().summary_shown_at = Some(0);
        assert!(n.take_summary_click(60_000));
        assert!(!n.take_summary_click(61_000), "once");
        n.lock().summary_shown_at = Some(0);
        assert!(!n.take_summary_click(2 * 3_600_000), "too long ago");
    }

    #[test]
    fn settings_keep_the_sent_day_and_mark_them_loaded() {
        let n = Notifier::default();
        assert!(!n.lock().loaded);
        let with = json!({"daily_summary_sent_on": "2026-11-13", "window_management": "hide"});
        n.set_settings(with.as_object().unwrap());
        n.set_settings(json!({}).as_object().unwrap());
        let s = n.lock();
        assert!(s.loaded);
        assert_eq!(s.summary_sent_on, Some(jiff::civil::date(2026, 11, 13)));
    }
}
