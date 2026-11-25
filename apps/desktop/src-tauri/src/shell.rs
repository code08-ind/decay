//! Windows, deep links, the hotkey and quitting (Software Design §4.9).
//!
//! Decay is a menu bar agent: no Dock icon (`LSUIElement`) until the Library or Settings
//! window is open, then a regular app (Dock icon, ⌘Tab) until both are closed again.
//! The search panel is created hidden at start so the hotkey shows it instantly; it hides
//! when it loses focus. (The non-activating `NSPanel` comes with the search UI, P1.9.)

use std::path::PathBuf;
use std::sync::Mutex;

use serde_json::json;
use tauri::window::{Effect, EffectsBuilder};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::core::{AppError, Core, quit_marker};
use crate::deeplink::{self, DeepLink, Window};
use crate::i18n::{t, tf};

/// App state beside the engine.
#[derive(Debug)]
pub struct Shell {
    pub support: PathBuf,
    /// The registered global hotkey (as stored in settings).
    pub hotkey: Mutex<Option<String>>,
    /// Why the engine couldn't start, if it couldn't.
    pub fatal: Mutex<Option<crate::core::StartError>>,
    /// What a window should pick up when it opens or is told to (`ui_take_pending`): a
    /// freshly created window would miss an event sent before its page listens.
    pub pending: Mutex<Pending>,
    /// A `decay://` link was handled (a cold start from a link opens only its window).
    pub link_seen: std::sync::atomic::AtomicBool,
    /// The native window theme (`None`: follow the OS), from the `theme` setting.
    pub theme: Mutex<Option<tauri::Theme>>,
    /// Whether onboarding is done (`onboarded_at`), and a Library request waiting to know.
    pub onboarding: Mutex<OnboardGate>,
    /// Serialises login-item changes (onboarding, Settings, start-up).
    pub autolaunch: Mutex<()>,
    /// The system's Accessibility prompt was shown in this run (macOS shows it once; later
    /// requests open System Settings instead).
    pub ax_prompted: std::sync::atomic::AtomicBool,
}

/// Whether onboarding is done, as far as the shell knows yet.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Onboarded {
    /// Start-up hasn't read the setting yet.
    #[default]
    Unknown,
    No,
    Yes,
}

/// Until onboarding is done the Library opens onboarding instead. Before start-up has read
/// the setting, a Library request (a launch link, the tray, the Dock) waits for it rather
/// than guessing.
#[derive(Debug, Default)]
pub struct OnboardGate {
    pub state: Onboarded,
    /// The route of a Library request made while the state was unknown (the latest wins).
    waiting: Option<String>,
}

impl OnboardGate {
    /// Where a Library request for `route` goes now; `None`: it waits for [`Self::resolve`].
    pub fn request(&mut self, route: &str) -> Option<(Window, String)> {
        match self.state {
            Onboarded::Yes => Some((Window::Library, route.to_owned())),
            Onboarded::No => Some((Window::Onboarding, "/".to_owned())),
            Onboarded::Unknown => {
                self.waiting = Some(route.to_owned());
                None
            }
        }
    }

    /// The setting is known; returns the waiting Library request's route, to open now.
    pub fn resolve(&mut self, onboarded: bool) -> Option<String> {
        self.state = if onboarded { Onboarded::Yes } else { Onboarded::No };
        self.waiting.take()
    }
}

/// Records whether onboarding is done and opens a Library request that waited for it.
pub fn set_onboarded(app: &AppHandle, onboarded: bool) {
    let waiting = app
        .state::<Shell>()
        .onboarding
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .resolve(onboarded);
    if let Some(route) = waiting {
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || show_window(&handle, Window::Library, &route));
    }
}

/// Data waiting for a window.
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pending {
    /// The panel's search text (`decay://search?q=`).
    pub prefill: Option<String>,
    /// A license key to confirm in Settings (`decay://license/activate`).
    pub license_key: Option<String>,
}

