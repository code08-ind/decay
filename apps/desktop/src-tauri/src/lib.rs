//! Decay's desktop shell (P1.7, Software Design §4.9): a menu bar agent around the engine.
//!
//! - **Tray** (F16): stale-count badge, status line and the actions (Search, Hold 1 h,
//!   Pause/Resume, Run sweep now, Undo last sweep, Library, Settings, Quit).
//! - **Global hotkey** (F24): ⌥⌘D / Ctrl+Alt+D toggles the search panel; rebindable
//!   (`set_hotkey`, `CONFLICT` when taken).
//! - **Single instance and deep links** (F26): a second launch focuses the running app and
//!   forwards its `decay://` link.
//! - **Launch at login** (F17): `set_launch_at_login`; release builds apply the setting at
//!   start (debug builds never register themselves).
//! - **The engine**: started at launch with its key from the Keychain; its events become
//!   UI events (`status:changed`, `sweep:completed`, …) and tray updates.
//! - **Quit** writes `run/quit-by-user`, so the native host doesn't start Decay again.
//!
//! Launch arguments: `--background` (login item, started by the native host) shows no
//! window; a plain launch opens the Library (after a short wait for a `decay://` link on
//! macOS); `--version` / `--help` answer and exit. Opening Decay again from Finder shows
//! the Library. Quit (tray, ⌘Q) writes the quit marker; SIGTERM stops cleanly without it.

mod commands;
mod core;
mod deeplink;
mod keys;
mod menu;
mod shell;
mod tray;

use decay_engine::EngineEvent;
use serde_json::json;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Emitter, Manager, RunEvent, WindowEvent, Wry};
use tauri_plugin_deep_link::DeepLinkExt;

use crate::core::{Core, DataDir, install_host, quit_marker};
use crate::deeplink::Window;
use crate::shell::Shell;

/// Started without a window (login item, native host).
const BACKGROUND_ARG: &str = "--background";
/// The app menu's Quit item.
const APP_QUIT: &str = "app-quit";
/// How long a plain launch waits for a `decay://` link before opening the Library.
const LINK_GRACE: std::time::Duration = std::time::Duration::from_millis(700);

fn forward(app: &AppHandle, event: EngineEvent) {
    match event {
        EngineEvent::StatusChanged(status) => {
            let _ = app.emit("status:changed", &*status);
            app.state::<Core>().set_status(*status);
            let handle = app.clone();
            let _ = app.run_on_main_thread(move || tray::refresh(&handle));
        }
        EngineEvent::SweepCompleted { sweep_id, shadow, archived, failed } => {
            let _ = app.emit(
                "sweep:completed",
                json!({ "id": sweep_id, "isShadow": shadow, "archived": archived, "failed": failed }),
            );
        }
        EngineEvent::NothingToSweep { shadow } => {
            let _ = app.emit("sweep:nothing", json!({ "isShadow": shadow }));
        }
        EngineEvent::SweepUndone { sweep_id, restored, failed } => {
            let _ = app.emit(
                "sweep:undone",
                json!({ "sweepId": sweep_id, "restored": restored, "failed": failed }),
            );
        }
        EngineEvent::AccessibilityChanged { trusted } => {
            let accessibility = if trusted { "granted" } else { "denied" };
            let _ = app.emit("permission:changed", json!({ "accessibility": accessibility }));
        }
        EngineEvent::AdapterConnected { browser, profile_id } => {
            let _ = app.emit("adapter:health", json!({ "kind": "browser", "browser": browser, "profile": profile_id, "status": "connected" }));
        }
        EngineEvent::AdapterDisconnected { browser, profile_id } => {
            let _ = app.emit("adapter:health", json!({ "kind": "browser", "browser": browser, "profile": profile_id, "status": "down" }));
        }
        EngineEvent::SettingsChanged(settings) => commands::broadcast_settings(app, &settings),
        EngineEvent::Problem(what) => tracing::warn!("engine: {what}"),
    }
}

fn fail(app: &AppHandle, why: String) {
    tracing::error!("Decay can't start: {why}");
    *app.state::<Shell>().fatal.lock().unwrap_or_else(std::sync::PoisonError::into_inner) =
        Some(why);
    tray::refresh(app);
}

