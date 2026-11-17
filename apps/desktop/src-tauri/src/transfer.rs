//! Export and import (P2.5, PRD F20, API Design §2.6, Designs/09c "Export & import").
//!
//! The file is always picked in a native dialog opened here, never named by the page: a
//! window can only ask for "an export as JSON", not write or read a path of its choosing.
//! Exports are written next to the target (owner-only on Unix: the archive is stored
//! encrypted, the export isn't) and renamed into place, so there's never a half-written
//! file; imports read at most [`MAX_IMPORT_BYTES`]. The engine does the import (format
//! check, backup first, never-recorded sites, see `decay_engine::transfer`).
//!
//! When a file's "never record" rules would delete stored items, `import_archive`
//! imports nothing and answers `{ confirm: { token, purge, fileName } }`; the file waits
//! here until `confirm_import(token, proceed)`, as adding such a rule by hand asks first.

#![allow(clippy::needless_pass_by_value, reason = "Tauri commands take their arguments by value")]

use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use decay_engine::runtime::now_ms;
use decay_engine::{FileImport, ImportOptions, ImportRefused};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Manager as _, Window};
use tauri_plugin_dialog::DialogExt;

use crate::commands::{Res, blocking};
use crate::core::AppError;
use crate::i18n::{t, tf};

/// Largest file an import reads.
pub const MAX_IMPORT_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    Json,
    Html,
    Csv,
}

impl ExportFormat {
    fn extension(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Html => "html",
            Self::Csv => "csv",
        }
    }

    /// The suggested file name, e.g. `Decay archive 2026-11-14.json`.
    pub fn file_name(self, date: &str) -> String {
        let what = if self == Self::Html { "bookmarks" } else { "archive" };
        tf(
            "file.name",
            &[("what", what.into()), ("date", date.into()), ("ext", self.extension().into())],
        )
    }

    fn filter_name(self) -> String {
        t(match self {
            Self::Json => "file.json",
            Self::Html => "file.html",
            Self::Csv => "file.csv",
        })
    }
}

/// `export_archive.include` (JSON only): rules and settings, both on by default.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Include {
    #[serde(default = "yes")]
    pub rules: bool,
    #[serde(default = "yes")]
    pub settings: bool,
}

const fn yes() -> bool {
    true
}

impl Default for Include {
    fn default() -> Self {
        Self { rules: true, settings: true }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Exported {
    pub path: String,
    pub count: usize,
}

/// A file error, by what went wrong (a full disk isn't a permission problem).
fn io(what: &str, e: &std::io::Error) -> AppError {
    use std::io::ErrorKind;
    let code = match e.kind() {
        ErrorKind::PermissionDenied | ErrorKind::ReadOnlyFilesystem => "PERMISSION_DENIED",
        ErrorKind::NotFound => "NOT_FOUND",
        _ => "IO_ERROR",
    };
    AppError::new(code, tf(what, &[("error", e.to_string().into())]))
}

/// Writes `bytes` to `path` through a temporary file in the same folder, renamed into
/// place: the target is either the old file or the whole new one. Owner-only on Unix.
pub fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    let name = path.file_name().ok_or_else(|| AppError::new("INVALID_INPUT", t("file.noName")))?;
    let tmp = path.with_file_name(format!(".{}.decay-tmp", name.to_string_lossy()));
    let written = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut f = options.open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)?;
        // The rename itself survives a crash once the folder is synced.
        #[cfg(unix)]
        if let Some(dir) = path.parent() {
            std::fs::File::open(dir)?.sync_all()?;
        }
        Ok(())
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(io("file.cantSave", &e));
    }
    Ok(())
}

/// The export's content (runs on the engine thread: one consistent read).
fn build(
    store: &decay_store::Store,
    format: ExportFormat,
    include: Include,
    now: i64,
    tz: &jiff::tz::TimeZone,
) -> Result<(Vec<u8>, usize), AppError> {
    let db = |e: decay_store::StoreError| AppError::db(&e);
    Ok(match format {
        ExportFormat::Json => {
            let (doc, n) = store.export_json(include.rules, include.settings, now).map_err(db)?;
            let bytes = serde_json::to_vec_pretty(&doc)
                .map_err(|e| AppError::new("DB_ERROR", e.to_string()))?;
            (bytes, n)
        }
        ExportFormat::Html => {
            let (html, n) = store.export_bookmarks(tz).map_err(db)?;
            (html.into_bytes(), n)
        }
        ExportFormat::Csv => {
            let (csv, n) = store.export_csv(tz).map_err(db)?;
            // A BOM so spreadsheet apps read the titles as UTF-8.
            let mut bytes = b"\xEF\xBB\xBF".to_vec();
            bytes.extend(csv.into_bytes());
            (bytes, n)
        }
    })
}