impl Shell {
    pub fn new(support: PathBuf) -> Self {
        Self {
            support,
            hotkey: Mutex::new(None),
            fatal: Mutex::new(None),
            pending: Mutex::new(Pending::default()),
            link_seen: std::sync::atomic::AtomicBool::new(false),
            theme: Mutex::new(None),
            onboarding: Mutex::new(OnboardGate::default()),
            autolaunch: Mutex::new(()),
            ax_prompted: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Takes what's waiting for `window` (each value is handed out once).
    pub fn take_pending(&self, window: &str) -> Pending {
        let mut p = self.pending.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        match window {
            "panel" => Pending { prefill: p.prefill.take(), license_key: None },
            "settings" => Pending { prefill: None, license_key: p.license_key.take() },
            _ => Pending::default(),
        }
    }

    pub fn hotkey(&self) -> Option<String> {
        self.hotkey.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
    }

    /// Onboarding is known to be done.
    pub fn onboarded(&self) -> bool {
        self.onboarding.lock().unwrap_or_else(std::sync::PoisonError::into_inner).state
            == Onboarded::Yes
    }

    /// Why the engine couldn't start, in the user's language.
    pub fn fatal(&self) -> Option<String> {
        let fatal = self.fatal.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        fatal.as_ref().map(crate::core::StartError::text)
    }
}

fn spec(window: Window) -> (String, f64, f64) {
    match window {
        Window::Panel => (t("window.search"), 680.0, 440.0),
        Window::Library => ("Decay".into(), 1100.0, 720.0),
        Window::Settings => (t("window.settings"), 920.0, 640.0),
        Window::Onboarding => (t("window.onboarding"), ONBOARDING_SIZE.0, ONBOARDING_SIZE.1),
    }
}

/// How a window is framed (UI Design Guidelines §8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chrome {
    /// The system's title bar (macOS; the macOS panel is an `NSPanel`).
    Native,
    /// Windows: no system frame; the page draws the title bar and caption buttons. `mica`
    /// on Windows 11, where the window is transparent over the Mica backdrop (Windows 10
    /// has none, so its window stays opaque).
    Custom { mica: bool },
    /// The search panel on Windows: frameless; `acrylic` on Windows 11 (the window is the
    /// card, over an Acrylic backdrop), else a transparent window with the card drawn in it.
    Panel { acrylic: bool },
}

/// The first Windows 11 build (Mica and Acrylic backdrops, rounded corners).
const WINDOWS_11: u32 = 22_000;

/// The chrome for `window`, given the Windows build number (`None`: not Windows).
pub fn chrome(window: Window, windows_build: Option<u32>) -> Chrome {
    let Some(build) = windows_build else { return Chrome::Native };
    let eleven = build >= WINDOWS_11;
    match window {
        Window::Panel => Chrome::Panel { acrylic: eleven },
        Window::Library | Window::Settings | Window::Onboarding => Chrome::Custom { mica: eleven },
    }
}

/// The page's query string: tells the page which chrome it is in (`chrome=custom`: draw
/// the title bar; `backdrop=mica|acrylic`: let the backdrop show through).
pub fn page_query(chrome: Chrome) -> &'static str {
    match chrome {
        Chrome::Native | Chrome::Panel { acrylic: false } => "",
        Chrome::Custom { mica: false } => "?chrome=custom",
        Chrome::Custom { mica: true } => "?chrome=custom&backdrop=mica",
        Chrome::Panel { acrylic: true } => "?backdrop=acrylic",
    }
}

/// The panel window's size: with the margin for the card's own shadow, or (Acrylic) the
/// card alone, which the system frames and shadows.
pub fn panel_size(chrome: Chrome) -> (f64, f64) {
    match chrome {
        Chrome::Panel { acrylic: true } => (PANEL_SIZE.0 - 32.0, PANEL_SIZE.1 - 32.0),
        _ => PANEL_SIZE,
    }
}