fn setup(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    shell::set_dock_visible(app, false);
    let support = app.state::<Shell>().support.clone();
    let _ = std::fs::remove_file(quit_marker(&support));
    tray::create(app)?;

    let handle = app.clone();
    let endpoint = decay_bridge::ipc::Endpoint::new_for_app()?;
    if let Err(why) =
        app.state::<Core>()
            .start(&DataDir::for_build(support.clone()), endpoint, move |e| forward(&handle, e))
    {
        fail(app, why);
    } else {
        // Settings come from the engine thread: read them off the main thread.
        let handle = app.clone();
        std::thread::spawn(move || {
            let (hotkey, theme) = handle
                .state::<Core>()
                .ask(|e, _| {
                    let s = e.store();
                    let read = (
                        s.setting::<String>("hotkey").ok().flatten(),
                        s.setting::<String>("theme").ok().flatten(),
                    );
                    (read, Vec::new())
                })
                .unwrap_or((None, None));
            if let Some(theme) = theme {
                let main = handle.clone();
                let _ = handle.run_on_main_thread(move || shell::apply_theme(&main, &theme));
            }
            if let Some(hotkey) = hotkey {
                let main = handle.clone();
                let _ = handle.run_on_main_thread(move || {
                    match shell::try_hotkey(&main, &hotkey) {
                        Ok(swap) => swap.commit(&main),
                        Err(e) => tracing::warn!("hotkey {hotkey}: {e}"),
                    }
                    tray::refresh(&main);
                });
            }
            apply_launch_at_login(&handle);
        });
    }
    std::thread::spawn(move || install_host(&support));
    shell::prewarm_panel(app);

    let links = app.clone();
    app.deep_link().on_open_url(move |event| {
        let urls: Vec<String> = event.urls().iter().map(ToString::to_string).collect();
        shell::open_links(&links, urls.iter().map(String::as_str));
    });
    #[cfg(all(debug_assertions, windows))]
    if let Err(e) = app.deep_link().register_all() {
        tracing::warn!("decay:// not registered: {e}");
    }
    let launch_links: Vec<String> = app
        .deep_link()
        .get_current()
        .ok()
        .flatten()
        .unwrap_or_default()
        .iter()
        .map(ToString::to_string)
        .collect();
    if !launch_links.is_empty() {
        shell::open_links(app, launch_links.iter().map(String::as_str));
    } else if !std::env::args().any(|a| a == BACKGROUND_ARG) {
        // A plain launch opens the Library — unless it was a launch from a link: macOS
        // delivers that link only after start-up, so wait for it briefly.
        let handle = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(LINK_GRACE);
            let main = handle.clone();
            let _ = handle.run_on_main_thread(move || {
                if !main.state::<Shell>().link_seen.load(std::sync::atomic::Ordering::SeqCst) {
                    shell::show_window(&main, Window::Library, "/");
                }
            });
        });
    }
    #[cfg(unix)]
    stop_on_signals(app);
    tray::refresh(app);
    Ok(())
}

/// SIGTERM / SIGINT (`launchctl stop`, `kill`, Ctrl+C) end the app like a system quit:
/// the engine stops in order (no quit marker: the user didn't quit).
#[cfg(unix)]
fn stop_on_signals(app: &AppHandle) {
    use tokio::signal::unix::{SignalKind, signal};
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let (Ok(mut term), Ok(mut int)) =
            (signal(SignalKind::terminate()), signal(SignalKind::interrupt()))
        else {
            tracing::warn!("signal handlers unavailable");
            return;
        };
        tokio::select! {
            _ = term.recv() => {}
            _ = int.recv() => {}
        }
        tracing::info!("stopping on a signal");
        handle.exit(0);
    });
}