/// Exports the archive to a file the user picks. `null` when the dialog is cancelled.
#[tauri::command]
pub async fn export_archive(
    app: AppHandle,
    window: Window,
    format: ExportFormat,
    include: Option<Include>,
) -> Res<Option<Exported>> {
    let include = include.unwrap_or_default();
    let today = jiff::Zoned::now().date().to_string();
    let dialog = app
        .dialog()
        .file()
        .set_title(t("dialog.export"))
        .set_file_name(format.file_name(&today))
        .add_filter(format.filter_name(), &[format.extension()])
        .set_parent(&window);
    blocking(app, move |core| {
        let Some(picked) = dialog.blocking_save_file() else { return Ok(None) };
        let path = picked.into_path().map_err(|e| AppError::new("INVALID_INPUT", e.to_string()))?;
        let tz = jiff::tz::TimeZone::system();
        let (bytes, count) =
            core.ask(move |e, now| (build(e.store(), format, include, now, &tz), Vec::new()))??;
        write_atomically(&path, &bytes)?;
        tracing::info!(?format, count, "archive exported");
        Ok(Some(Exported { path: path.display().to_string(), count }))
    })
    .await
}

/// What's in a picked file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportKind {
    Json,
    Bookmarks,
}

/// Whether `lower` (lowercased HTML) has a bookmark entry: `<dt>` then, after only
/// whitespace, `<a`. A saved web page has links but no such list.
fn has_bookmark_entry(lower: &str) -> bool {
    lower.match_indices("<dt>").any(|(i, _)| lower[i + 4..].trim_start().starts_with("<a"))
}

/// JSON or bookmarks, by content (a renamed file still reads right). A file that's
/// neither by content is JSON only when named `.json` (so a broken export says why); an
/// `.html` page without bookmark entries is refused rather than read for every link.
pub fn import_kind(path: &Path, text: &str) -> Option<ImportKind> {
    let head = text.trim_start_matches('\u{feff}').trim_start();
    if head.starts_with('{') {
        return Some(ImportKind::Json);
    }
    let lower = head.to_ascii_lowercase();
    if lower.starts_with("<!doctype netscape-bookmark-file") || has_bookmark_entry(&lower) {
        return Some(ImportKind::Bookmarks);
    }
    match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("json") => Some(ImportKind::Json),
        _ => None,
    }
}

/// Reads a regular file of at most [`MAX_IMPORT_BYTES`] (counted while reading: the
/// size can change after a check, and a pipe or device has none).
fn read_limited(path: &Path) -> Result<String, AppError> {
    let too_large = || AppError::new("INVALID_INPUT", t("file.tooLarge"));
    let file = std::fs::File::open(path).map_err(|e| io("file.cantOpen", &e))?;
    let meta = file.metadata().map_err(|e| io("file.cantOpen", &e))?;
    if !meta.is_file() {
        return Err(AppError::new("INVALID_INPUT", t("file.pick")));
    }
    if meta.len() > MAX_IMPORT_BYTES {
        return Err(too_large());
    }
    let mut bytes = Vec::new();
    file.take(MAX_IMPORT_BYTES + 1).read_to_end(&mut bytes).map_err(|e| io("file.cantRead", &e))?;
    if bytes.len() as u64 > MAX_IMPORT_BYTES {
        return Err(too_large());
    }
    String::from_utf8(bytes).map_err(|_| AppError::new("INVALID_INPUT", t("file.notText")))
}

pub fn refused(why: &ImportRefused) -> AppError {
    match why {
        ImportRefused::Store(e) => AppError::db(e),
        ImportRefused::NotAnArchive => {
            AppError::new("INVALID_INPUT", t("refused.import.notArchive"))
        }
        ImportRefused::NewerFormat(n) => {
            AppError::new("INVALID_INPUT", tf("refused.import.newer", &[("n", (*n).into())]))
        }
        ImportRefused::NeedsConfirm(n) => {
            AppError::new("INVALID_INPUT", tf("refused.import.confirm", &[("n", (*n).into())]))
        }
    }
}

/// `import_archive` result: what the engine did, and the file's name.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Imported {
    #[serde(flatten)]
    pub result: FileImport,
    pub file_name: String,
    /// A bookmarks file with more links than are read ([`decay_store::MAX_BOOKMARKS`]).
    pub links_capped: bool,
}

/// A file whose rules would delete stored items, waiting for `confirm_import`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NeedsConfirm {
    pub token: u64,
    /// Stored items its "never record" rules would delete.
    pub purge: usize,
    pub file_name: String,
}

/// `import_archive` / `confirm_import`: done, or a confirmation needed first.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ImportOutcome {
    Done(Imported),
    Confirm(NeedsConfirm),
}