/// The Windows build number (`CurrentBuildNumber`), read once; `None` elsewhere.
pub fn windows_build() -> Option<u32> {
    #[cfg(windows)]
    {
        static BUILD: std::sync::OnceLock<Option<u32>> = std::sync::OnceLock::new();
        *BUILD.get_or_init(|| {
            use winreg::RegKey;
            use winreg::enums::HKEY_LOCAL_MACHINE;
            RegKey::predef(HKEY_LOCAL_MACHINE)
                .open_subkey(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion")
                .and_then(|k| k.get_value::<String, _>("CurrentBuildNumber"))
                .ok()
                .and_then(|b| b.trim().parse().ok())
                // Unreadable: Windows all the same (the plain variants work everywhere).
                .or(Some(0))
        })
    }
    #[cfg(not(windows))]
    None
}

fn build(app: &AppHandle, window: Window, route: &str) -> tauri::Result<WebviewWindow> {
    let (title, w, h) = spec(window);
    let chrome = chrome(window, windows_build());
    let url = WebviewUrl::App(format!("index.html{}#{route}", page_query(chrome)).into());
    let theme =
        *app.state::<Shell>().theme.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let builder = WebviewWindowBuilder::new(app, window.label(), url)
        .title(title)
        .inner_size(w, h)
        .theme(theme);
    let builder = match chrome {
        Chrome::Native => builder,
        Chrome::Custom { mica } => backdrop(builder.decorations(false), mica, Effect::Mica),
        Chrome::Panel { acrylic } => backdrop(builder, acrylic, Effect::Acrylic),
    };
    let built = match window {
        // Windows (macOS uses `mac_panel`): an undecorated, always-on-top window.
        Window::Panel => builder
            .inner_size(panel_size(chrome).0, panel_size(chrome).1)
            .transparent(true)
            .decorations(false)
            // The system shadow and rounded corners frame the Acrylic card; the plain
            // window draws its own card and shadow inside a transparent margin.
            .shadow(matches!(chrome, Chrome::Panel { acrylic: true }))
            .always_on_top(true)
            .skip_taskbar(true)
            .resizable(false)
            .visible(false)
            .center()
            .build()?,
        // A sidebar beside the tab (Designs/09); taller tabs scroll.
        Window::Settings => builder.min_inner_size(820.0, 520.0).center().build()?,
        Window::Onboarding => {
            builder.resizable(false).maximizable(false).minimizable(false).center().build()?
        }
        // UI Design Guidelines: Library minimum 860×560.
        Window::Library => builder.min_inner_size(860.0, 560.0).center().build()?,
    };
    #[cfg(windows)]
    if chrome != Chrome::Native {
        round_corners(&built);
    }
    Ok(built)
}

/// A window over a system backdrop (transparent, so the backdrop shows), or as it was.
pub fn backdrop(
    builder: WebviewWindowBuilder<'_, tauri::Wry, AppHandle>,
    backdrop: bool,
    effect: Effect,
) -> WebviewWindowBuilder<'_, tauri::Wry, AppHandle> {
    if backdrop {
        builder.transparent(true).effects(EffectsBuilder::new().effect(effect).build())
    } else {
        builder
    }
}

/// Windows 11: rounded corners for a window Decay frames itself (see
/// `decay_platform::round_corners`).
#[cfg(windows)]
pub fn round_corners(window: &WebviewWindow) {
    if let Ok(hwnd) = window.hwnd() {
        decay_platform::round_corners(hwnd.0 as isize);
    }
}

/// The `theme` setting (`system`, `light`, `dark`) as a native window theme.
pub fn native_theme(setting: &str) -> Option<tauri::Theme> {
    match setting {
        "light" => Some(tauri::Theme::Light),
        "dark" => Some(tauri::Theme::Dark),
        _ => None,
    }
}

/// Applies the `theme` setting to the native chrome of every window (title bars, menus);
/// the pages theme themselves from `settings:changed`.
pub fn apply_theme(app: &AppHandle, setting: &str) {
    let theme = native_theme(setting);
    *app.state::<Shell>().theme.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = theme;
    for window in app.webview_windows().values() {
        let _ = window.set_theme(theme);
    }
}

/// The onboarding window (logical px): the step list beside the step (Designs/03).
pub const ONBOARDING_SIZE: (f64, f64) = (840.0, 600.0);

/// The panel's size (logical px): 680 wide, up to 480 tall, plus a 16-px margin all
/// round for the card's shadow (the window itself is transparent).
pub const PANEL_SIZE: (f64, f64) = (712.0, 512.0);

