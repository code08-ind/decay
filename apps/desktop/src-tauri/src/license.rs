//! Licensing in the app (P4.2, PRD F27, API Design §2.7): the license row in the
//! database, the tier worked out from it (decay-license, offline), the engine's Pro
//! features following it, and `license:changed` for the windows.
//!
//! - **Activate:** a key that verifies is stored; anything else is refused with the reason.
//! - **Deactivate:** forgets the key (Pro ends now; a trial that's still running counts).
//! - **Trial:** 14 days of Pro, started by the user once per installation.
//! - **When Pro ends** (deactivation, or the trial running out, checked every few minutes)
//!   `free_since` records it: items archived from then on keep the free 90-day retention,
//!   older ones keep theirs (decay-engine `maintenance`). A free installation records the
//!   first time it was seen, so a retention set before licensing existed is kept for what
//!   was archived until then.
//! - Debug builds started with `DECAY_PRO=1` have Pro whatever the license says.

use std::time::Duration;

use decay_engine::Engine;
use decay_license::{Status, Stored, Tier};
use decay_store::LicenseRow;
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager};

use crate::commands::{Res, blocking};
use crate::core::{AppError, Core};
use crate::i18n::t;

/// How often a running trial is checked for its end.
const CHECK_EVERY: Duration = Duration::from_mins(5);

fn debug_pro() -> bool {
    cfg!(debug_assertions) && std::env::var_os("DECAY_PRO").is_some()
}

/// The status for a row at `now`.
fn status_of(row: &LicenseRow, now: i64) -> Status {
    decay_license::status(
        &Stored { key: row.key.clone(), trial_started_at: row.trial_started_at },
        now,
    )
}

fn tier_name(tier: Tier) -> &'static str {
    match tier {
        Tier::Free => "free",
        Tier::Trial => "trial",
        Tier::Pro => "pro",
    }
}

/// The row as it should be at `now`: the tier it shows, and when Pro ended (or since when
/// this installation has been free); `None` when nothing changes.
pub fn settle(row: &LicenseRow, status: &Status, now: i64) -> Option<LicenseRow> {
    let mut next = row.clone();
    tier_name(status.tier).clone_into(&mut next.tier);
    if status.tier.is_pro() {
        next.free_since = None;
    } else if next.free_since.is_none() {
        // The trial's end if that's what ended Pro; else now (deactivated, or never Pro).
        next.free_since = Some(status.trial_ends_at.filter(|end| *end <= now).unwrap_or(now));
    }
    (next != *row).then_some(next)
}

/// Brings the row and the engine in line with the license at `now`; the status.
fn refresh(e: &mut Engine, now: i64) -> Result<(Status, Vec<decay_engine::Output>), AppError> {
    let row = e.store().license().map_err(|err| AppError::db(&err))?;
    let status = status_of(&row, now);
    let row = match settle(&row, &status, now) {
        Some(next) => {
            e.store().save_license(&next).map_err(|err| AppError::db(&err))?;
            next
        }
        None => row,
    };
    let pro = status.tier.is_pro() || debug_pro();
    let out = e.set_license(pro, row.free_since, now).map_err(|err| AppError::db(&err))?;
    Ok((status, out))
}

/// [`refresh`] after `change` (a write to the row), shaped for `Core::ask`.
fn after(
    e: &mut Engine,
    now: i64,
    change: impl FnOnce(&Engine, i64) -> Result<(), AppError>,
) -> (Result<Status, AppError>, Vec<decay_engine::Output>) {
    match change(e, now).and_then(|()| refresh(e, now)) {
        Ok((status, out)) => (Ok(status), out),
        Err(err) => (Err(err), Vec::new()),
    }
}

/// Reads and writes the row (`change` edits it).
fn edit_row(
    e: &Engine,
    change: impl FnOnce(&mut LicenseRow) -> Result<(), AppError>,
) -> Result<(), AppError> {
    let mut row = e.store().license().map_err(|err| AppError::db(&err))?;
    change(&mut row)?;
    e.store().save_license(&row).map_err(|err| AppError::db(&err))
}

fn to_json(status: &Status) -> Value {
    serde_json::to_value(status).unwrap_or_else(|_| json!({ "tier": "free" }))
}

/// Applies a change and tells every window (and the tray) the new status.
fn changed(app: &AppHandle, status: &Status) -> Value {
    let value = to_json(status);
    let _ = app.emit("license:changed", &value);
    crate::tray::refresh(app);
    value
}

