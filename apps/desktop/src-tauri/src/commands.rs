//! Tauri commands (API Design §2) of the shell: status and control, the hotkey, launch at
//! login, the theme setting and windows. Search, Library and the rest of Settings come
//! with their screens.
//!
//! Commands that wait for the engine run on a blocking thread, never the main thread.

#![allow(clippy::needless_pass_by_value, reason = "Tauri commands take their arguments by value")]

use decay_core::Mode;
use decay_engine::{EngineStatus, SweepRefused, UndoRefused};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_autostart::ManagerExt;

use crate::core::{AppError, Core};
use crate::deeplink::Window;
use crate::menu::HOLD_INDEFINITE_MS;
use crate::shell;

pub(crate) type Res<T> = Result<T, AppError>;

/// Runs `f` (which waits for the engine) on a blocking thread, never the main thread.
pub(crate) async fn blocking<T: Send + 'static>(
    app: AppHandle,
    f: impl FnOnce(&Core) -> Res<T> + Send + 'static,
) -> Res<T> {
    tauri::async_runtime::spawn_blocking(move || f(&app.state::<Core>()))
        .await
        .map_err(|e| AppError::new("TIMEOUT", e.to_string()))?
}

/// `hold { duration }`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum HoldDuration {
    Named(NamedHold),
    Minutes { minutes: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum NamedHold {
    #[serde(rename = "1h")]
    OneHour,
    #[serde(rename = "untilTomorrow")]
    UntilTomorrow,
    #[serde(rename = "indefinite")]
    Indefinite,
}

/// "Until tomorrow" ends at the next 06:00 local time at least this far away: in the
/// evening that's tomorrow morning; at 02:00 it's this morning, not a day later.
const UNTIL_TOMORROW_MIN_MS: i64 = 2 * 3_600_000;

/// When a hold of `duration` starting at `now` ends. "Until tomorrow" = the next 06:00
/// local time (a hold for the evening that doesn't eat into the next workday).
pub fn hold_end(duration: &HoldDuration, now: &jiff::Zoned) -> Res<i64> {
    let ms = now.timestamp().as_millisecond();
    Ok(match duration {
        HoldDuration::Named(NamedHold::OneHour) => ms + 3_600_000,
        HoldDuration::Named(NamedHold::Indefinite) => HOLD_INDEFINITE_MS,
        HoldDuration::Named(NamedHold::UntilTomorrow) => {
            let invalid = |e: jiff::Error| AppError::new("INVALID_INPUT", e.to_string());
            let six_on = |date: jiff::civil::Date| {
                date.at(6, 0, 0, 0)
                    .to_zoned(now.time_zone().clone())
                    .map(|z| z.timestamp().as_millisecond())
                    .map_err(invalid)
            };
            let today = six_on(now.date())?;
            if today - ms >= UNTIL_TOMORROW_MIN_MS {
                today
            } else {
                six_on(now.date().tomorrow().map_err(invalid)?)?
            }
        }
        HoldDuration::Minutes { minutes } => {
            if *minutes == 0 || *minutes > 7 * 24 * 60 {
                return Err(AppError::new("INVALID_INPUT", "a hold lasts 1 minute to 7 days"));
            }
            ms + i64::from(*minutes) * 60_000
        }
    })
}

#[tauri::command]
pub async fn get_status(app: AppHandle) -> Res<EngineStatus> {
    blocking(app, |core| core.ask(|e, now| (e.status(now), Vec::new()))).await
}

#[tauri::command]
pub async fn set_mode(app: AppHandle, mode: Mode) -> Res<Value> {
    blocking(app, move |core| {
        core.ask(move |e, now| match e.set_mode(mode, now) {
            Ok(out) => (Ok(json!({ "mode": mode })), out),
            Err(err) => (Err(AppError::db(&err)), Vec::new()),
        })?
    })
    .await
}

/// Ends a pause: back to the mode before it (`set_mode` would lose it).
#[tauri::command]
pub async fn resume(app: AppHandle) -> Res<Value> {
    blocking(app, |core| {
        core.ask(|e, now| match e.resume(now) {
            Ok(out) => (Ok(json!({ "mode": e.status(now).mode })), out),
            Err(err) => (Err(AppError::db(&err)), Vec::new()),
        })?
    })
    .await
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HoldResult {
    hold_until: Option<i64>,
}

#[tauri::command]
pub async fn hold(app: AppHandle, duration: HoldDuration) -> Res<HoldResult> {
    let until = hold_end(&duration, &jiff::Zoned::now())?;
    blocking(app, move |core| {
        core.ask(move |e, now| match e.hold(Some(until), now) {
            Ok(out) => (Ok(HoldResult { hold_until: Some(until) }), out),
            Err(err) => (Err(AppError::db(&err)), Vec::new()),
        })?
    })
    .await
}

#[tauri::command]
pub async fn release_hold(app: AppHandle) -> Res<Value> {
    blocking(app, |core| {
        core.ask(|e, now| match e.hold(None, now) {
            Ok(out) => (Ok(json!({})), out),
            Err(err) => (Err(AppError::db(&err)), Vec::new()),
        })?
    })
    .await
}

fn sweep_refused(why: &SweepRefused) -> AppError {
    let code = match why {
        SweepRefused::NoBrowser => "ADAPTER_UNAVAILABLE",
        SweepRefused::Busy => "CONFLICT",
        SweepRefused::Paused | SweepRefused::OnHold(_) => "UNSAFE",
    };
    AppError::new(code, why.to_string())
}

/// Starts a sweep; its result comes as `sweep:completed`.
#[tauri::command]
pub async fn run_sweep_now(app: AppHandle) -> Res<Value> {
    blocking(app, |core| {
        core.ask(|e, now| match e.sweep_now(now) {
            Ok(out) => (Ok(json!({ "started": true })), out),
            Err(why) => (Err(sweep_refused(&why)), Vec::new()),
        })?
    })
    .await
}

/// Starts an Undo; its result comes as `sweep:undone`.
#[tauri::command]
pub async fn undo_sweep(app: AppHandle, id: Option<String>) -> Res<Value> {
    blocking(app, move |core| {
        core.ask(move |e, now| match e.undo(id.as_deref(), now) {
            Ok(Ok(out)) => (Ok(json!({ "started": true })), out),
            Ok(Err(why)) => {
                let code = match why {
                    UndoRefused::NoBrowser => "ADAPTER_UNAVAILABLE",
                    UndoRefused::StillClosing => "CONFLICT",
                    UndoRefused::NothingToUndo => "NOT_FOUND",
                };
                (Err(AppError::new(code, why.to_string())), Vec::new())
            }
            Err(err) => (Err(AppError::db(&err)), Vec::new()),
        })?
    })
    .await
}

/// Registers the panel hotkey and saves it (`CONFLICT` if another app holds it).
#[tauri::command]
pub async fn set_hotkey(app: AppHandle, accelerator: String) -> Res<Value> {
    // Registered next to the old one, saved, and only then does it replace it: the live
    // hotkey and the saved one never disagree.
    let swap = shell::try_hotkey(&app, &accelerator)?;
    let stored = save_settings(app.clone(), vec![("hotkey".into(), json!(accelerator))]).await;
    if let Err(e) = stored {
        swap.rollback(&app);
        return Err(e);
    }
    swap.commit(&app);
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || crate::tray::refresh(&handle));
    Ok(json!({ "ok": true }))
}

/// What's waiting for the calling window (a search prefill, a license key to confirm).
#[tauri::command]
pub fn ui_take_pending(window: tauri::Window, shell: State<'_, shell::Shell>) -> shell::Pending {
    shell.take_pending(window.label())
}

/// Settings the shell lets the UI change so far (the Settings screens add the rest).
/// Each value is checked before it's stored.
pub fn validate_settings(patch: &serde_json::Map<String, Value>) -> Res<Vec<(String, Value)>> {
    if patch.is_empty() {
        return Err(AppError::new("INVALID_INPUT", "nothing to change"));
    }
    patch
        .iter()
        .map(|(key, value)| match key.as_str() {
            "theme" if matches!(value.as_str(), Some("system" | "light" | "dark")) => {
                Ok((key.clone(), value.clone()))
            }
            "theme" => Err(AppError::new("INVALID_INPUT", "theme is system, light or dark")),
            _ => Err(AppError::new("INVALID_INPUT", format!("{key} can't be changed here"))),
        })
        .collect()
}

/// Every setting with defaults filled in (DB Design §3.9).
#[tauri::command]
pub async fn get_settings(app: AppHandle) -> Res<serde_json::Map<String, Value>> {
    blocking(app, |core| {
        core.ask(|e, _| {
            (e.store().settings_snapshot().map_err(|err| AppError::db(&err)), Vec::new())
        })?
    })
    .await
}

/// Tells every window about the settings (`settings:changed`) and applies the theme to
/// the native chrome. Called on the engine thread right after a change, so broadcasts
/// keep the order of the writes.
pub fn broadcast_settings(app: &AppHandle, settings: &serde_json::Map<String, Value>) {
    if let Some(theme) = settings.get("theme").and_then(Value::as_str) {
        let handle = app.clone();
        let theme = theme.to_owned();
        let _ = app.run_on_main_thread(move || shell::apply_theme(&handle, &theme));
    }
    let _ = tauri::Emitter::emit(app, "settings:changed", settings);
}

/// Saves `changes` on the engine thread and broadcasts the result from there.
async fn save_settings(
    app: AppHandle,
    changes: Vec<(String, Value)>,
) -> Res<serde_json::Map<String, Value>> {
    let handle = app.clone();
    blocking(app, move |core| {
        core.ask(move |e, now| {
            let saved = changes
                .iter()
                .try_for_each(|(k, v)| e.store().set_setting(k, v, now))
                .and_then(|()| e.store().settings_snapshot())
                .map_err(|err| AppError::db(&err));
            if let Ok(settings) = &saved {
                broadcast_settings(&handle, settings);
            }
            (saved, Vec::new())
        })?
    })
    .await
}

/// Changes settings (so far: `theme`); every window hears `settings:changed`.
#[tauri::command]
pub async fn update_settings(
    app: AppHandle,
    patch: serde_json::Map<String, Value>,
) -> Res<serde_json::Map<String, Value>> {
    let changes = validate_settings(&patch)?;
    save_settings(app, changes).await
}

/// Turns launch at login on or off (F17). Debug builds only save the setting: they never
/// register a login item for a binary in `target/`.
#[tauri::command]
pub async fn set_launch_at_login(app: AppHandle, enabled: bool) -> Res<Value> {
    if !cfg!(debug_assertions) {
        let handle = app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let shell = handle.state::<shell::Shell>();
            let _one_at_a_time =
                shell.autolaunch.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let launcher = handle.autolaunch();
            if enabled { launcher.enable() } else { launcher.disable() }
        })
        .await
        .map_err(|e| AppError::new("INTERNAL", e.to_string()))?
        .map_err(|e| AppError::new("PERMISSION_DENIED", e.to_string()))?;
    }
    save_settings(app, vec![("launch_at_login".into(), json!(enabled))]).await?;
    Ok(json!({ "enabled": enabled }))
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShowWindow {
    Library,
    Settings,
    Onboarding,
}