/// Where the panel goes on a monitor, in logical points (monitor origin and size, panel
/// size): centred, a third of the way down (UI Design Guidelines §8). Logical units, so
/// the result is right whichever monitor the panel is on now (mixed 1×/2× setups).
pub fn panel_origin(monitor: (f64, f64, f64, f64), panel: (f64, f64)) -> (f64, f64) {
    let (mx, my, mw, mh) = monitor;
    let x = mx + (mw - panel.0) / 2.0;
    let y = my + ((mh - panel.1) / 3.0).max(0.0);
    (x, y)
}

/// Moves the panel to the screen with the pointer (the active screen).
fn place_panel(app: &AppHandle, panel: &WebviewWindow) {
    let cursor = app.cursor_position().ok();
    let monitors = app.available_monitors().unwrap_or_default();
    let monitor = cursor
        .and_then(|c| {
            monitors.iter().find(|m| {
                let (p, s) = (m.position(), m.size());
                c.x >= f64::from(p.x)
                    && c.y >= f64::from(p.y)
                    && c.x < f64::from(p.x) + f64::from(s.width)
                    && c.y < f64::from(p.y) + f64::from(s.height)
            })
        })
        .or_else(|| monitors.first());
    let Some(m) = monitor else {
        let _ = panel.center();
        return;
    };
    let scale = m.scale_factor();
    let pos = m.position().to_logical::<f64>(scale);
    let size = m.size().to_logical::<f64>(scale);
    let panel_size = panel_size(chrome(Window::Panel, windows_build()));
    let (x, y) = panel_origin((pos.x, pos.y, size.width, size.height), panel_size);
    let _ = panel.set_position(tauri::LogicalPosition::new(x, y));
}

/// Creates the hidden panel (at start).
pub fn prewarm_panel(app: &AppHandle) {
    if app.get_webview_window(Window::Panel.label()).is_some() {
        return;
    }
    #[cfg(target_os = "macos")]
    let built = mac_panel::build(app);
    #[cfg(not(target_os = "macos"))]
    let built = build(app, Window::Panel, "/panel").map(|_| ());
    if let Err(e) = built {
        tracing::warn!("panel: {e}");
    }
}

/// The search panel as a macOS `NSPanel` (Software Design §4.9): non-activating (typing in
/// it doesn't take the browser out of focus), floating above everything on every Space,
/// incl. full-screen apps, and hidden when it loses key focus.
#[cfg(target_os = "macos")]
#[allow(clippy::unused_unit, reason = "panel_event! requires an explicit return type")]
mod mac_panel {
    use tauri::{AppHandle, WebviewUrl};
    use tauri_nspanel::{
        CollectionBehavior, ManagerExt, PanelBuilder, PanelLevel, StyleMask, tauri_panel,
    };

    use super::{PANEL_SIZE, Window, hide_panel};

    tauri_panel! {
        panel!(SearchPanel {
            config: {
                can_become_key_window: true,
                can_become_main_window: false,
                is_floating_panel: true
            }
        })

        panel_event!(SearchPanelEvents {
            window_did_resign_key(notification: &NSNotification) -> ()
        })
    }

    pub fn build(app: &AppHandle) -> tauri::Result<()> {
        let panel = PanelBuilder::<_, SearchPanel>::new(app, Window::Panel.label())
            .url(WebviewUrl::App("index.html#/panel".into()))
            .title(crate::i18n::t("window.search"))
            .size(tauri::Size::Logical(tauri::LogicalSize::new(PANEL_SIZE.0, PANEL_SIZE.1)))
            .level(PanelLevel::ModalPanel)
            .collection_behavior(
                CollectionBehavior::new().can_join_all_spaces().full_screen_auxiliary().transient(),
            )
            .has_shadow(false)
            .opaque(false)
            .hides_on_deactivate(false)
            .with_window(|w| {
                w.decorations(false).transparent(true).visible(false).skip_taskbar(true)
            })
            .style_mask(StyleMask::empty().nonactivating_panel())
            .build()?;
        // The handler replaces the window's delegate (tauri-nspanel doesn't forward), so
        // Tauri's own window events (focus, move, resize, scale) don't fire for the panel:
        // anything the panel must react to goes here.
        let events = SearchPanelEvents::new();
        let handle = app.clone();
        events.window_did_resign_key(move |_| hide_panel(&handle));
        panel.set_event_handler(Some(events.as_ref()));
        Ok(())
    }

