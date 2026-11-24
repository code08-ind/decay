//! Browser profiles (P2.8, F35): Settings → Browsers lists them and renames them; the
//! windows hear `profiles:changed` after a rename.

use decay_engine::{Engine, ForgetRefused, ProfileInfo, RenameRefused};
use decay_store::MAX_PROFILE_NAME;
use tauri::AppHandle;

use crate::commands::{Res, blocking};
use crate::core::AppError;
use crate::i18n::t;

/// What a profile call answers through `Core::ask`.
type Answer<T> = (Res<T>, Vec<decay_engine::Output>);

fn unknown() -> AppError {
    AppError::new("NOT_FOUND", t("error.profileUnknown"))
}

/// A name far longer than the store keeps, or an absurd app id, is a bad call.
fn checked(app_id: &str, name: Option<&str>) -> Res<()> {
    if name.is_some_and(|n| n.len() > MAX_PROFILE_NAME * 16) || app_id.len() > 256 {
        return Err(AppError::new("INVALID_INPUT", t("error.profileName")));
    }
    Ok(())
}

fn renamed(
    e: &mut Engine,
    app_id: &str,
    profile_id: &str,
    name: Option<&str>,
) -> Answer<Option<String>> {
    match e.rename_profile(app_id, profile_id, name) {
        Ok(Ok((stored, out))) => (Ok(stored), out),
        Ok(Err(RenameRefused::UnknownProfile)) => (Err(unknown()), Vec::new()),
        Err(err) => (Err(AppError::db(&err)), Vec::new()),
    }
}

fn forgotten(e: &mut Engine, app_id: &str, profile_id: &str) -> Answer<()> {
    match e.forget_profile(app_id, profile_id) {
        Ok(Ok(out)) => (Ok(()), out),
        Ok(Err(ForgetRefused::Connected)) => {
            (Err(AppError::new("INVALID_INPUT", t("error.profileConnected"))), Vec::new())
        }
        Ok(Err(ForgetRefused::UnknownProfile)) => (Err(unknown()), Vec::new()),
        Err(err) => (Err(AppError::db(&err)), Vec::new()),
    }
}

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
    checked(&app_id, name.as_deref())?;
    blocking(app, move |core| {
        core.ask(move |e, _| renamed(e, &app_id, &profile_id, name.as_deref()))?
    })
    .await
}

/// Forgets a browser profile that isn't connected (its tabs keep reopening only there; it
/// comes back if it connects again).
#[tauri::command]
pub async fn forget_profile(app: AppHandle, app_id: String, profile_id: String) -> Res<()> {
    blocking(app, move |core| core.ask(move |e, _| forgotten(e, &app_id, &profile_id))?).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{NOW, engine};

    #[test]
    fn unknown_profiles_and_bad_calls_are_refused() {
        let (mut e, _dir) = engine();
        assert_eq!(e.profiles(NOW).unwrap(), Vec::new());
        assert_eq!(
            renamed(&mut e, "com.google.Chrome", "p", Some("Work")).0.unwrap_err().code,
            "NOT_FOUND"
        );
        assert_eq!(forgotten(&mut e, "com.google.Chrome", "p").0.unwrap_err().code, "NOT_FOUND");
        assert!(checked("com.google.Chrome", Some("Work")).is_ok());
        assert!(checked("com.google.Chrome", None).is_ok());
        let long = "x".repeat(MAX_PROFILE_NAME * 16 + 1);
        assert_eq!(checked("com.google.Chrome", Some(&long)).unwrap_err().code, "INVALID_INPUT");
        assert_eq!(checked(&"a".repeat(257), None).unwrap_err().code, "INVALID_INPUT");
    }
}