#[tauri::command]
pub fn ui_show_window(app: AppHandle, window: ShowWindow, route: Option<String>) -> Value {
    let (window, default) = match window {
        ShowWindow::Library => (Window::Library, "/"),
        ShowWindow::Settings => (Window::Settings, "/general"),
        ShowWindow::Onboarding => (Window::Onboarding, "/"),
    };
    // Routes are in-app paths only.
    let route = route.filter(|r| r.starts_with('/') && r.len() <= 200 && !r.contains("//"));
    shell::show_window(&app, window, route.as_deref().unwrap_or(default));
    json!({})
}

#[tauri::command]
pub fn ui_hide_panel(app: AppHandle) -> Value {
    shell::hide_panel(&app);
    json!({})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> jiff::Zoned {
        s.parse().unwrap()
    }

    #[test]
    fn hold_durations() {
        let now = at("2026-11-13T22:15:00+01:00[Europe/Paris]");
        let ms = now.timestamp().as_millisecond();
        let parse = |v: Value| serde_json::from_value::<HoldDuration>(v).unwrap();
        assert_eq!(hold_end(&parse(json!("1h")), &now).unwrap(), ms + 3_600_000);
        assert_eq!(hold_end(&parse(json!({"minutes": 30})), &now).unwrap(), ms + 1_800_000);
        assert_eq!(hold_end(&parse(json!("indefinite")), &now).unwrap(), HOLD_INDEFINITE_MS);
        let tomorrow = hold_end(&parse(json!("untilTomorrow")), &now).unwrap();
        assert_eq!(
            tomorrow,
            at("2026-11-14T06:00:00+01:00[Europe/Paris]").timestamp().as_millisecond()
        );
        for bad in [json!({"minutes": 0}), json!({"minutes": 20_000})] {
            assert_eq!(hold_end(&parse(bad), &now).unwrap_err().code, "INVALID_INPUT");
        }
        assert!(serde_json::from_value::<HoldDuration>(json!("forever")).is_err());
    }

    #[test]
    fn until_tomorrow_after_midnight_is_this_morning() {
        let hold = HoldDuration::Named(NamedHold::UntilTomorrow);
        let at_two = at("2026-11-14T02:00:00+01:00[Europe/Paris]");
        assert_eq!(
            hold_end(&hold, &at_two).unwrap(),
            at("2026-11-14T06:00:00+01:00[Europe/Paris]").timestamp().as_millisecond()
        );
        // Too close to 06:00: the next morning.
        let at_five = at("2026-11-14T05:00:00+01:00[Europe/Paris]");
        assert_eq!(
            hold_end(&hold, &at_five).unwrap(),
            at("2026-11-15T06:00:00+01:00[Europe/Paris]").timestamp().as_millisecond()
        );
    }

    #[test]
    fn until_tomorrow_across_a_clock_change() {
        // The night summer time ends in Paris (25-hour day): still 06:00 local.
        let now = at("2026-10-24T23:00:00+02:00[Europe/Paris]");
        let end = hold_end(&HoldDuration::Named(NamedHold::UntilTomorrow), &now).unwrap();
        assert_eq!(end, at("2026-10-25T06:00:00+01:00[Europe/Paris]").timestamp().as_millisecond());
    }

    #[test]
    fn only_known_settings_with_valid_values_are_accepted() {
        let patch = |v: Value| v.as_object().unwrap().clone();
        let ok = validate_settings(&patch(json!({"theme": "dark"}))).unwrap();
        assert_eq!(ok, vec![("theme".to_owned(), json!("dark"))]);
        for bad in
            [json!({"theme": "neon"}), json!({"theme": 1}), json!({"mode": "paused"}), json!({})]
        {
            assert_eq!(validate_settings(&patch(bad)).unwrap_err().code, "INVALID_INPUT");
        }
    }

    #[test]
    fn refusals_map_to_app_errors() {
        assert_eq!(sweep_refused(&SweepRefused::NoBrowser).code, "ADAPTER_UNAVAILABLE");
        assert_eq!(sweep_refused(&SweepRefused::Busy).code, "CONFLICT");
        assert_eq!(sweep_refused(&SweepRefused::OnHold(1)).code, "UNSAFE");
    }
}