    /// Shows the panel and gives it the keyboard (the app itself stays in the background).
    /// `AppKit` calls run on the main thread, whoever asks.
    pub fn show(app: &AppHandle) -> bool {
        let exists = app.get_webview_panel(Window::Panel.label()).is_ok();
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || {
            if let Ok(p) = handle.get_webview_panel(Window::Panel.label()) {
                p.show_and_make_key();
            }
        });
        exists
    }

    pub fn hide(app: &AppHandle) -> bool {
        let exists = app.get_webview_panel(Window::Panel.label()).is_ok();
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || {
            if let Ok(p) = handle.get_webview_panel(Window::Panel.label()) {
                p.hide();
            }
        });
        exists
    }

    pub fn is_key(app: &AppHandle) -> bool {
        app.get_webview_panel(Window::Panel.label()).is_ok_and(|p| p.is_visible())
    }
}

/// Shows the panel, prefilled (hotkey, tray, `decay://search`). The page takes the
/// prefill with `ui_take_pending`, on load or on `panel:open`.
pub fn show_panel(app: &AppHandle, prefill: Option<&str>) {
    app.state::<Shell>()
        .pending
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .prefill = prefill.map(str::to_owned);
    prewarm_panel(app);
    let Some(panel) = app.get_webview_window(Window::Panel.label()) else { return };
    place_panel(app, &panel);
    let _ = panel.emit("panel:open", json!({}));
    #[cfg(target_os = "macos")]
    if mac_panel::show(app) {
        return;
    }
    let _ = panel.show();
    let _ = panel.set_focus();
}

pub fn hide_panel(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    if mac_panel::hide(app) {
        return;
    }
    if let Some(panel) = app.get_webview_window(Window::Panel.label()) {
        let _ = panel.hide();
    }
}

/// The hotkey: shows the panel, or hides it if it's showing.
pub fn toggle_panel(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    let visible = mac_panel::is_key(app);
    #[cfg(not(target_os = "macos"))]
    let visible = app
        .get_webview_window(Window::Panel.label())
        .is_some_and(|p| p.is_visible().unwrap_or(false) && p.is_focused().unwrap_or(false));
    if visible { hide_panel(app) } else { show_panel(app, None) }
}

/// Brings an open window to the front, where it is (no navigation); false if it isn't open.
pub fn focus_window(app: &AppHandle, window: Window) -> bool {
    let Some(existing) = app.get_webview_window(window.label()) else { return false };
    set_dock_visible(app, true);
    let _ = existing.unminimize();
    let _ = existing.show();
    let _ = existing.set_focus();
    true
}

/// Opens (or focuses) the Library, Settings or onboarding at `route`. Until onboarding is
/// done, the Library opens the onboarding instead (first run, tray, Dock, a second launch).
pub fn show_window(app: &AppHandle, window: Window, route: &str) {
    if window == Window::Panel {
        show_panel(app, None);
        return;
    }
    let (window, route) = if window == Window::Library {
        let gate = app
            .state::<Shell>()
            .onboarding
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .request(route);
        match gate {
            Some(target) => target,
            None => return, // opened once start-up knows (set_onboarded)
        }
    } else {
        (window, route.to_owned())
    };
    let route = route.as_str();
    if let Some(existing) = app.get_webview_window(window.label()) {
        set_dock_visible(app, true);
        let _ = existing.unminimize();
        let _ = existing.show();
        let _ = existing.set_focus();
        let _ = existing.emit("navigate", json!({ "window": window.label(), "route": route }));
        return;
    }
    match build(app, window, route) {
        Ok(w) => {
            set_dock_visible(app, true);
            let _ = w.set_focus();
        }
        Err(e) => tracing::warn!(window = %window.label(), "can't open: {e}"),
    }
}