/// A parsed file waiting for its confirmation (one at a time: a newer pick replaces it).
struct Waiting {
    token: u64,
    doc: Value,
    purge: usize,
    file_name: String,
    bookmarks: bool,
    links_capped: bool,
}

/// The file waiting for `confirm_import`, if any.
#[derive(Default)]
pub struct PendingImport {
    waiting: Mutex<Option<Waiting>>,
    next_token: std::sync::atomic::AtomicU64,
}

/// A picked file, read and parsed.
struct Parsed {
    doc: Value,
    file_name: String,
    bookmarks: bool,
    links_capped: bool,
}

fn parse_file(core: &crate::core::Core, path: &Path) -> Result<Parsed, AppError> {
    let text = read_limited(path)?;
    let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    match import_kind(path, &text) {
        Some(ImportKind::Json) => {
            let doc = serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}'))
                .map_err(|_| AppError::new("INVALID_INPUT", t("file.notJson")))?;
            Ok(Parsed { doc, file_name, bookmarks: false, links_capped: false })
        }
        Some(ImportKind::Bookmarks) => {
            // Bookmarks reopen in the browser the engine picks (see `import_browser_app`).
            let app_id = core.ask(|e, now| (e.import_browser_app(now), Vec::new()))?;
            let (items, links_capped) = decay_store::bookmarks_to_items(&text, &app_id, now_ms());
            if items.is_empty() {
                return Err(AppError::new("INVALID_INPUT", t("file.noLinks")));
            }
            let doc = serde_json::json!({ "items": items });
            Ok(Parsed { doc, file_name, bookmarks: true, links_capped })
        }
        None => Err(AppError::new("INVALID_INPUT", t("file.which"))),
    }
}

/// Imports `parsed`, allowing `allow_purge` deletions. A file whose rules would delete
/// more is kept in [`PendingImport`] and answered with a confirmation request.
fn run_import(
    core: &crate::core::Core,
    pending: &PendingImport,
    parsed: Parsed,
    allow_purge: usize,
) -> Res<ImportOutcome> {
    let Parsed { doc, file_name, bookmarks, links_capped } = parsed;
    let (purge, doc) = core
        .ask(move |e, _| (e.import_purge_count(&doc).map(|n| (n, doc)), Vec::new()))?
        .map_err(|why| refused(&why))?;
    if purge > allow_purge {
        let token = pending.next_token.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        let confirm = NeedsConfirm { token, purge, file_name: file_name.clone() };
        *pending.waiting.lock().unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some(Waiting { token, doc, purge, file_name, bookmarks, links_capped });
        return Ok(ImportOutcome::Confirm(confirm));
    }
    let options = ImportOptions { bookmarks, allow_purge };
    let result = core.ask(move |e, now| match e.import_from_app(doc, now, options) {
        Ok((r, out)) => (Ok(r), out),
        Err(why) => (Err(refused(&why)), Vec::new()),
    })??;
    tracing::info!(?result, "archive imported");
    Ok(ImportOutcome::Done(Imported { result, file_name, links_capped }))
}

/// Tells the windows what an import changed.
fn announce(app: &AppHandle, outcome: &ImportOutcome) {
    if let ImportOutcome::Done(done) = outcome {
        let r = &done.result.report;
        // Also a file that only turned a default off, or brought rules already here.
        if r.rules_imported + r.rules_existing + r.rules_invalid + r.rules_turned_off > 0 {
            crate::rules::announce(app);
        }
        let _ = tauri::Emitter::emit(app, "archive:changed", ());
    }
}

/// Imports a Decay JSON export or a bookmarks file the user picks. `null` when the
/// dialog is cancelled; `{ confirm }` when its rules would delete stored items.
#[tauri::command]
pub async fn import_archive(app: AppHandle, window: Window) -> Res<Option<ImportOutcome>> {
    let dialog = app
        .dialog()
        .file()
        .set_title(t("dialog.import"))
        .add_filter(t("dialog.importFilter"), &["json", "html", "htm"])
        .set_parent(&window);
    let (handle, emit) = (app.clone(), app.clone());
    let outcome = blocking(app, move |core| {
        let Some(picked) = dialog.blocking_pick_file() else { return Ok(None) };
        let path: PathBuf =
            picked.into_path().map_err(|e| AppError::new("INVALID_INPUT", e.to_string()))?;
        let parsed = parse_file(core, &path)?;
        let pending = handle.state::<PendingImport>();
        // A new pick replaces a file still waiting for its confirmation.
        pending.waiting.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take();
        run_import(core, &pending, parsed, 0).map(Some)
    })
    .await?;
    if let Some(outcome) = &outcome {
        announce(&emit, outcome);
    }
    Ok(outcome)
}

