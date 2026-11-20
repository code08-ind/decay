//! The decisions behind the Windows layer (P3.1), kept apart from the OS calls so they're
//! tested on every platform: which top-level windows are app windows, what counts as full
//! screen, how a process is named, whether the session is locked, how long since input.

/// `WS_VISIBLE`.
pub const WS_VISIBLE: u32 = 0x1000_0000;
/// `WS_CHILD`.
pub const WS_CHILD: u32 = 0x4000_0000;
/// `WS_MINIMIZEBOX`.
pub const WS_MINIMIZEBOX: u32 = 0x0002_0000;
/// `WS_DISABLED`.
pub const WS_DISABLED: u32 = 0x0800_0000;
/// `WS_EX_TOOLWINDOW`: palettes and floating toolbars (not in the taskbar).
pub const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
/// `WS_EX_NOACTIVATE`: overlays that never take focus.
pub const WS_EX_NOACTIVATE: u32 = 0x0800_0000;
/// `WS_EX_APPWINDOW`: shown in the taskbar even when owned.
pub const WS_EX_APPWINDOW: u32 = 0x0004_0000;

/// What the OS says about one top-level window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(clippy::struct_excessive_bools, reason = "independent facts the OS reports")]
pub struct TopLevel {
    pub style: u32,
    pub ex_style: u32,
    /// It has an owner window (dialogs, property sheets).
    pub owned: bool,
    /// DWM hides it (`DWMWA_CLOAKED`): a UWP window on another virtual desktop or a
    /// suspended app's frame. Never shown, never touched.
    pub cloaked: bool,
    pub has_title: bool,
}

/// A window the user would call an app window (what Alt+Tab and the taskbar show): visible,
/// not a child or tool window, not an owned dialog (unless it asks for the taskbar), not
/// cloaked, not an overlay, with a title. The equivalent of macOS's `AXStandardWindow`.
pub fn is_app_window(w: &TopLevel) -> bool {
    let shown = w.style & WS_VISIBLE != 0 && w.style & WS_CHILD == 0 && !w.cloaked;
    let tool = w.ex_style & WS_EX_TOOLWINDOW != 0 || w.ex_style & WS_EX_NOACTIVATE != 0;
    let taskbar = !w.owned || w.ex_style & WS_EX_APPWINDOW != 0;
    shown && !tool && taskbar && w.has_title
}

/// It has an enabled minimise button.
pub fn can_minimize(style: u32) -> bool {
    style & WS_MINIMIZEBOX != 0 && style & WS_DISABLED == 0
}

/// A rectangle (`left, top, right, bottom`, in screen pixels).
pub type Rect = (i32, i32, i32, i32);

/// The window covers its whole monitor (a full-screen game, video or presentation): the
/// taskbar is hidden and minimising it would be a surprise, so it's never hidden.
pub fn covers_monitor(window: Rect, monitor: Rect) -> bool {
    window.0 <= monitor.0 && window.1 <= monitor.1 && window.2 >= monitor.2 && window.3 >= monitor.3
}

/// The app id and name of an executable: `C:\…\chrome.exe` → (`chrome.exe`, `chrome`).
/// The id is the lower-cased file name (what the engine's browser ids use); `None` for an
/// empty path.
pub fn exe_app(path: &str) -> Option<(String, String)> {
    let file = path.rsplit(['\\', '/']).next().filter(|f| !f.is_empty())?;
    let name = match file.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && ext.eq_ignore_ascii_case("exe") => stem,
        _ => file,
    };
    Some((file.to_lowercase(), name.to_owned()))
}

/// `WTS_SESSIONSTATE_LOCK` in `WTSINFOEX_LEVEL1_W.SessionFlags` (Windows 8+; Windows 7
/// had the two values swapped, and isn't supported).
pub const WTS_SESSIONSTATE_LOCK: i32 = 0;
/// `WTS_SESSIONSTATE_UNLOCK`.
pub const WTS_SESSIONSTATE_UNLOCK: i32 = 1;

/// Locked: the session's flags say so, or another session owns the console (fast user
/// switching). `None` when neither can be read.
pub fn session_locked(
    flags: Option<i32>,
    console_session: Option<u32>,
    own_session: Option<u32>,
) -> Option<bool> {
    let elsewhere = match (console_session, own_session) {
        // 0xFFFFFFFF: no session is attached to the console (switching right now).
        (Some(u32::MAX), Some(_)) => Some(true),
        (Some(console), Some(own)) => Some(console != own),
        _ => None,
    };
    let lock = match flags {
        Some(WTS_SESSIONSTATE_LOCK) => Some(true),
        Some(WTS_SESSIONSTATE_UNLOCK) => Some(false),
        _ => None,
    };
    match (lock, elsewhere) {
        (Some(true), _) | (_, Some(true)) => Some(true),
        (Some(false), _) | (_, Some(false)) => Some(false),
        (None, None) => None,
    }
}

