//! Insights (P2.7, PRD F34, Designs/14): on-device metrics for the Library's Insights
//! page, the searches the panel reports, and saving the "Share with beta team" and CSV
//! files the page builds (shown to the user in full before they're saved).

#![allow(clippy::needless_pass_by_value, reason = "Tauri commands take their arguments by value")]

use decay_core::{RuleAction, SqlEnum as _};
use decay_engine::MetricsReport;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::{AppHandle, Window};
use tauri_plugin_dialog::DialogExt;

use crate::commands::{Res, blocking};
use crate::core::AppError;

/// Largest metrics file the page may ask to save.
const MAX_FILE_BYTES: usize = 4 * 1024 * 1024;

/// Every stored day and the top sites since `since` (ms; default 30 days ago).
#[tauri::command]
pub async fn get_metrics(app: AppHandle, since: Option<i64>) -> Res<MetricsReport> {
    blocking(app, move |core| {
        core.ask(move |e, now| {
            let since = since.unwrap_or(now - 30 * 86_400_000);
            (e.metrics(since, now).map_err(|err| AppError::db(&err)), Vec::new())
        })?
    })
    .await
}

/// One search in the search panel (the panel calls this once per opening).
#[tauri::command]
pub async fn record_search(app: AppHandle) -> Res<()> {
    blocking(app, |core| {
        core.ask(|e, now| (e.record_search(now).map_err(|err| AppError::db(&err)), Vec::new()))?
    })
    .await
}

/// What the beta export says about this installation (no content: counts, settings that
/// describe how Decay is set up, browser kinds, a random install id).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BetaContext {
    pub install_id: String,
    pub app_version: String,
    pub platform: &'static str,
    /// Connected browser kinds (no profiles).
    pub browsers: Vec<Value>,
    pub mode: decay_core::Mode,
    pub preset: String,
    pub thresholds: decay_core::Thresholds,
    pub shadow_started_at: Option<i64>,
    pub window_management: String,
    /// Enabled rules per action (no patterns).
    pub rules: serde_json::Map<String, Value>,
}

/// The beta export's context (makes the install id on first use).
#[tauri::command]
pub async fn get_beta_context(app: AppHandle) -> Res<BetaContext> {
    blocking(app, |core| {
        let status = core.status();
        core.ask(move |e, now| {
            let read = || -> Result<BetaContext, decay_store::StoreError> {
                let s = e.store();
                let mut rules = serde_json::Map::new();
                for action in [
                    RuleAction::Protect,
                    RuleAction::Exclude,
                    RuleAction::CustomThreshold,
                    RuleAction::CloseAllowed,
                ] {
                    let n = s.rules()?.iter().filter(|r| r.enabled && r.action == action).count();
                    let key = serde_json::to_value(action)
                        .ok()
                        .and_then(|v| v.as_str().map(str::to_owned))
                        .unwrap_or_else(|| action.as_sql().to_owned());
                    rules.insert(key, json!(n));
                }
                let mut browsers: Vec<Value> = status
                    .iter()
                    .flat_map(|st| st.browsers.iter().map(|b| json!(b.browser)))
                    .collect();
                browsers.sort_by_key(ToString::to_string);
                browsers.dedup();
                Ok(BetaContext {
                    install_id: s.beta_install_id(now)?,
                    app_version: env!("CARGO_PKG_VERSION").to_owned(),
                    platform: if cfg!(windows) { "windows" } else { "macos" },
                    browsers,
                    mode: s.mode()?,
                    preset: s.setting::<String>("preset")?.unwrap_or_else(|| "balanced".into()),
                    thresholds: s.thresholds()?,
                    shadow_started_at: s.setting::<i64>("shadow_started_at")?,
                    window_management: s
                        .setting::<String>("window_management")?
                        .unwrap_or_else(|| "hide".into()),
                    rules,
                })
            };
            (read().map_err(|err| AppError::db(&err)), Vec::new())
        })?
    })
    .await
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MetricsFile {
    /// `decay-beta-metrics` JSON.
    Beta,
    /// Daily rows.
    Csv,
}

/// Checks a file the page asks to save is what it says: the beta JSON (its format
/// header) or the daily CSV (its header row).
fn checked(kind: MetricsFile, content: &str) -> Result<(), AppError> {
    let bad = |why: &str| AppError::new("INVALID_INPUT", why.to_owned());
    if content.len() > MAX_FILE_BYTES {
        return Err(bad("This file is too large."));
    }
    match kind {
        MetricsFile::Beta => {
            let doc: Value =
                serde_json::from_str(content).map_err(|_| bad("This isn't a metrics export."))?;
            if doc.get("format").and_then(Value::as_str) != Some("decay-beta-metrics") {
                return Err(bad("This isn't a metrics export."));
            }
        }
        MetricsFile::Csv => {
            if !content.starts_with("day,activeMinutes,") {
                return Err(bad("This isn't a metrics export."));
            }
        }
    }
    Ok(())
}

/// Saves the beta JSON or the daily CSV where the user picks (exactly the text the page
/// showed). `null` when the dialog is cancelled.
#[tauri::command]
pub async fn save_metrics_file(
    app: AppHandle,
    window: Window,
    kind: MetricsFile,
    content: String,
) -> Res<Option<String>> {
    checked(kind, &content)?;
    let today = jiff::Zoned::now().date().to_string();
    let (name, filter, ext) = match kind {
        MetricsFile::Beta => (format!("decay-beta-metrics-{today}.json"), "JSON", "json"),
        MetricsFile::Csv => (format!("decay-metrics-{today}.csv"), "Spreadsheet (CSV)", "csv"),
    };
    let dialog = app
        .dialog()
        .file()
        .set_title("Save metrics")
        .set_file_name(name)
        .add_filter(filter, &[ext])
        .set_parent(&window);
    blocking(app, move |_| {
        let Some(picked) = dialog.blocking_save_file() else { return Ok(None) };
        let path = picked.into_path().map_err(|e| AppError::new("INVALID_INPUT", e.to_string()))?;
        crate::transfer::write_atomically(&path, content.as_bytes())?;
        Ok(Some(path.display().to_string()))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_metrics_files_are_saved() {
        assert!(
            checked(MetricsFile::Beta, r#"{"format":"decay-beta-metrics","version":2}"#).is_ok()
        );
        assert!(checked(MetricsFile::Beta, r#"{"format":"decay-archive"}"#).is_err());
        assert!(checked(MetricsFile::Beta, "not json").is_err());
        assert!(checked(MetricsFile::Csv, "day,activeMinutes,active\n").is_ok());
        assert!(checked(MetricsFile::Csv, "title,url\n").is_err());
        let big = format!("day,activeMinutes,{}", "x".repeat(MAX_FILE_BYTES));
        assert!(checked(MetricsFile::Csv, &big).is_err());
    }
}