/// Imports the file waiting for confirmation (`proceed`), or drops it. `null` when
/// dropped, or when `token` isn't the waiting file's (nothing is imported then).
#[tauri::command]
pub async fn confirm_import(
    app: AppHandle,
    token: u64,
    proceed: bool,
) -> Res<Option<ImportOutcome>> {
    let (handle, emit) = (app.clone(), app.clone());
    let outcome = blocking(app, move |core| {
        let pending = handle.state::<PendingImport>();
        let waiting = {
            let mut slot =
                pending.waiting.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if slot.as_ref().is_none_or(|w| w.token != token) {
                return Ok(None);
            }
            slot.take()
        };
        let Some(w) = waiting.filter(|_| proceed) else { return Ok(None) };
        let parsed = Parsed {
            doc: w.doc,
            file_name: w.file_name,
            bookmarks: w.bookmarks,
            links_capped: w.links_capped,
        };
        // Exactly what was confirmed: a file that would now delete more asks again.
        run_import(core, &pending, parsed, w.purge).map(Some)
    })
    .await?;
    if let Some(outcome) = &outcome {
        announce(&emit, outcome);
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_and_kinds() {
        assert_eq!(ExportFormat::Json.file_name("2026-11-14"), "Decay archive 2026-11-14.json");
        assert_eq!(ExportFormat::Html.file_name("2026-11-14"), "Decay bookmarks 2026-11-14.html");
        let p = Path::new("x.txt");
        assert_eq!(import_kind(p, "\u{feff}  {\"items\": []}"), Some(ImportKind::Json));
        assert_eq!(
            import_kind(p, "<!DOCTYPE NETSCAPE-Bookmark-file-1>\n<DL>"),
            Some(ImportKind::Bookmarks)
        );
        assert_eq!(
            import_kind(Path::new("b.HTM"), "<DL><p>\n  <DT> <A HREF=\"https://x/\">x</A>"),
            Some(ImportKind::Bookmarks),
            "Netscape entries without the doctype"
        );
        // A saved web page is not a bookmarks file, whatever its name.
        let page = "<html><body><a href=\"https://a/\">a</a><dl><dt>Term</dt></dl></body></html>";
        assert_eq!(import_kind(Path::new("page.html"), page), None);
        assert_eq!(import_kind(Path::new("a.json"), "oops"), Some(ImportKind::Json));
        assert_eq!(import_kind(p, "hello"), None);
    }

    #[test]
    fn writes_replace_the_target_whole() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("Decay archive.json");
        std::fs::write(&target, "old").unwrap();
        write_atomically(&target, b"new").unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new");
        let names: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(names.len(), 1, "no temporary file left");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&target).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "owner only");
        }
        let missing = dir.path().join("no-such-folder").join("x.json");
        assert_eq!(write_atomically(&missing, b"x").unwrap_err().code, "NOT_FOUND");
    }

    #[test]
    fn large_or_binary_files_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("b.json");
        std::fs::write(&bin, [0xff, 0xfe, 0x00]).unwrap();
        assert_eq!(read_limited(&bin).unwrap_err().code, "INVALID_INPUT");
        let big = dir.path().join("big.json");
        std::fs::File::create(&big).unwrap().set_len(MAX_IMPORT_BYTES + 1).unwrap();
        assert!(read_limited(&big).unwrap_err().message.contains("too large"));
        assert_eq!(read_limited(dir.path()).unwrap_err().code, "INVALID_INPUT", "a folder");
        let bom = dir.path().join("bom.json");
        std::fs::write(&bom, "\u{feff}{\"items\": []}").unwrap();
        assert!(read_limited(&bom).unwrap().starts_with('\u{feff}'));
    }

    #[test]
    fn exports_build_from_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let paths = decay_store::StorePaths::in_dir(dir.path());
        let (store, _) =
            decay_store::Store::open(paths, decay_store::DbKey::generate().unwrap(), 1_000)
                .unwrap();
        let tz = jiff::tz::TimeZone::UTC;
        let (json, n) = build(&store, ExportFormat::Json, Include::default(), 1_000, &tz).unwrap();
        assert_eq!(n, 0);
        let doc: Value = serde_json::from_slice(&json).unwrap();
        assert_eq!(doc["format"], "decay-archive");
        assert!(doc["rules"].is_array(), "rules by default");
        let (csv, _) = build(&store, ExportFormat::Csv, Include::default(), 1_000, &tz).unwrap();
        assert!(csv.starts_with(b"\xEF\xBB\xBFtitle,url"));
        let (html, _) = build(&store, ExportFormat::Html, Include::default(), 1_000, &tz).unwrap();
        assert!(html.starts_with(b"<!DOCTYPE NETSCAPE-Bookmark-file-1>"));
    }
}