/// Dock icon and ⌘Tab while a regular window is open (macOS; Windows shows a taskbar
/// button for every open window anyway).
pub fn set_dock_visible(app: &AppHandle, visible: bool) {
    #[cfg(target_os = "macos")]
    {
        let policy = if visible {
            tauri::ActivationPolicy::Regular
        } else {
            tauri::ActivationPolicy::Accessory
        };
        let _ = app.set_activation_policy(policy);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (app, visible);
}

/// A window closed: back to a menu bar agent when no regular window is left.
pub fn window_closed(app: &AppHandle, label: &str) {
    if label == Window::Panel.label() {
        return;
    }
    let regular_left = [Window::Library, Window::Settings, Window::Onboarding]
        .iter()
        .any(|w| w.label() != label && app.get_webview_window(w.label()).is_some());
    if !regular_left {
        set_dock_visible(app, false);
    }
}

/// Handles links the OS hands over while Decay runs (the deep-link plugin's `on_open_url`).
/// Not right away: on macOS that callback runs inside the plugin's event hook, with Tauri's
/// plugin store locked, and opening a window takes that lock too, so the app froze on every
/// link while running (v0.1.0 draft). Queued from another thread instead
/// (`run_on_main_thread` called on the main thread would run it at once), it runs on the
/// main thread after the hook has returned.
pub fn open_links_later(app: &AppHandle, urls: Vec<String>) {
    let app = app.clone();
    std::thread::spawn(move || {
        let handle = app.clone();
        let _ =
            app.run_on_main_thread(move || open_links(&handle, urls.iter().map(String::as_str)));
    });
}

/// Handles `decay://` links (from the OS, or forwarded from a second launch).
pub fn open_links<'a>(app: &AppHandle, links: impl IntoIterator<Item = &'a str>) {
    for raw in links {
        app.state::<Shell>().link_seen.store(true, std::sync::atomic::Ordering::SeqCst);
        let Some(link) = deeplink::parse(raw) else {
            tracing::warn!("ignored a malformed decay:// link");
            continue;
        };
        let (window, route) = link.target();
        match &link {
            DeepLink::Search { query } => show_panel(app, Some(query)),
            DeepLink::LicenseActivate { key } => {
                // The UI asks before activating; the key isn't put in the route but waits
                // for the Settings page to take it.
                app.state::<Shell>()
                    .pending
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .license_key = Some(key.clone());
                show_window(app, window, &route);
                let _ = app.emit_to(window.label(), "pending", json!({}));
            }
            _ => show_window(app, window, &route),
        }
    }
}

/// A hotkey change in progress: the new shortcut is registered next to the old one until
/// it's saved ([`HotkeySwap::commit`]) or dropped ([`HotkeySwap::rollback`]).
#[derive(Debug)]
pub struct HotkeySwap {
    accelerator: String,
    new: Option<Shortcut>,
    old: Option<Shortcut>,
}

/// Registers `accelerator` as the panel hotkey alongside the current one. Fails (and
/// changes nothing) when it isn't a shortcut or another app holds it.
pub fn try_hotkey(app: &AppHandle, accelerator: &str) -> Result<HotkeySwap, AppError> {
    let shortcut: Shortcut =
        accelerator.parse().map_err(|e: <Shortcut as std::str::FromStr>::Err| {
            AppError::new("INVALID_INPUT", tf("error.shortcut", &[("error", e.to_string().into())]))
        })?;
    let current = app.state::<Shell>().hotkey();
    let old = current.as_deref().and_then(|c| c.parse::<Shortcut>().ok());
    if old == Some(shortcut) {
        return Ok(HotkeySwap { accelerator: accelerator.to_owned(), new: None, old: None });
    }
    app.global_shortcut()
        .on_shortcut(shortcut, |app, _, event| {
            if event.state == ShortcutState::Pressed {
                toggle_panel(app);
            }
        })
        .map_err(|e| {
            AppError::new(
                "CONFLICT",
                tf(
                    "error.shortcutTaken",
                    &[("shortcut", accelerator.into()), ("error", e.to_string().into())],
                ),
            )
        })?;
    Ok(HotkeySwap { accelerator: accelerator.to_owned(), new: Some(shortcut), old })
}

impl HotkeySwap {
    /// The new hotkey replaces the old one.
    pub fn commit(self, app: &AppHandle) {
        if let Some(old) = self.old {
            let _ = app.global_shortcut().unregister(old);
        }
        *app.state::<Shell>().hotkey.lock().unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some(self.accelerator);
    }