/// Seconds since the last input: `GetLastInputInfo` stamps it with the 32-bit tick count,
/// which wraps every 49.7 days, so the difference is taken in 32 bits.
pub fn idle_secs(now_ms: u64, last_input_ms: u32) -> u64 {
    #[allow(clippy::cast_possible_truncation, reason = "the stamp is the low 32 bits")]
    let now = now_ms as u32;
    u64::from(now.wrapping_sub(last_input_ms)) / 1000
}

/// `GUID_CONSOLE_DISPLAY_STATE` data: 0 off, 1 on, 2 dimmed. Dimmed isn't asleep.
pub fn display_asleep(state: u32) -> Option<bool> {
    match state {
        0 => Some(true),
        1 | 2 => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const APP: TopLevel = TopLevel {
        style: WS_VISIBLE | WS_MINIMIZEBOX,
        ex_style: 0,
        owned: false,
        cloaked: false,
        has_title: true,
    };

    #[test]
    fn app_windows_are_what_alt_tab_shows() {
        assert!(is_app_window(&APP));
        assert!(!is_app_window(&TopLevel { style: 0, ..APP }), "hidden");
        assert!(!is_app_window(&TopLevel { style: WS_VISIBLE | WS_CHILD, ..APP }));
        assert!(!is_app_window(&TopLevel { ex_style: WS_EX_TOOLWINDOW, ..APP }));
        assert!(!is_app_window(&TopLevel { ex_style: WS_EX_NOACTIVATE, ..APP }));
        assert!(!is_app_window(&TopLevel { cloaked: true, ..APP }));
        assert!(!is_app_window(&TopLevel { has_title: false, ..APP }));
        assert!(!is_app_window(&TopLevel { owned: true, ..APP }), "a dialog");
        assert!(is_app_window(&TopLevel { owned: true, ex_style: WS_EX_APPWINDOW, ..APP }));
    }

    #[test]
    fn minimise_buttons_and_full_screen() {
        assert!(can_minimize(WS_VISIBLE | WS_MINIMIZEBOX));
        assert!(!can_minimize(WS_VISIBLE));
        assert!(!can_minimize(WS_MINIMIZEBOX | WS_DISABLED));
        let monitor = (0, 0, 1920, 1080);
        assert!(covers_monitor((0, 0, 1920, 1080), monitor));
        assert!(covers_monitor((-8, -8, 1928, 1088), monitor), "maximised borders hang over");
        assert!(!covers_monitor((0, 0, 1920, 1040), monitor), "the taskbar shows");
        assert!(!covers_monitor((1920, 0, 3840, 1080), monitor), "another monitor");
    }

    #[test]
    fn executables_name_their_app() {
        let e = |p: &str| exe_app(p);
        assert_eq!(
            e(r"C:\Program Files\Google\Chrome\Application\chrome.exe"),
            Some(("chrome.exe".into(), "chrome".into()))
        );
        assert_eq!(e(r"C:\Windows\explorer.EXE"), Some(("explorer.exe".into(), "explorer".into())));
        assert_eq!(e("/odd/Tool"), Some(("tool".into(), "Tool".into())));
        assert_eq!(e(".exe"), Some((".exe".into(), ".exe".into())));
        assert_eq!(e(r"C:\dir\"), None);
        assert_eq!(e(""), None);
    }

    #[test]
    fn locked_sessions() {
        let own = Some(2);
        assert_eq!(session_locked(Some(WTS_SESSIONSTATE_LOCK), Some(2), own), Some(true));
        assert_eq!(session_locked(Some(WTS_SESSIONSTATE_UNLOCK), Some(2), own), Some(false));
        assert_eq!(
            session_locked(Some(WTS_SESSIONSTATE_UNLOCK), Some(3), own),
            Some(true),
            "switched user"
        );
        assert_eq!(session_locked(None, Some(u32::MAX), own), Some(true));
        assert_eq!(session_locked(None, Some(2), own), Some(false));
        assert_eq!(session_locked(Some(-1), None, None), None);
        assert_eq!(session_locked(Some(WTS_SESSIONSTATE_UNLOCK), None, None), Some(false));
    }

    #[test]
    fn idle_time_survives_the_tick_count_wrapping() {
        assert_eq!(idle_secs(10_000, 4_000), 6);
        assert_eq!(idle_secs(u64::from(u32::MAX) + 1 + 2_000, u32::MAX - 999), 3);
        assert_eq!(idle_secs(5_000, 5_000), 0);
    }

    #[test]
    fn display_states() {
        assert_eq!(display_asleep(0), Some(true));
        assert_eq!(display_asleep(1), Some(false));
        assert_eq!(display_asleep(2), Some(false), "dimmed");
        assert_eq!(display_asleep(9), None);
    }
}
