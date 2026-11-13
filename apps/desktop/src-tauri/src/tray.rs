//! The menu bar item (F16): icon, stale-count badge, tooltip and menu, rebuilt from the
//! engine's status each time it changes (see `menu` for what it shows).

use decay_core::Mode;
use std::sync::Mutex;
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;

use tauri::{AppHandle, Manager, Wry};

use crate::core::Core;
use crate::deeplink::Window;
use crate::menu::{self, Entry, id};
use crate::shell::{self, Shell};

const TRAY: &str = "decay";
const HOLD_MS: i64 = 3_600_000;

/// The entries on screen and the status item (relabelled in place).
type Shown = (Vec<Entry>, Option<MenuItem<Wry>>);

/// The menu on screen.
#[derive(Default)]
pub struct TrayState {
    shown: Mutex<Option<Shown>>,
}

impl std::fmt::Debug for TrayState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TrayState")
    }
}

/// The local UTC offset at a moment (daylight saving may change before a hold ends).
fn offset_at(ms: i64) -> i32 {
    jiff::Timestamp::from_millisecond(ms)
        .map_or(0, |t| t.to_zoned(jiff::tz::TimeZone::system()).offset().seconds() / 60)
}

fn current_entries(app: &AppHandle) -> Vec<Entry> {
    let shell = app.state::<Shell>();
    match shell.fatal() {
        Some(why) => menu::failed_entries(&why),
        None => menu::entries(
            app.state::<Core>().status().as_ref(),
            shell.hotkey().as_deref(),
            &offset_at,
        ),
    }
}

/// Builds the menu; returns it with its first (status) item.
fn build_menu(
    app: &AppHandle,
    entries: &[Entry],
) -> tauri::Result<(Menu<Wry>, Option<MenuItem<Wry>>)> {
    let menu = Menu::new(app)?;
    let mut first = None;
    for entry in entries {
        match entry {
            Entry::Separator => menu.append(&PredefinedMenuItem::separator(app)?)?,
            Entry::Item { id, label, enabled, accelerator } => {
                // An accelerator the menu can't display (it parses differently from the
                // global shortcut) is left off rather than losing the menu.
                let item = MenuItem::with_id(app, *id, label, *enabled, accelerator.as_deref())
                    .or_else(|_| MenuItem::with_id(app, *id, label, *enabled, None::<&str>))?;
                menu.append(&item)?;
                first.get_or_insert(item);
            }
        }
    }
    Ok((menu, first))
}

/// Creates the menu bar item.
pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let icon = Image::from_bytes(include_bytes!("../icons/tray/44x44.png"))?;
    let entries = current_entries(app);
    let (menu, status_item) = build_menu(app, &entries)?;
    TrayIconBuilder::with_id(TRAY)
        .icon(icon)
        .icon_as_template(true)
        .tooltip("Decay")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| on_menu(app, event.id().as_ref()))
        .build(app)?;
    *lock(&app.state::<TrayState>().shown) = Some((entries, status_item));
    Ok(())
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Brings badge, tooltip and menu up to date (on the main thread). Only a menu whose
/// items changed is rebuilt; a new count just relabels the status line.
pub fn refresh(app: &AppHandle) {
    let Some(tray) = app.tray_by_id(TRAY) else { return };
    let status = app.state::<Core>().status();
    let failed = app.state::<Shell>().fatal().is_some();
    let badge = if failed { Some("!".to_owned()) } else { menu::badge(status.as_ref()) };
    #[cfg(target_os = "macos")]
    let _ = tray.set_title(badge.as_deref());
    #[cfg(not(target_os = "macos"))]
    let _ = &badge; // Windows: the count is in the tooltip (an icon overlay comes later).
    let _ = tray.set_tooltip(Some(menu::tooltip(status.as_ref(), &offset_at)));

    let entries = current_entries(app);
    let state = app.state::<TrayState>();
    let mut shown = lock(&state.shown);
    if let Some((old, status_item)) = shown.as_mut()
        && menu::same_shape(old, &entries)
    {
        if let (Some(Entry::Item { label, .. }), Some(Entry::Item { label: was, .. })) =
            (entries.first(), old.first())
            && label != was
            && let Some(item) = status_item
        {
            let _ = item.set_text(label);
        }
        *old = entries;
        return;
    }
    match build_menu(app, &entries) {
        Ok((m, status_item)) => {
            let _ = tray.set_menu(Some(m));
            *shown = Some((entries, status_item));
        }
        Err(e) => tracing::warn!("tray menu: {e}"),
    }
}

/// Logs a failed tray action (the menu has no place for errors; the status shows the state).
fn report(what: &'static str) -> impl Fn(decay_store::StoreError) -> Vec<decay_engine::Output> {
    move |e| {
        tracing::warn!("{what} failed: {e}");
        Vec::new()
    }
}

fn on_menu(app: &AppHandle, item: &str) {
    let core = app.state::<Core>();
    match item {
        id::SEARCH => shell::show_panel(app, None),
        id::HOLD => {
            core.send(move |e, now| {
                e.hold(Some(now + HOLD_MS), now).unwrap_or_else(report("hold"))
            });
        }
        id::RELEASE_HOLD => {
            core.send(move |e, now| e.hold(None, now).unwrap_or_else(report("release hold")));
        }
        id::PAUSE => {
            core.send(move |e, now| e.set_mode(Mode::Paused, now).unwrap_or_else(report("pause")));
        }
        id::RESUME => core.send(move |e, now| e.resume(now).unwrap_or_else(report("resume"))),
        id::SWEEP_NOW => core.send(|e, now| {
            e.sweep_now(now).unwrap_or_else(|why| {
                tracing::info!("sweep refused: {why}");
                Vec::new()
            })
        }),
        id::UNDO => core.send(|e, now| match e.undo(None, now) {
            Ok(Ok(out)) => out,
            Ok(Err(why)) => {
                tracing::info!("undo refused: {why}");
                Vec::new()
            }
            Err(err) => report("undo")(err),
        }),
        id::LIBRARY => shell::show_window(app, Window::Library, "/"),
        id::SETTINGS => shell::show_window(app, Window::Settings, "/general"),
        id::QUIT => shell::quit(app),
        _ => {}
    }
}