/// At start-up, then every few minutes: the engine follows the license; a trial ending
/// is announced.
pub fn start(app: &AppHandle) {
    let handle = app.clone();
    let spawned = std::thread::Builder::new().name("decay-license".into()).spawn(move || {
        let mut last: Option<Tier> = None;
        loop {
            let status =
                handle.state::<Core>().ask(|e, now| after(e, now, |_, _| Ok(()))).and_then(|s| s);
            match status {
                Ok(status) => {
                    if last.is_some_and(|t| t != status.tier) {
                        changed(&handle, &status);
                    }
                    last = Some(status.tier);
                }
                Err(e) => tracing::warn!("license: {}", e.message),
            }
            std::thread::sleep(CHECK_EVERY);
        }
    });
    if let Err(e) = spawned {
        tracing::warn!("license checks not started: {e}");
    }
}

/// `get_license`: `{ tier, trialDaysLeft?, trialEndsAt?, trialAvailable, email? }`.
#[tauri::command]
pub async fn get_license(app: AppHandle) -> Res<Value> {
    let status = blocking(app, |core| core.ask(|e, now| after(e, now, |_, _| Ok(())))?).await?;
    Ok(to_json(&status))
}

/// `activate_license { key }`: stores a key that verifies.
#[tauri::command]
pub async fn activate_license(app: AppHandle, key: String) -> Res<Value> {
    let license = decay_license::verify(&key).map_err(|why| {
        let reason = match why {
            decay_license::LicenseError::Format => "license.refused.format",
            decay_license::LicenseError::Signature => "license.refused.signature",
            decay_license::LicenseError::Unsupported => "license.refused.newer",
        };
        AppError::new("INVALID_INPUT", t(reason))
    })?;
    tracing::info!(license = %license.license_id, "license activated");
    let handle = app.clone();
    let status = blocking(app, move |core| {
        core.ask(move |e, now| {
            after(e, now, |e, now| {
                edit_row(e, |row| {
                    row.key = Some(key.trim().to_owned());
                    row.activated_at = Some(now);
                    Ok(())
                })
            })
        })?
    })
    .await?;
    Ok(changed(&handle, &status))
}

/// `deactivate_license`: forgets the key on this computer.
#[tauri::command]
pub async fn deactivate_license(app: AppHandle) -> Res<Value> {
    let handle = app.clone();
    let status = blocking(app, |core| {
        core.ask(|e, now| {
            after(e, now, |e, _| {
                edit_row(e, |row| {
                    row.key = None;
                    row.activated_at = None;
                    Ok(())
                })
            })
        })?
    })
    .await?;
    tracing::info!("license deactivated");
    Ok(changed(&handle, &status))
}

/// `start_trial`: 14 days of Pro, once per installation.
#[tauri::command]
pub async fn start_trial(app: AppHandle) -> Res<Value> {
    let handle = app.clone();
    let status = blocking(app, |core| {
        core.ask(|e, now| {
            after(e, now, |e, now| {
                edit_row(e, |row| {
                    if row.trial_started_at.is_some() {
                        return Err(AppError::new("CONFLICT", t("license.refused.trialUsed")));
                    }
                    row.trial_started_at = Some(now);
                    Ok(())
                })
            })
        })?
    })
    .await?;
    tracing::info!("trial started");
    Ok(changed(&handle, &status))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000_000;
    const DAY: i64 = 86_400_000;

    fn row(trial: Option<i64>, free_since: Option<i64>, tier: &str) -> LicenseRow {
        LicenseRow {
            tier: tier.into(),
            trial_started_at: trial,
            free_since,
            ..LicenseRow::default()
        }
    }

    #[test]
    fn free_since_marks_when_pro_ended() {
        // Never Pro: free since it was first seen.
        let fresh = row(None, None, "free");
        let next = settle(&fresh, &status_of(&fresh, NOW), NOW).unwrap();
        assert_eq!((next.tier.as_str(), next.free_since), ("free", Some(NOW)));
        assert_eq!(settle(&next, &status_of(&next, NOW + DAY), NOW + DAY), None, "kept");
        // A running trial: Pro (no free_since).
        let trial = row(Some(NOW), Some(NOW - DAY), "free");
        let next = settle(&trial, &status_of(&trial, NOW), NOW).unwrap();
        assert_eq!((next.tier.as_str(), next.free_since), ("trial", None));
        // The trial ran out while Decay wasn't checking: free since its end, not now.
        let later = NOW + 20 * DAY;
        let ended = settle(&next, &status_of(&next, later), later).unwrap();
        assert_eq!((ended.tier.as_str(), ended.free_since), ("free", Some(NOW + 14 * DAY)));
    }

    #[test]
    fn statuses_serialise_for_the_windows() {
        let st = status_of(&row(Some(NOW), None, "free"), NOW);
        let v = to_json(&st);
        assert_eq!(v["tier"], "trial");
        assert_eq!(v["trialDaysLeft"], 14);
    }
}
