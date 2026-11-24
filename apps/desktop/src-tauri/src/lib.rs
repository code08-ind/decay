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
mod diagnostics;
mod help;
mod i18n;
mod keys;
mod labels;
mod library;
mod license;
mod menu;
mod metrics;
mod notify;
mod onboarding;
#[cfg(test)]
mod testing;
// The tray popover is Windows-only; its placement is tested everywhere.
#[cfg_attr(not(windows), allow(dead_code))]
mod popover;
mod profiles;
mod rules;
mod search;
mod shell;
mod summary;
mod transfer;
mod tray;
// Only the Windows tray draws its icon; the drawing is tested everywhere.
#[cfg_attr(not(windows), allow(dead_code))]
mod tray_icon;
mod update;

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
const HELP_DOCS: &str = "help-docs";
const HELP_SHORTCUTS: &str = "help-shortcuts";
const HELP_WHATS_NEW: &str = "help-whats-new";
const HELP_FEEDBACK: &str = "help-feedback";
/// How long a plain launch waits for a `decay://` link before opening the Library.
const LINK_GRACE: std::time::Duration = std::time::Duration::from_millis(700);

fn forward(app: &AppHandle, event: EngineEvent) {
    let now = jiff::Timestamp::now().as_millisecond();
    app.state::<notify::Notifier>().on_event(app, &event, now);
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
        EngineEvent::WindowsHidden { sweep_id, hidden, failed } => {
            let _ = app.emit(
                "windows:hidden",
                json!({ "sweepId": sweep_id, "hidden": hidden, "failed": failed }),
            );
        }
        EngineEvent::WindowsClosed { sweep_id, closed, failed } => {
            let _ = app.emit(
                "windows:closed",
                json!({ "sweepId": sweep_id, "closed": closed, "failed": failed }),
            );
        }
        EngineEvent::AccessibilityChanged { trusted } => {
            let accessibility = if trusted { "granted" } else { "denied" };
            let _ = app.emit("permission:changed", json!({ "accessibility": accessibility }));
        }
        EngineEvent::AdapterConnected { browser, profile_id } => {
            tracing::info!(?browser, "browser connected");
            let _ = app.emit("adapter:health", json!({ "kind": "browser", "browser": browser, "profile": profile_id, "status": "connected" }));
        }
        EngineEvent::AdapterDisconnected { browser, profile_id } => {
            tracing::info!(?browser, "browser disconnected");
            let _ = app.emit("adapter:health", json!({ "kind": "browser", "browser": browser, "profile": profile_id, "status": "down" }));
        }
        EngineEvent::Restored { restored, failed, skipped } => {
            let _ = app.emit(
                "items:restored",
                json!({ "restored": restored, "failed": failed, "skipped": skipped }),
            );
        }
        EngineEvent::ProfilesChanged => {
            let _ = app.emit("profiles:changed", json!({}));
        }
        EngineEvent::SettingsChanged(settings) => commands::broadcast_settings(app, &settings),
        EngineEvent::Problem(what) => tracing::warn!("engine: {what}"),
    }
}