/// The app's own menu (macOS menu bar while a window is open). Its Quit is the user's
/// quit, like the tray's: it writes the quit marker (the default menu's Quit didn't).
fn app_menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let quit = MenuItem::with_id(app, APP_QUIT, "Quit Decay", true, Some("CmdOrCtrl+Q"))?;
    let decay = Submenu::with_items(
        app,
        "Decay",
        true,
        &[
            &PredefinedMenuItem::about(app, None, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::hide(app, None)?,
            &PredefinedMenuItem::hide_others(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;
    let edit = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(app, None)?,
            &PredefinedMenuItem::redo(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
        ],
    )?;
    let window = Submenu::with_items(
        app,
        "Window",
        true,
        &[&PredefinedMenuItem::minimize(app, None)?, &PredefinedMenuItem::close_window(app, None)?],
    )?;
    Menu::with_items(app, &[&decay, &edit, &window])
}

/// Release builds follow the `launch_at_login` setting (on by default, F17); debug builds
/// never register a login item for a binary in `target/`.
fn apply_launch_at_login(app: &AppHandle) {
    if cfg!(debug_assertions) {
        return;
    }
    let wanted = app
        .state::<Core>()
        .ask(|e, _| (e.store().setting::<bool>("launch_at_login").ok().flatten(), Vec::new()))
        .ok()
        .flatten()
        .unwrap_or(true);
    let launcher = tauri_plugin_autostart::ManagerExt::autolaunch(app);
    if launcher.is_enabled().unwrap_or(!wanted) != wanted {
        let done = if wanted { launcher.enable() } else { launcher.disable() };
        if let Err(e) = done {
            tracing::warn!("launch at login: {e}");
        }
    }
}

/// A second launch: its link was forwarded to the deep-link handler; otherwise focus the
/// running app (unless it's a background start).
fn second_launch(app: &AppHandle, args: &[String]) {
    let has_link = args.iter().any(|a| a.to_ascii_lowercase().starts_with("decay://"));
    let background = args.iter().any(|a| a == BACKGROUND_ARG);
    if !has_link && !background {
        shell::show_window(app, Window::Library, "/");
    }
}

/// `--version` / `--help`: answered without starting anything.
fn answer_cli(args: &[String]) -> Option<String> {
    let arg = args.get(1)?;
    match arg.as_str() {
        "--version" | "-V" => Some(format!("Decay {}", env!("CARGO_PKG_VERSION"))),
        "--help" | "-h" => Some(format!(
            "Decay {}\n\nUsage: decay [--background] [decay://…]\n  --background  start without a window",
            env!("CARGO_PKG_VERSION")
        )),
        _ => None,
    }
}

/// Runs the app.
pub fn run() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(answer) = answer_cli(&args) {
        println!("{answer}");
        return;
    }
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_writer(std::io::stderr)
        .init();
    let Some(support) = decay_bridge::ipc::support_dir() else {
        tracing::error!("no home folder: Decay can't store anything");
        std::process::exit(1);
    };

    let app = tauri::Builder::default()
        // First, so a second launch never gets further than forwarding its arguments.
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| second_launch(app, &args)))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![BACKGROUND_ARG]),
        ))
        .manage(Core::default())
        .manage(Shell::new(support))
        .manage(tray::TrayState::default())
        .invoke_handler(tauri::generate_handler![
            commands::get_status,
            commands::set_mode,
            commands::resume,
            commands::hold,
            commands::release_hold,
            commands::run_sweep_now,
            commands::undo_sweep,
            commands::set_hotkey,
            commands::set_launch_at_login,
            commands::request_accessibility,
            commands::ui_show_window,
            commands::ui_hide_panel,
            commands::ui_take_pending,
            commands::get_settings,
            commands::update_settings,
        ])
        .menu(app_menu)
        .on_menu_event(|app, event| {
            if event.id() == APP_QUIT {
                shell::quit(app);
            }
        })
        .setup(|app| setup(app.handle()))
        .on_window_event(|window, event| match event {
            WindowEvent::Focused(false) if window.label() == Window::Panel.label() => {
                let _ = window.hide();
            }
            WindowEvent::Destroyed => shell::window_closed(window.app_handle(), window.label()),
            _ => {}
        })
        .build(tauri::generate_context!());
    let app = match app {
        Ok(app) => app,
        Err(e) => {
            tracing::error!("Decay can't start: {e}");
            std::process::exit(1);
        }
    };
    app.run(|app, event| match event {
        // Closing the last window keeps the menu bar agent running.
        RunEvent::ExitRequested { code: None, api, .. } => api.prevent_exit(),
        RunEvent::Exit => app.state::<Core>().stop(),
        // Opened again from Finder / Spotlight while running (no second process on macOS).
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { .. } => shell::show_window(app, Window::Library, "/"),
        _ => {}
    });
}

#[cfg(test)]
mod tests {
    use super::answer_cli;

    #[test]
    fn cli_flags_answer_without_starting() {
        let args = |a: &str| vec!["decay".to_owned(), a.to_owned()];
        assert!(answer_cli(&args("--version")).unwrap().starts_with("Decay "));
        assert!(answer_cli(&args("-h")).unwrap().contains("--background"));
        assert_eq!(answer_cli(&args("--background")), None);
        assert_eq!(answer_cli(&args("decay://search")), None);
        assert_eq!(answer_cli(&["decay".to_owned()]), None);
    }
}