    /// The old hotkey stays.
    pub fn rollback(self, app: &AppHandle) {
        if let Some(new) = self.new {
            let _ = app.global_shortcut().unregister(new);
        }
    }
}

/// Quit (tray, ⌘Q): the host won't start Decay again until the user does.
pub fn quit(app: &AppHandle) {
    let shell = app.state::<Shell>();
    let marker = quit_marker(&shell.support);
    if let Some(dir) = marker.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(&marker, b"") {
        tracing::warn!("quit marker: {e}");
    }
    app.state::<Core>().stop();
    app.exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_panel_sits_centred_a_third_down() {
        // A 2560×1440 screen at the origin, a 1424×1024 panel (712×512 at 2×).
        // A 1440×900-point screen at the origin.
        assert_eq!(
            panel_origin((0.0, 0.0, 1440.0, 900.0), PANEL_SIZE),
            (364.0, 129.333_333_333_333_34)
        );
        // A second monitor to the left, above the main one.
        assert_eq!(
            panel_origin((-1920.0, -200.0, 1920.0, 1080.0), (712.0, 512.0)),
            (-1316.0, -200.0 + 568.0 / 3.0)
        );
        // A panel taller than the screen stays on it.
        assert_eq!(panel_origin((0.0, 0.0, 800.0, 400.0), (712.0, 512.0)), (44.0, 0.0));
    }

    #[test]
    fn windows_draws_its_own_chrome_with_backdrops_on_windows_11() {
        // macOS: the system's.
        for w in [Window::Library, Window::Settings, Window::Onboarding, Window::Panel] {
            assert_eq!(chrome(w, None), Chrome::Native);
        }
        assert_eq!(page_query(Chrome::Native), "");
        // Windows 10 (19045): a custom title bar over an opaque page; the plain panel.
        assert_eq!(chrome(Window::Library, Some(19_045)), Chrome::Custom { mica: false });
        assert_eq!(chrome(Window::Panel, Some(19_045)), Chrome::Panel { acrylic: false });
        assert_eq!(page_query(Chrome::Custom { mica: false }), "?chrome=custom");
        assert_eq!(page_query(Chrome::Panel { acrylic: false }), "");
        assert_eq!(panel_size(Chrome::Panel { acrylic: false }), PANEL_SIZE);
        // Windows 11: Mica windows, an Acrylic panel that is just the card.
        for w in [Window::Library, Window::Settings, Window::Onboarding] {
            assert_eq!(chrome(w, Some(22_000)), Chrome::Custom { mica: true });
        }
        assert_eq!(chrome(Window::Panel, Some(26_100)), Chrome::Panel { acrylic: true });
        assert_eq!(page_query(Chrome::Custom { mica: true }), "?chrome=custom&backdrop=mica");
        assert_eq!(page_query(Chrome::Panel { acrylic: true }), "?backdrop=acrylic");
        assert_eq!(panel_size(Chrome::Panel { acrylic: true }), (680.0, 480.0));
        // An unreadable build number (0) gets the plain variants.
        assert_eq!(chrome(Window::Settings, Some(0)), Chrome::Custom { mica: false });
    }

    #[test]
    fn library_requests_wait_for_the_onboarding_state() {
        let mut gate = OnboardGate::default();
        // Unknown: requests wait; the latest is opened once known.
        assert_eq!(gate.request("/item/a"), None);
        assert_eq!(gate.request("/"), None);
        assert_eq!(gate.resolve(false), Some("/".to_owned()));
        assert_eq!(gate.resolve(false), None, "handed out once");
        // Not onboarded: onboarding instead.
        assert_eq!(gate.request("/item/a"), Some((Window::Onboarding, "/".to_owned())));
        assert_eq!(gate.resolve(true), None);
        assert_eq!(gate.request("/item/a"), Some((Window::Library, "/item/a".to_owned())));
    }

    #[test]
    fn the_theme_setting_maps_to_native_themes() {
        assert_eq!(native_theme("dark"), Some(tauri::Theme::Dark));
        assert_eq!(native_theme("light"), Some(tauri::Theme::Light));
        assert_eq!(native_theme("system"), None);
    }
}