fn fail(app: &AppHandle, why: crate::core::StartError) {
    tracing::error!("Decay can't start: {why}");
    // No settings without the engine: the system's language.
    apply_locale(app, None);
    // Onboarding can't work without the engine: the Library shows the error.
    shell::set_onboarded(app, true);
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
        update::start(app);
        notify::start(app);
        license::start(app);
        labels::start(app);
        // Settings come from the engine thread: read them off the main thread.
        let handle = app.clone();
        std::thread::spawn(move || {
            let (hotkey, theme, onboarded, settings) = handle
                .state::<Core>()
                .ask(|e, _| {
                    let s = e.store();
                    let mut settings = s.settings_snapshot().ok();
                    if let (Some(map), Ok(Some(day))) =
                        (settings.as_mut(), s.setting::<String>("daily_summary_sent_on"))
                    {
                        map.insert("daily_summary_sent_on".into(), day.into());
                    }
                    let read = (
                        s.setting::<String>("hotkey").ok().flatten(),
                        s.setting::<String>("theme").ok().flatten(),
                        e.onboarded().unwrap_or(true),
                        settings,
                    );
                    (read, Vec::new())
                })
                .unwrap_or((None, None, true, None));
            if let Some(settings) = &settings {
                handle.state::<notify::Notifier>().set_settings(settings);
                diagnostics::apply_settings(settings);
            }
            apply_locale(&handle, settings.as_ref().and_then(|s| s.get("locale")?.as_str()));
            shell::set_onboarded(&handle, onboarded);
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
    std::thread::spawn(move || {
        let _ = install_host(&support);
    });
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

/// Follows the `locale` setting (F31): the app's text, and the menus already built with
/// the previous language.
pub(crate) fn apply_locale(app: &AppHandle, setting: Option<&str>) {
    let before = i18n::locale();
    i18n::set_locale(setting);
    if i18n::locale() == before {
        return; // the menus already say it this way (the first build was in English)
    }
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Ok(menu) = app_menu(&handle) {
            let _ = handle.set_menu(menu);
        }
        tray::refresh(&handle);
    });
}

/// The app's own menu (macOS menu bar while a window is open). Its Quit is the user's
/// quit, like the tray's: it writes the quit marker (the default menu's Quit didn't).
fn app_menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    use i18n::t;
    let s = |key: &str| Some(t(key));
    let quit = MenuItem::with_id(app, APP_QUIT, t("menu.quit"), true, Some("CmdOrCtrl+Q"))?;
    let decay = Submenu::with_items(
        app,
        "Decay",
        true,
        &[
            &PredefinedMenuItem::about(app, s("menu.about").as_deref(), None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::hide(app, s("menu.hide").as_deref())?,
            &PredefinedMenuItem::hide_others(app, s("menu.hideOthers").as_deref())?,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;
    let edit = Submenu::with_items(
        app,
        t("menu.edit"),
        true,
        &[
            &PredefinedMenuItem::undo(app, s("menu.undo").as_deref())?,
            &PredefinedMenuItem::redo(app, s("menu.redo").as_deref())?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, s("menu.cut").as_deref())?,
            &PredefinedMenuItem::copy(app, s("menu.copy").as_deref())?,
            &PredefinedMenuItem::paste(app, s("menu.paste").as_deref())?,
            &PredefinedMenuItem::select_all(app, s("menu.selectAll").as_deref())?,
        ],
    )?;
    let window = Submenu::with_items(
        app,
        t("menu.window"),
        true,
        &[
            &PredefinedMenuItem::minimize(app, s("menu.minimize").as_deref())?,
            &PredefinedMenuItem::close_window(app, s("menu.close").as_deref())?,
        ],
    )?;
    // Help (F32). macOS adds its menu search to the help menu: registered by role, so the
    // title can be in any language.
    let help = Submenu::with_items(
        app,
        t("menu.help"),
        true,
        &[
            &MenuItem::with_id(app, HELP_DOCS, t("menu.docs"), true, None::<&str>)?,
            &MenuItem::with_id(
                app,
                HELP_SHORTCUTS,
                t("menu.shortcuts"),
                true,
                Some("CmdOrCtrl+/"),
            )?,
            &MenuItem::with_id(app, HELP_WHATS_NEW, t("menu.whatsNew"), true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, HELP_FEEDBACK, t("menu.feedback"), true, None::<&str>)?,
        ],
    )?;
    #[cfg(target_os = "macos")]
    {
        let _ = window.set_as_windows_menu_for_nsapp();
        let _ = help.set_as_help_menu_for_nsapp();
    }
    Menu::with_items(app, &[&decay, &edit, &window, &help])
}

/// Release builds follow the `launch_at_login` setting (on by default, F17) once
/// onboarding is done (no login item before the user has seen the choice); debug builds
/// never register a login item for a binary in `target/`.
pub(crate) fn apply_launch_at_login(app: &AppHandle) {
    if cfg!(debug_assertions) || !app.state::<Shell>().onboarded() {
        return;
    }
    let shell = app.state::<Shell>();
    let _one_at_a_time = shell.autolaunch.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
#[allow(clippy::too_many_lines, reason = "the builder lists every command")]
pub fn run() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(answer) = answer_cli(&args) {
        println!("{answer}");
        return;
    }
    let support = decay_bridge::ipc::support_dir();
    // Crash reports first, on this (the main) thread before any other exists, so a
    // panic anywhere is caught; nothing is sent until the setting is on.
    diagnostics::init_crash_reports();
    diagnostics::init_logging(support.as_deref());
    tracing::info!(version = %env!("CARGO_PKG_VERSION"), "Decay starting");
    let Some(support) = support else {
        tracing::error!("no home folder: Decay can't store anything");
        std::process::exit(1);
    };

    let builder = tauri::Builder::default()
        // First, so a second launch never gets further than forwarding its arguments.
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| second_launch(app, &args)))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![BACKGROUND_ARG]),
        ));
    // The search panel is an NSPanel on macOS.
    #[cfg(target_os = "macos")]
    let builder = builder.plugin(tauri_nspanel::init());
    let app = builder
        .manage(Core::default())
        .manage(Shell::new(support))
        .manage(tray::TrayState::default())
        .manage(popover::PopoverState::default())
        .manage(update::Updates::default())
        .manage(notify::Notifier::default())
        .manage(transfer::PendingImport::default())
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
            onboarding::request_accessibility,
            onboarding::detect_browsers,
            onboarding::install_native_hosts,
            onboarding::open_extension_store,
            onboarding::complete_onboarding,
            commands::ui_show_window,
            commands::ui_hide_panel,
            commands::ui_open_search,
            commands::ui_quit,
            commands::ui_take_pending,
            commands::get_settings,
            license::get_license,
            license::activate_license,
            license::deactivate_license,
            license::start_trial,
            update::get_update_state,
            summary::get_daily_summary,
            update::check_for_updates,
            update::install_update,
            profiles::list_profiles,
            profiles::rename_profile,
            profiles::forget_profile,
            search::search,
            search::restore_items,
            search::restore_batch,
            search::restore_items_in,
            search::protect_item,
            search::ui_show_item,
            commands::update_settings,
            library::library_stats,
            library::get_item,
            library::delete_items,
            library::list_sweeps,
            library::get_sweep,
            library::get_shadow_report,
            rules::list_rules,
            rules::create_rule,
            rules::update_rule,
            rules::delete_rule,
            rules::test_rule,
            rules::preview_exclude,
            transfer::export_archive,
            transfer::import_archive,
            transfer::confirm_import,
            diagnostics::get_diagnostics_info,
            diagnostics::open_logs_folder,
            diagnostics::export_diagnostics,
            diagnostics::log_ui_error,
            metrics::get_metrics,
            metrics::record_search,
            metrics::get_beta_context,
            metrics::save_metrics_file,
            help::open_link,
            i18n::system_language,
            help::whats_new,
            help::whats_new_seen,
            help::ui_show_whats_new,
        ])
        .menu(app_menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            APP_QUIT => shell::quit(app),
            HELP_DOCS => help::open_link_from_menu(app, help::Link::Help),
            HELP_FEEDBACK => help::open_link_from_menu(app, help::Link::Feedback),
            HELP_SHORTCUTS => shell::show_window(app, Window::Settings, "/shortcuts"),
            HELP_WHATS_NEW => help::show_whats_new(app),
            _ => {}
        })
        .setup(|app| setup(app.handle()))
        .on_window_event(|window, event| match event {
            WindowEvent::Focused(false) if window.label() == Window::Panel.label() => {
                let _ = window.hide();
            }
            WindowEvent::Focused(false) if window.label() == popover::LABEL => {
                popover::blurred(window.app_handle());
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
        RunEvent::Exit => {
            app.state::<Core>().stop();
            update::install_on_quit(app);
        }
        // Opened again from Finder / Spotlight while running (no second process on macOS).
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { .. } => {
            // Opened right after the morning notice (most likely its click): the summary.
            let now = jiff::Timestamp::now().as_millisecond();
            let summary = app.state::<notify::Notifier>().take_summary_click(now);
            shell::show_window(app, Window::Library, if summary { "/summary" } else { "/" });
        }
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
