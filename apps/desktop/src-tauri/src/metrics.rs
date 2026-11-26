//! Insights (P2.7, PRD F34, Designs/14): on-device metrics for the Library's Insights
//! page, the searches the panel reports, and saving the CSV file the page builds.

#![allow(clippy::needless_pass_by_value, reason = "Tauri commands take their arguments by value")]

use decay_engine::MetricsReport;
use serde::Deserialize;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MetricsFile {
    /// Daily rows.
    Csv,
}

/// Checks a file the page asks to save is what it says: the daily CSV (its header row).
fn checked(kind: MetricsFile, content: &str) -> Result<(), AppError> {
    let bad = |key: &str| AppError::new("INVALID_INPUT", crate::i18n::t(key));
    if content.len() > MAX_FILE_BYTES {
        return Err(bad("metrics.tooLarge"));
    }
    match kind {
        MetricsFile::Csv => {
            if !content.starts_with("day,activeMinutes,") {
                return Err(bad("metrics.notExport"));
            }
        }
    }
    Ok(())
}

/// Saves the daily CSV where the user picks (exactly the text the page built). `null` when the dialog is cancelled.
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
        MetricsFile::Csv => {
            (format!("decay-metrics-{today}.csv"), crate::i18n::t("file.csv"), "csv")
        }
    };
    let dialog = app
        .dialog()
        .file()
        .set_title(crate::i18n::t("dialog.metrics"))
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
        assert!(checked(MetricsFile::Csv, "day,activeMinutes,active\n").is_ok());
        assert!(checked(MetricsFile::Csv, "title,url\n").is_err());
        let big = format!("day,activeMinutes,{}", "x".repeat(MAX_FILE_BYTES));
        assert!(checked(MetricsFile::Csv, &big).is_err());
    }

    #[test]
    fn the_metrics_report_serializes() {
        use crate::testing::{NOW, engine};
        let (e, _dir) = engine();
        e.record_search(NOW).unwrap();
        let report = e.metrics(NOW - 86_400_000, NOW).unwrap();
        assert!(serde_json::to_value(&report).is_ok());
    }
}
