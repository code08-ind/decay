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
use decay_license::{PUBLIC_KEY, Status, Stored, Tier};
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

/// The status for a row at `now`, keys checked against `pk` (the embedded public key
/// outside tests).
fn status_of(row: &LicenseRow, now: i64, pk: &[u8; 32]) -> Status {
    let stored = Stored { key: row.key.clone(), trial_started_at: row.trial_started_at };
    decay_license::status_with(&stored, now, pk)
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

/// What a license call answers through `Core::ask`.
type Answer = (Result<Status, AppError>, Vec<decay_engine::Output>);

/// Brings the row and the engine in line with the license at `now`; the status.
fn refresh(
    e: &mut Engine,
    now: i64,
    pk: &[u8; 32],
) -> Result<(Status, Vec<decay_engine::Output>), AppError> {
    let row = e.store().license().map_err(|err| AppError::db(&err))?;
    let status = status_of(&row, now, pk);
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

/// [`refresh`] after `change` (an edit of the row; nothing is written when it fails).
fn after(
    e: &mut Engine,
    now: i64,
    pk: &[u8; 32],
    change: impl FnOnce(&mut LicenseRow, i64) -> Result<(), AppError>,
) -> Answer {
    let edited = e.store().license().map_err(|err| AppError::db(&err)).and_then(|mut row| {
        let before = row.clone();
        change(&mut row, now)?;
        if row == before {
            Ok(())
        } else {
            e.store().save_license(&row).map_err(|err| AppError::db(&err))
        }
    });
    match edited.and_then(|()| refresh(e, now, pk)) {
        Ok((status, out)) => (Ok(status), out),
        Err(err) => (Err(err), Vec::new()),
    }
}

/// The license as it is now (and the engine following it).
fn current(e: &mut Engine, now: i64, pk: &[u8; 32]) -> Answer {
    after(e, now, pk, |_, _| Ok(()))
}

/// Why a key was refused, in the user's words.
fn refused(why: decay_license::LicenseError) -> AppError {
    let reason = match why {
        decay_license::LicenseError::Format => "license.refused.format",
        decay_license::LicenseError::Signature => "license.refused.signature",
        decay_license::LicenseError::Unsupported => "license.refused.newer",
    };
    AppError::new("INVALID_INPUT", t(reason))
}

/// Stores a key that verifies against `pk`; anything else is refused, nothing changes.
fn activate(e: &mut Engine, now: i64, pk: &[u8; 32], key: &str) -> Answer {
    let license = match decay_license::verify_with(key, pk) {
        Ok(l) => l,
        Err(why) => return (Err(refused(why)), Vec::new()),
    };
    tracing::info!(license = %license.license_id, "license activated");
    after(e, now, pk, |row, now| {
        row.key = Some(key.trim().to_owned());
        row.activated_at = Some(now);
        Ok(())
    })
}

/// Forgets the key on this computer.
fn deactivate(e: &mut Engine, now: i64, pk: &[u8; 32]) -> Answer {
    tracing::info!("license deactivated");
    after(e, now, pk, |row, _| {
        row.key = None;
        row.activated_at = None;
        Ok(())
    })
}

/// Starts the 14-day trial, once per installation.
fn begin_trial(e: &mut Engine, now: i64, pk: &[u8; 32]) -> Answer {
    after(e, now, pk, |row, now| {
        if row.trial_started_at.is_some() {
            return Err(AppError::new("CONFLICT", t("license.refused.trialUsed")));
        }
        row.trial_started_at = Some(now);
        tracing::info!("trial started");
        Ok(())
    })
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
                handle.state::<Core>().ask(|e, now| current(e, now, &PUBLIC_KEY)).and_then(|s| s);
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

/// Runs a license call on the engine thread.
async fn call(app: AppHandle, f: fn(&mut Engine, i64, &[u8; 32]) -> Answer) -> Res<Status> {
    blocking(app, move |core| core.ask(move |e, now| f(e, now, &PUBLIC_KEY))?).await
}

/// `get_license`: `{ tier, trialDaysLeft?, trialEndsAt?, trialAvailable, email? }`.
#[tauri::command]
pub async fn get_license(app: AppHandle) -> Res<Value> {
    Ok(to_json(&call(app, current).await?))
}

/// `activate_license { key }`: stores a key that verifies.
#[tauri::command]
pub async fn activate_license(app: AppHandle, key: String) -> Res<Value> {
    let handle = app.clone();
    let status =
        blocking(app, move |core| core.ask(move |e, now| activate(e, now, &PUBLIC_KEY, &key))?)
            .await?;
    Ok(changed(&handle, &status))
}

/// `deactivate_license`: forgets the key on this computer.
#[tauri::command]
pub async fn deactivate_license(app: AppHandle) -> Res<Value> {
    let handle = app.clone();
    let status = call(app, deactivate).await?;
    Ok(changed(&handle, &status))
}

/// `start_trial`: 14 days of Pro, once per installation.
#[tauri::command]
pub async fn start_trial(app: AppHandle) -> Res<Value> {
    let handle = app.clone();
    let status = call(app, begin_trial).await?;
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
        let next = settle(&fresh, &status_of(&fresh, NOW, &PUBLIC_KEY), NOW).unwrap();
        assert_eq!((next.tier.as_str(), next.free_since), ("free", Some(NOW)));
        assert_eq!(
            settle(&next, &status_of(&next, NOW + DAY, &PUBLIC_KEY), NOW + DAY),
            None,
            "kept"
        );
        // A running trial: Pro (no free_since).
        let trial = row(Some(NOW), Some(NOW - DAY), "free");
        let next = settle(&trial, &status_of(&trial, NOW, &PUBLIC_KEY), NOW).unwrap();
        assert_eq!((next.tier.as_str(), next.free_since), ("trial", None));
        // The trial ran out while Decay wasn't checking: free since its end, not now.
        let later = NOW + 20 * DAY;
        let ended = settle(&next, &status_of(&next, later, &PUBLIC_KEY), later).unwrap();
        assert_eq!((ended.tier.as_str(), ended.free_since), ("free", Some(NOW + 14 * DAY)));
    }

    #[test]
    fn statuses_serialise_for_the_windows() {
        let st = status_of(&row(Some(NOW), None, "free"), NOW, &PUBLIC_KEY);
        let v = to_json(&st);
        assert_eq!(v["tier"], "trial");
        assert_eq!(v["trialDaysLeft"], 14);
    }

    use crate::testing::engine;

    const SEED: [u8; 32] = [9; 32];

    fn pk() -> [u8; 32] {
        decay_license::public_key_of(&SEED)
    }

    fn key(email: &str) -> String {
        let license = decay_license::License {
            v: 1,
            email: email.into(),
            tier: "pro".into(),
            issued_at: NOW,
            license_id: "t-1".into(),
        };
        decay_license::issue(&license, &SEED)
    }

    #[test]
    fn a_fresh_installation_is_free_since_it_was_first_seen() {
        let (mut e, _dir) = engine();
        let status = current(&mut e, NOW, &pk()).0.unwrap();
        assert_eq!((status.tier, status.trial_available), (Tier::Free, true));
        assert!(!e.is_pro() || debug_pro());
        assert_eq!(e.store().license().unwrap().free_since, Some(NOW));
    }

    #[test]
    fn activating_and_deactivating_a_key() {
        let (mut e, _dir) = engine();
        let (refused, out) = activate(&mut e, NOW, &pk(), "DECAY-NOPE");
        assert_eq!(refused.unwrap_err().code, "INVALID_INPUT");
        assert_eq!(out, Vec::new());
        assert_eq!(e.store().license().unwrap().key, None, "nothing stored");
        // A key from another issuer is refused too.
        let other = activate(&mut e, NOW, &decay_license::public_key_of(&[1; 32]), &key("a@b.c"));
        assert_eq!(other.0.unwrap_err().code, "INVALID_INPUT");

        let pro =
            activate(&mut e, NOW, &pk(), &format!("  {}\n", key("me@example.com"))).0.unwrap();
        assert_eq!((pro.tier, pro.email.as_deref()), (Tier::Pro, Some("me@example.com")));
        assert!(e.is_pro());
        let row = e.store().license().unwrap();
        assert_eq!((row.tier.as_str(), row.activated_at, row.free_since), ("pro", Some(NOW), None));
        assert!(row.key.is_some_and(|k| k == key("me@example.com")), "stored trimmed");

        let later = NOW + DAY;
        let free = deactivate(&mut e, later, &pk()).0.unwrap();
        assert_eq!(free.tier, Tier::Free);
        assert!(!e.is_pro() || debug_pro());
        let row = e.store().license().unwrap();
        assert_eq!((row.key, row.activated_at, row.free_since), (None, None, Some(later)));
    }

    #[test]
    fn the_trial_runs_once() {
        let (mut e, _dir) = engine();
        let trial = begin_trial(&mut e, NOW, &pk()).0.unwrap();
        assert_eq!(
            (trial.tier, trial.trial_days_left, trial.trial_available),
            (Tier::Trial, Some(14), false)
        );
        assert!(e.is_pro());
        let again = begin_trial(&mut e, NOW + DAY, &pk());
        assert_eq!(again.0.unwrap_err().code, "CONFLICT");
        // Fifteen days on: free again, since the trial's end.
        let over = current(&mut e, NOW + 15 * DAY, &pk()).0.unwrap();
        assert_eq!(over.tier, Tier::Free);
        assert_eq!(e.store().license().unwrap().free_since, Some(NOW + 14 * DAY));
    }

    #[test]
    fn refusals_explain_themselves() {
        use decay_license::LicenseError::{Format, Signature, Unsupported};
        for why in [Format, Signature, Unsupported] {
            let err = refused(why);
            assert_eq!(err.code, "INVALID_INPUT");
            assert!(!err.message.is_empty() && !err.message.starts_with("license."), "{why:?}");
        }
    }
}
