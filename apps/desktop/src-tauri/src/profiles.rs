//! Browser profiles (P2.8, F35): Settings → Browsers lists them and renames them; the
//! windows hear `profiles:changed` after a rename.

use decay_engine::{ForgetRefused, ProfileInfo, RenameRefused};
use decay_store::MAX_PROFILE_NAME;
use tauri::AppHandle;

use crate::commands::{Res, blocking};
use crate::core::AppError;
use crate::i18n::t;

/// Every browser profile Decay has seen, by browser and first sighting.
#[tauri::command]
pub async fn list_profiles(app: AppHandle) -> Res<Vec<ProfileInfo>> {
    blocking(app, move |core| {
        core.ask(move |e, now| (e.profiles(now).map_err(|err| AppError::db(&err)), Vec::new()))?
    })
    .await
}

/// Names a profile (`null` or blank: unnamed). Returns the name as stored.
#[tauri::command]
pub async fn rename_profile(
    app: AppHandle,
    app_id: String,
    profile_id: String,
    name: Option<String>,
) -> Res<Option<String>> {
    // The store trims and cuts to MAX_PROFILE_NAME; anything far longer is a bad call.
    if name.as_ref().is_some_and(|n| n.len() > MAX_PROFILE_NAME * 16) || app_id.len() > 256 {
        return Err(AppError::new("INVALID_INPUT", t("error.profileName")));
    }
    blocking(app, move |core| {
        core.ask(move |e, _| match e.rename_profile(&app_id, &profile_id, name.as_deref()) {
            Ok(Ok((stored, out))) => (Ok(stored), out),
            Ok(Err(RenameRefused::UnknownProfile)) => {
                (Err(AppError::new("NOT_FOUND", t("error.profileUnknown"))), Vec::new())
            }
            Err(err) => (Err(AppError::db(&err)), Vec::new()),
        })?
    })
    .await
}

/// Forgets a browser profile that isn't connected (its tabs keep reopening only there; it
/// comes back if it connects again).
#[tauri::command]
pub async fn forget_profile(app: AppHandle, app_id: String, profile_id: String) -> Res<()> {
    blocking(app, move |core| {
        core.ask(move |e, _| match e.forget_profile(&app_id, &profile_id) {
            Ok(Ok(out)) => (Ok(()), out),
            Ok(Err(ForgetRefused::Connected)) => {
                (Err(AppError::new("INVALID_INPUT", t("error.profileConnected"))), Vec::new())
            }
            Ok(Err(ForgetRefused::UnknownProfile)) => {
                (Err(AppError::new("NOT_FOUND", t("error.profileUnknown"))), Vec::new())
            }
            Err(err) => (Err(AppError::db(&err)), Vec::new()),
        })?
    })
    .await
}
