//! Windows, deep links, the hotkey and quitting (Software Design §4.9).
//!
//! Decay is a menu bar agent: no Dock icon (`LSUIElement`) until the Library or Settings
//! window is open, then a regular app (Dock icon, ⌘Tab) until both are closed again.
//! The search panel is created hidden at start so the hotkey shows it instantly; it hides
//! when it loses focus. (The non-activating `NSPanel` comes with the search UI, P1.9.)

use std::path::PathBuf;
use std::sync::Mutex;

use serde_json::json;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::core::{AppError, Core, quit_marker};
use crate::deeplink::{self, DeepLink, Window};

/// App state beside the engine.
#[derive(Debug)]
pub struct Shell {
    pub support: PathBuf,
    /// The registered global hotkey (as stored in settings).
    pub hotkey: Mutex<Option<String>>,
    /// Why the engine couldn't start, if it couldn't.
    pub fatal: Mutex<Option<String>>,
    /// What a window should pick up when it opens or is told to (`ui_take_pending`): a
    /// freshly created window would miss an event sent before its page listens.
    pub pending: Mutex<Pending>,
    /// A `decay://` link was handled (a cold start from a link opens only its window).
    pub link_seen: std::sync::atomic::AtomicBool,
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

    pub fn fatal(&self) -> Option<String> {
        self.fatal.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
    }
}

fn spec(window: Window) -> (&'static str, f64, f64) {
    match window {
        Window::Panel => ("Search", 680.0, 440.0),
        Window::Library => ("Decay", 1100.0, 720.0),
        Window::Settings => ("Settings", 720.0, 560.0),
    }
}

fn build(app: &AppHandle, window: Window, route: &str) -> tauri::Result<WebviewWindow> {
    let (title, w, h) = spec(window);
    let url = WebviewUrl::App(format!("index.html#{route}").into());
    let builder = WebviewWindowBuilder::new(app, window.label(), url).title(title).inner_size(w, h);
    match window {
        Window::Panel => builder
            .decorations(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .resizable(false)
            .visible(false)
            .center()
            .build(),
        Window::Settings => builder.resizable(false).center().build(),
        Window::Library => builder.min_inner_size(800.0, 520.0).center().build(),
    }
}

/// Creates the hidden panel (at start).
pub fn prewarm_panel(app: &AppHandle) {
    if app.get_webview_window(Window::Panel.label()).is_none()
        && let Err(e) = build(app, Window::Panel, "/panel")
    {
        tracing::warn!("panel: {e}");
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
    let _ = panel.center();
    let _ = panel.show();
    let _ = panel.set_focus();
    let _ = panel.emit("panel:open", json!({}));
}

pub fn hide_panel(app: &AppHandle) {
    if let Some(panel) = app.get_webview_window(Window::Panel.label()) {
        let _ = panel.hide();
    }
}

/// The hotkey: shows the panel, or hides it if it's in front.
pub fn toggle_panel(app: &AppHandle) {
    let visible = app
        .get_webview_window(Window::Panel.label())
        .is_some_and(|p| p.is_visible().unwrap_or(false) && p.is_focused().unwrap_or(false));
    if visible { hide_panel(app) } else { show_panel(app, None) }
}

/// Opens (or focuses) the Library or Settings at `route`.
pub fn show_window(app: &AppHandle, window: Window, route: &str) {
    if window == Window::Panel {
        show_panel(app, None);
        return;
    }
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
        Err(e) => tracing::warn!(window = window.label(), "can't open: {e}"),
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
    let regular_left = [Window::Library, Window::Settings]
        .iter()
        .any(|w| w.label() != label && app.get_webview_window(w.label()).is_some());
    if !regular_left {
        set_dock_visible(app, false);
    }
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
    let shortcut: Shortcut = accelerator
        .parse()
        .map_err(|e| AppError::new("INVALID_INPUT", format!("not a shortcut: {e}")))?;
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
        .map_err(|e| AppError::new("CONFLICT", format!("{accelerator} is in use: {e}")))?;
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
