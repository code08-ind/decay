//! Diagnostics (P2.6, PRD F29, Software Design §8 "Log leakage").
//!
//! - **Logs**: `…/Decay/logs/decay.<UTC date>.log`, a new file each UTC day; files older
//!   than 7 days (and all but the newest 7) are removed at start. Written as they happen
//!   (nothing lost at exit or on a crash), at most 64 MB a run. Every line is scrubbed
//!   before it reaches the file ([`redact`]); what's logged is chosen not to carry content
//!   in the first place (payload errors name fields, not values; the extension's errors
//!   their code). Debug builds also log to stderr, unscrubbed.
//! - **Export diagnostics**: a zip the user saves in a native dialog —
//!   `diagnostics.json` (versions, OS, the engine's state, the known settings, counts from
//!   the store; no addresses, titles, sites, rule patterns or ids) and the log files,
//!   scrubbed again.
//! - **Crash reports** (opt-in, setting `crash_reports`, off by default; only in builds
//!   made with a valid `DECAY_SENTRY_DSN`): Sentry starts once, on the main thread before
//!   any other, so a panic on any thread is caught; while the setting is off every event
//!   is dropped before sending (nothing goes out). A report carries the panic, its stack
//!   and the last 50 log lines (from every thread), all scrubbed; no machine name, user
//!   or request data.
//! - Errors from the windows' JavaScript are logged (scrubbed, at most 50 per window a
//!   run).

#![allow(clippy::needless_pass_by_value, reason = "Tauri commands take their arguments by value")]

use std::collections::{HashMap, VecDeque};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use serde::Serialize;
use serde_json::{Value, json};
use tauri::{AppHandle, Manager as _, Window};
use tauri_plugin_dialog::DialogExt;
use tracing_subscriber::Layer as _;
use tracing_subscriber::fmt::MakeWriter as _;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

use crate::commands::{Res, blocking};
use crate::core::{AppError, Core};
use crate::i18n::{t, tf};

/// Log files kept: at most this many, none older than this many days.
const KEPT_LOGS: usize = 7;
const KEPT_DAYS: u64 = 7;
/// Most a run writes to the log (a looping warning mustn't fill the disk).
const MAX_LOG_BYTES_PER_RUN: u64 = 64 * 1024 * 1024;
/// At most this much of each log file goes into a diagnostics export (its end).
const MAX_LOG_EXPORT: u64 = 8 * 1024 * 1024;
/// JavaScript errors logged per window a run.
const MAX_UI_ERRORS: u32 = 50;
/// Log lines a crash report carries.
const BREADCRUMBS: usize = 50;

/// Where crash reports go: set when the release is built. `None` when unset or not a
/// valid DSN (never Sentry's own `SENTRY_DSN` from the environment).
fn sentry_dsn() -> Option<sentry::types::Dsn> {
    option_env!("DECAY_SENTRY_DSN").and_then(|d| d.trim().parse().ok())
}

/// The logs folder.
pub fn logs_dir(support: &Path) -> PathBuf {
    support.join("logs")
}

static HOME: OnceLock<Option<String>> = OnceLock::new();

fn home() -> Option<&'static str> {
    HOME.get_or_init(|| {
        let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        std::env::var(var).ok().filter(|h| h.len() > 1)
    })
    .as_deref()
}

/// A log line without personal data:
/// - quoted text (`"…"`, `'…'`, `` `…` ``, `“…”`, `‘…’`) → `"<text>"` (backticks keep a
///   bare identifier, a field name);
/// - web and other addresses → `<url>`, also `host.tld/path` and `www.…` without a
///   scheme (the app's own `localhost` / `tauri://` addresses stay, for stack traces);
/// - e-mail addresses → `<email>`;
/// - the home folder → `~` (files under it other than `~/Library/…` → `~/<path>`), any
///   other user's folder name → `<user>`.
pub fn redact(line: &str) -> String {
    let s = quoted(line);
    let s = words(&s);
    let s = home_paths(&s);
    user_folders(&s)
}

/// Quote pairs: opening, closing.
const QUOTES: [(char, char); 5] =
    [('"', '"'), ('`', '`'), ('\u{201c}', '\u{201d}'), ('\u{2018}', '\u{2019}'), ('\'', '\'')];

/// Whether `c` before a `'` makes it an opening quote (not an apostrophe: `can't`).
fn opens_single(prev: Option<char>) -> bool {
    prev.is_none_or(|p| p.is_whitespace() || matches!(p, '(' | '[' | '=' | ':' | ','))
}

/// Whether `c` after a `'` makes it a closing quote.
fn closes_single(next: Option<char>) -> bool {
    next.is_none_or(|n| n.is_whitespace() || matches!(n, ')' | ']' | ',' | '.' | ';' | ':'))
}

fn identifier(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 40
        && s.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Quoted text → `"<text>"` (see [`redact`]).
fn quoted(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(line.len());
    let mut at = 0;
    while at < chars.len() {
        let ch = chars[at];
        let Some(&(open, close)) = QUOTES.iter().find(|(o, _)| *o == ch) else {
            out.push(ch);
            at += 1;
            continue;
        };
        if open == '\'' && !opens_single(at.checked_sub(1).map(|p| chars[p])) {
            out.push(ch);
            at += 1;
            continue;
        }
        // The closing quote (a backslash escapes one).
        let mut pos = at + 1;
        let mut end = None;
        while pos < chars.len() {
            match chars[pos] {
                '\\' if open != '`' => pos += 1,
                d if d == close && (open != '\'' || closes_single(chars.get(pos + 1).copied())) => {
                    end = Some(pos);
                    break;
                }
                _ => {}
            }
            pos += 1;
        }
        let Some(end) = end else {
            // Unclosed: the rest of the line is the quote's (an apostrophe stays).
            if open == '\'' {
                out.push(ch);
                at += 1;
                continue;
            }
            out.push_str("\"<text>");
            return out;
        };
        let inner: String = chars[at + 1..end].iter().collect();
        if inner.is_empty() || (open == '`' && identifier(&inner)) {
            out.push(open);
            out.push_str(&inner);
            out.push(close);
        } else {
            out.push_str("\"<text>\"");
        }
        at = end + 1;
    }
    out
}

/// The app's own addresses (its pages and IPC), kept: they say where in the app's code
/// an error happened.
fn own_address(after_scheme: &str) -> bool {
    let host = after_scheme.split(['/', '?', '#']).next().unwrap_or_default();
    let host = host
        .rsplit_once(':')
        .filter(|(_, p)| p.chars().all(|c| c.is_ascii_digit()))
        .map_or(host, |(h, _)| h);
    matches!(host, "localhost" | "tauri.localhost" | "ipc.localhost" | "127.0.0.1")
}

/// `host.tld/…` or `www.…` (an address without its scheme).
fn bare_address(word: &str) -> bool {
    if word.len() > 4 && word.get(..4).is_some_and(|w| w.eq_ignore_ascii_case("www.")) {
        return true;
    }
    let Some(cut) = word.find(['/', '?']) else { return false };
    let host = &word[..cut];
    let labels: Vec<&str> = host.split('.').collect();
    labels.len() >= 2
        && labels
            .iter()
            .all(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
        && labels.last().is_some_and(|t| t.len() >= 2 && t.chars().all(|c| c.is_ascii_alphabetic()))
}

/// Addresses and e-mail addresses, word by word.
fn words(s: &str) -> String {
    const SCHEMES: [&str; 8] =
        ["javascript:", "data:", "mailto:", "blob:", "about:", "file:", "tel:", "sms:"];
    let stop = |c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '`' | ',');
    let punct_len =
        |w: &str| w.len() - w.trim_end_matches(['.', ',', ';', ':', ')', ']', '!']).len();
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while !rest.is_empty() {
        let start = rest.find(|c: char| !stop(c)).unwrap_or(rest.len());
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        let end = rest.find(stop).unwrap_or(rest.len());
        let token = &rest[..end];
        rest = &rest[end..];
        // What follows a leading `key=`, `(` or `[` is the candidate (one before any
        // `/ ? : @`, so an address's own query `?a=b` doesn't count).
        let body = token.find(['/', '?', ':', '@']).unwrap_or(token.len());
        let lead = token[..body].rfind(['=', '(', '[']).map_or(0, |i| i + 1);
        let (prefix, word) = token.split_at(lead);
        let cut = word.len() - punct_len(word);
        let (word, tail) = word.split_at(cut);
        let lower = word.to_ascii_lowercase();
        let replaced = if let Some(i) = word.find("://") {
            let scheme_start = word[..i]
                .rfind(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-')))
                .map_or(0, |p| p + 1);
            (!own_address(&word[i + 3..])).then(|| format!("{}<url>", &word[..scheme_start]))
        } else if SCHEMES.iter().any(|p| lower.starts_with(p)) || bare_address(word) {
            Some("<url>".to_owned())
        } else if word.split_once('@').is_some_and(|(a, b)| {
            !a.is_empty()
                && b.contains('.')
                && b.rsplit('.')
                    .next()
                    .is_some_and(|t| t.len() >= 2 && t.chars().all(|c| c.is_ascii_alphabetic()))
        }) {
            Some("<email>".to_owned())
        } else {
            None
        };
        out.push_str(prefix);
        match replaced {
            Some(r) => out.push_str(&r),
            None => out.push_str(word),
        }
        out.push_str(tail);
    }
    out
}

/// The home folder → `~`; files under it, other than the app's own data in
/// `~/Library/`, → `~/<path>`.
fn home_paths(s: &str) -> String {
    let Some(h) = home() else { return s.to_owned() };
    let mut s = s.replace(h, "~");
    if h.contains('\\') {
        s = s.replace(&h.replace('\\', "\\\\"), "~");
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s.as_str();
    while let Some(i) = rest.find("~/").or_else(|| rest.find("~\\")) {
        out.push_str(&rest[..i + 2]);
        rest = &rest[i + 2..];
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let path = &rest[..end];
        if !(path.starts_with("Library/") || path.starts_with("AppData\\") || path.starts_with('<'))
        {
            out.push_str("<path>");
            rest = &rest[end..];
        }
    }
    out.push_str(rest);
    out
}

/// `/Users/<name>/…`, `C:\Users\<name>\…` (also `\\`-escaped, any case) and
/// `/home/<name>/…` → `<user>`.
fn user_folders(s: &str) -> String {
    let lower = s.to_ascii_lowercase();
    let mut out = String::with_capacity(s.len());
    let mut at = 0;
    loop {
        let found = ["/users/", "\\users\\", "/home/"]
            .iter()
            .filter_map(|p| lower[at..].find(p).map(|i| (at + i, p.len())))
            .min();
        let Some((i, len)) = found else { break };
        let mut name_start = i + len;
        while s[name_start..].starts_with('\\') {
            name_start += 1;
        }
        out.push_str(&s[at..name_start]);
        let rest = &s[name_start..];
        // The name runs to the next separator (it may hold spaces), or to a space or
        // quote when the path ends there.
        let end = rest
            .find(['/', '\\'])
            .filter(|e| !rest[..*e].contains(['"', '\n']))
            .unwrap_or_else(|| rest.find([' ', '"', '\'', '\n']).unwrap_or(rest.len()));
        let name = &rest[..end];
        if name.is_empty() || matches!(name, "Shared" | "Public" | "<user>") {
            at = name_start;
        } else {
            out.push_str("<user>");
            at = name_start + end;
        }
    }
    out.push_str(&s[at..]);
    out
}

/// The last log lines (scrubbed), for a crash report.
static RECENT: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());

fn remember(text: &str) {
    let mut recent = RECENT.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        if recent.len() == BREADCRUMBS {
            recent.pop_front();
        }
        recent.push_back(line.chars().take(1000).collect());
    }
}

/// Bytes written to the log this run.
static WRITTEN: AtomicU64 = AtomicU64::new(0);

/// A writer that scrubs what it's given (one formatted event per write), remembers it
/// for a crash report and stops at [`MAX_LOG_BYTES_PER_RUN`].
struct Redacting<W>(W);

impl<W: std::io::Write> std::io::Write for Redacting<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let text = redact(&String::from_utf8_lossy(buf));
        remember(&text);
        let before = WRITTEN.fetch_add(text.len() as u64, Ordering::Relaxed);
        if before < MAX_LOG_BYTES_PER_RUN {
            self.0.write_all(text.as_bytes())?;
            if before + text.len() as u64 >= MAX_LOG_BYTES_PER_RUN {
                self.0.write_all(
                    b"[the log limit for this run is reached: nothing more is written]\n",
                )?;
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

/// The log file (written synchronously: a line is on disk before the next one).
static LOG_FILE: OnceLock<tracing_appender::rolling::RollingFileAppender> = OnceLock::new();

/// Removes Decay's log files older than [`KEPT_DAYS`].
fn prune_old_logs(dir: &Path) {
    let max_age = std::time::Duration::from_secs(KEPT_DAYS * 86_400);
    for path in log_files(dir) {
        let old = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > max_age);
        if old {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Starts logging (see the module docs) and logs panics. `support` is `None` without a
/// home folder: stderr only.
pub fn init_logging(support: Option<&Path>) {
    let filter =
        || tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());
    let file = support.and_then(|s| {
        let dir = logs_dir(s);
        std::fs::create_dir_all(&dir).ok()?;
        prune_old_logs(&dir);
        let appender = tracing_appender::rolling::Builder::new()
            .rotation(tracing_appender::rolling::Rotation::DAILY)
            .filename_prefix("decay")
            .filename_suffix("log")
            .max_log_files(KEPT_LOGS)
            .build(&dir)
            .ok()?;
        let file = LOG_FILE.get_or_init(|| appender);
        Some(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(move || Redacting(file.make_writer()))
                .with_filter(filter()),
        )
    });
    let stderr = cfg!(debug_assertions).then(|| {
        tracing_subscriber::fmt::layer().with_writer(std::io::stderr).with_filter(filter())
    });
    let _ = tracing_subscriber::registry().with(file).with(stderr).try_init();

    let next = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let at = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
        let what = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_owned())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_default();
        tracing::error!(at = %at, "panic: {what}");
        next(info);
    }));
}

/// Crash reports are on (the setting).
static CRASH_REPORTS: AtomicBool = AtomicBool::new(false);
/// Sentry's client, started once (see the module docs).
static SENTRY: OnceLock<sentry::ClientInitGuard> = OnceLock::new();

/// A Sentry event without anything that could identify the user or what they browse,
/// with the last log lines as breadcrumbs.
fn scrub_event(mut event: sentry::protocol::Event<'static>) -> sentry::protocol::Event<'static> {
    event.server_name = None;
    event.user = None;
    event.request = None;
    event.extra.clear();
    event.tags.clear();
    event.message = event.message.map(|m| redact(&m));
    if let Some(entry) = event.logentry.as_mut() {
        entry.message = redact(&entry.message);
        entry.params.clear();
    }
    let frames = |st: &mut Option<sentry::protocol::Stacktrace>| {
        for f in st.iter_mut().flat_map(|st| st.frames.iter_mut()) {
            f.abs_path = f.abs_path.as_deref().map(redact);
            f.filename = f.filename.as_deref().map(redact);
            f.vars.clear();
            f.context_line = None;
            f.pre_context.clear();
            f.post_context.clear();
        }
    };
    for e in &mut event.exception.values {
        e.value = e.value.as_deref().map(redact);
        frames(&mut e.stacktrace);
    }
    for t in &mut event.threads.values {
        frames(&mut t.stacktrace);
    }
    let recent = RECENT.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
    event.breadcrumbs.values = recent
        .into_iter()
        .map(|line| sentry::protocol::Breadcrumb {
            category: Some("log".into()),
            message: Some(redact(&line)),
            ..Default::default()
        })
        .collect();
    event
}

/// Starts Sentry when this build has a DSN (see the module docs). Call once, first
/// thing on the main thread.
pub fn init_crash_reports() {
    let Some(dsn) = sentry_dsn() else { return };
    let mut options = sentry::ClientOptions::new();
    options.dsn = Some(dsn);
    options.release = Some(format!("decay@{}", env!("CARGO_PKG_VERSION")).into());
    options.environment = Some(if cfg!(debug_assertions) { "debug" } else { "release" }.into());
    options.send_default_pii = false;
    options.server_name = None;
    options.attach_stacktrace = false;
    options.auto_session_tracking = false;
    // Breadcrumbs are the shared log lines, added when an event is sent.
    options.max_breadcrumbs = 0;
    options.before_breadcrumb = Some(std::sync::Arc::new(|_| None));
    options.before_send =
        Some(std::sync::Arc::new(|e| CRASH_REPORTS.load(Ordering::SeqCst).then(|| scrub_event(e))));
    let _ = SENTRY.set(sentry::init(options));
}

/// Turns crash reports on or off (no effect in a build without a DSN).
pub fn set_crash_reports(on: bool) {
    let was = CRASH_REPORTS.swap(on && sentry_dsn().is_some(), Ordering::SeqCst);
    if was != on && sentry_dsn().is_some() {
        tracing::info!("crash reports {}", if on { "on" } else { "off" });
    }
}

/// Applies the settings that concern diagnostics.
pub fn apply_settings(settings: &serde_json::Map<String, Value>) {
    set_crash_reports(settings.get("crash_reports").and_then(Value::as_bool).unwrap_or(false));
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsInfo {
    /// This build can send crash reports (it has somewhere to send them).
    pub crash_reports_available: bool,
    /// The logs folder, the home folder shown as `~`.
    pub logs_dir: String,
}

/// What the Diagnostics settings show.
#[tauri::command]
pub fn get_diagnostics_info(shell: tauri::State<'_, crate::shell::Shell>) -> DiagnosticsInfo {
    let dir = logs_dir(&shell.support).display().to_string();
    let dir = home().map_or(dir.clone(), |h| dir.replace(h, "~"));
    DiagnosticsInfo { crash_reports_available: sentry_dsn().is_some(), logs_dir: dir }
}

/// Opens the logs folder in Finder / Explorer.
#[tauri::command]
pub async fn open_logs_folder(app: AppHandle) -> Res<()> {
    let dir = logs_dir(&app.state::<crate::shell::Shell>().support);
    tauri::async_runtime::spawn_blocking(move || {
        std::fs::create_dir_all(&dir).map_err(|e| {
            AppError::new("IO_ERROR", tf("error.logs", &[("error", e.to_string().into())]))
        })?;
        let opener = if cfg!(windows) { "explorer" } else { "/usr/bin/open" };
        std::process::Command::new(opener).arg(&dir).status().map(|_| ()).map_err(|e| {
            AppError::new("IO_ERROR", tf("error.logs", &[("error", e.to_string().into())]))
        })
    })
    .await
    .map_err(|e| AppError::new("INTERNAL", e.to_string()))?
}

/// JavaScript errors logged per window this run.
static UI_ERRORS: Mutex<Option<HashMap<String, u32>>> = Mutex::new(None);

/// Whether another error from `window` is logged; the first one over the limit says so.
fn ui_error_allowed(window: &str) -> bool {
    let mut slot = UI_ERRORS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let n = slot.get_or_insert_with(HashMap::new).entry(window.to_owned()).or_insert(0);
    *n = n.saturating_add(1);
    if *n == MAX_UI_ERRORS + 1 {
        tracing::warn!(window = %window, "more errors in this window are not logged until Decay restarts");
    }
    *n <= MAX_UI_ERRORS
}

/// Logs an error from a window's JavaScript (scrubbed like every line).
#[tauri::command]
pub fn log_ui_error(window: Window, message: String, stack: Option<String>) {
    let label = window.label();
    if !ui_error_allowed(label) {
        return;
    }
    let message: String = message.chars().take(2000).collect();
    let stack: String = stack.unwrap_or_default().chars().take(4000).collect();
    tracing::error!(window = %label, "ui: {message}\n{stack}");
}

/// The OS name and version (`macOS 15.1`).
pub(crate) fn os_version() -> String {
    #[cfg(target_os = "macos")]
    {
        let v = std::process::Command::new("/usr/bin/sw_vers")
            .arg("-productVersion")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|v| v.trim().to_owned())
            .unwrap_or_default();
        format!("macOS {v}")
    }
    #[cfg(not(target_os = "macos"))]
    {
        std::env::consts::OS.to_owned()
    }
}

/// `diagnostics.json` (see the module docs).
pub fn report(
    status: Option<&decay_engine::EngineStatus>,
    settings: &serde_json::Map<String, Value>,
    store: &decay_store::StoreDiagnostics,
    logs: &[String],
    now: i64,
) -> Value {
    let engine = status.map(|s| {
        json!({
            "mode": s.mode,
            "onHold": s.hold_until.is_some_and(|h| h > now),
            "sweeping": s.sweeping,
            "inMeeting": s.in_meeting,
            "accessibility": s.accessibility,
            "liveTabs": s.live_tabs,
            "staleCount": s.stale_count,
            "browsers": s.browsers.iter().map(|b| json!({ "browser": b.browser, "tabs": b.tabs })).collect::<Vec<_>>(),
        })
    });
    json!({
        "format": "decay-diagnostics",
        "version": 1,
        "createdAt": now,
        "app": {
            "version": env!("CARGO_PKG_VERSION"),
            "build": if cfg!(debug_assertions) { "debug" } else { "release" },
            "os": os_version(),
            "arch": std::env::consts::ARCH,
            "crashReportsAvailable": sentry_dsn().is_some(),
            "crashReportsOn": CRASH_REPORTS.load(Ordering::SeqCst),
        },
        "engine": engine,
        // The known settings only: a key added later isn't exported unless it's known.
        "settings": settings
            .iter()
            .filter(|(k, _)| decay_store::default_setting(k).is_some())
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect::<serde_json::Map<_, _>>(),
        "store": store,
        "logs": logs,
    })
}

/// The log files, oldest first.
fn log_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| {
                    p.is_file()
                        && p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                            n.starts_with("decay")
                                && Path::new(n).extension().is_some_and(|e| e == "log")
                        })
                })
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    files
}

/// The end of a log file (at most [`MAX_LOG_EXPORT`]), scrubbed again line by line.
fn log_tail(path: &Path) -> std::io::Result<String> {
    use std::io::{Read as _, Seek as _, SeekFrom};
    let mut f = std::fs::File::open(path)?;
    let len = f.metadata()?.len();
    if len > MAX_LOG_EXPORT {
        f.seek(SeekFrom::Start(len - MAX_LOG_EXPORT))?;
    }
    let mut bytes = Vec::new();
    f.take(MAX_LOG_EXPORT).read_to_end(&mut bytes)?;
    let text = String::from_utf8_lossy(&bytes);
    // A cut first line is dropped.
    let text =
        if len > MAX_LOG_EXPORT { text.split_once('\n').map_or("", |(_, r)| r) } else { &text };
    Ok(text.lines().map(redact).collect::<Vec<_>>().join("\n"))
}

/// The diagnostics zip: `diagnostics.json` and `logs/…`.
pub fn build_zip(report: &Value, logs: &[PathBuf]) -> Result<Vec<u8>, AppError> {
    let fail = |e: &dyn std::fmt::Display| {
        AppError::new("IO_ERROR", tf("error.zip", &[("error", e.to_string().into())]))
    };
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o600);
    zip.start_file("diagnostics.json", options).map_err(|e| fail(&e))?;
    let json = serde_json::to_vec_pretty(report).map_err(|e| fail(&e))?;
    zip.write_all(&json).map_err(|e| fail(&e))?;
    for path in logs {
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
        let Ok(text) = log_tail(path) else { continue };
        zip.start_file(format!("logs/{name}"), options).map_err(|e| fail(&e))?;
        zip.write_all(text.as_bytes()).map_err(|e| fail(&e))?;
    }
    Ok(zip.finish().map_err(|e| fail(&e))?.into_inner())
}

/// Saves a diagnostics zip where the user picks. `null` when the dialog is cancelled.
#[tauri::command]
pub async fn export_diagnostics(app: AppHandle, window: Window) -> Res<Option<String>> {
    let today = jiff::Zoned::now().date().to_string();
    let dialog = app
        .dialog()
        .file()
        .set_title(t("dialog.diagnostics"))
        .set_file_name(tf("dialog.diagnosticsFile", &[("date", today.clone().into())]))
        .add_filter(t("dialog.zip"), &["zip"])
        .set_parent(&window);
    let support = app.state::<crate::shell::Shell>().support.clone();
    blocking(app, move |core: &Core| {
        let Some(picked) = dialog.blocking_save_file() else { return Ok(None) };
        let path = picked.into_path().map_err(|e| AppError::new("INVALID_INPUT", e.to_string()))?;
        let logs = log_files(&logs_dir(&support));
        let names: Vec<String> =
            logs.iter().filter_map(|p| p.file_name()?.to_str().map(str::to_owned)).collect();
        let status = core.status();
        let report = core.ask(move |e, now| {
            let store = e.store();
            let read = store
                .settings_snapshot()
                .and_then(|s| Ok((s, store.diagnostics(now)?)))
                .map(|(settings, counts)| report(status.as_ref(), &settings, &counts, &names, now))
                .map_err(|err| AppError::db(&err));
            (read, Vec::new())
        })??;
        let bytes = build_zip(&report, &logs)?;
        crate::transfer::write_atomically(&path, &bytes)?;
        tracing::info!(files = logs.len(), "diagnostics exported");
        Ok(Some(path.display().to_string()))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every line from the review and QA that leaked, with what must not survive.
    const LEAKS: &[(&str, &[&str])] = &[
        ("closed https://bank.example/home/acct/12345?token=abc", &["bank", "12345", "token"]),
        ("url=https://github.com/Users/ann/private-repo/issues/7", &["github", "ann", "private"]),
        ("file:///Users/ann/Documents/Divorce%20papers.pdf", &["ann", "Divorce", "Documents"]),
        (r#"invalid type: string "Gmail", expected u64"#, &["Gmail"]),
        (r#"invalid type: string "三菱UFJ銀行ログイン""#, &["三菱", "UFJ"]),
        ("unknown variant `www.bank.example/acct`, expected one of", &["bank"]),
        (r#"ErrorPayload { code: X, message: "Inbox" }"#, &["Inbox"]),
        ("said 'My bank statement' then", &["bank", "statement"]),
        ("opened www.example.com/path?q=secret", &["example", "secret"]),
        ("site github.com/ann-secret-org", &["ann-secret"]),
        (r#"TypeError: "bank.example/x" cannot be parsed as a URL."#, &["bank"]),
        ("go chase.com/login?user=ann now", &["chase", "ann"]),
        ("mail ann.smith@gmail.com.", &["ann.smith", "gmail"]),
        (
            "smart \u{201c}Ann has results\u{201d} and \u{2018}x y\u{2019}",
            &["Ann", "results", "x y"],
        ),
        ("`My Project` in backticks", &["Project"]),
        ("in /Users/Ann Smith/Library/x", &["Ann", "Smith"]),
        (r"in C:\Users\Ann Smith\AppData\x", &["Ann", "Smith"]),
        (r#"path: "C:\\Users\\Ann\\Documents\\x.pdf""#, &["Ann", "Documents"]),
        (r"lower c:\users\ann\x", &["ann"]),
        ("call tel:+15551234567 or javascript:alert(1)", &["555", "alert"]),
        ("unclosed \"title that never ends", &["title", "never"]),
        ("(https://bank.example/x)", &["bank"]),
    ];

    #[test]
    fn nothing_personal_survives() {
        for (line, leaks) in LEAKS {
            let out = redact(line);
            for leak in *leaks {
                assert!(!out.contains(leak), "{leak:?} survived in {out:?} (from {line:?})");
            }
            assert_eq!(redact(&out), out, "scrubbing twice changes nothing: {line}");
        }
    }

    #[test]
    fn what_helps_debugging_stays() {
        for line in [
            "store open conn=3 items=12",
            "browser connected browser=Chrome",
            "panic: index out of bounds at=src/engine.rs:1088",
            "decay@0.1.0 profile:Default metadata: ok",
            "missing field `url` at line 1 column 2",
            "can't open: don't, won't",
            "file decay.2026-11-15.log rotated",
            "ui: boom at tauri://localhost/assets/index-AbC123.js:12:345",
            "dev http://localhost:1420/src/main.tsx:3:1",
            "/Users/Shared/x and /Users/<user>/y",
        ] {
            assert_eq!(redact(line), line, "changed: {line}");
        }
        assert_eq!(redact(r#"a "x \" y" b"#), r#"a "<text>" b"#);
        assert_eq!(redact(r#"kind="" ok"#), r#"kind="" ok"#);
        assert_eq!(redact("url=https://bank.example/x)."), "url=<url>).");
        assert_eq!(redact("ann@x.org"), "<email>");
        assert_eq!(redact("über ünïcode"), "über ünïcode");
        assert_eq!(redact(""), "");
    }

    #[test]
    fn files_under_home_lose_their_names() {
        let Some(h) = home() else { return };
        assert_eq!(redact(&format!("open {h}/Documents/Taxes.pdf ok")), "open ~/<path> ok");
        assert_eq!(redact(&format!("data {h}/Library/Application")), "data ~/Library/Application");
    }

    #[test]
    fn crash_reports_carry_nothing_personal() {
        use sentry::protocol::{Event, Exception, Frame, Stacktrace, User};
        remember("INFO one https://bank.example/x\nINFO two");
        let mut event = Event::new();
        event.server_name = Some("Anns-MacBook".into());
        event.user = Some(User { username: Some("ann".into()), ..Default::default() });
        event.message = Some("closing https://bank.example/x".into());
        event.extra.insert("title".into(), "My bank".into());
        event.tags.insert("site".into(), "bank.example".into());
        event.exception.values.push(Exception {
            ty: "panic".into(),
            value: Some("bad \"My bank statement\"".into()),
            stacktrace: Some(Stacktrace {
                frames: vec![Frame {
                    abs_path: Some("/Users/ann/src/x.rs".into()),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        });
        let e = scrub_event(event);
        assert!(e.breadcrumbs.values.iter().any(|b| b.message.as_deref() == Some("INFO two")));
        let text = serde_json::to_string(&e).unwrap();
        for leak in ["Anns", "\"ann\"", "bank", "statement", "/Users/ann"] {
            assert!(!text.contains(leak), "{leak} in {text}");
        }
    }

    #[test]
    fn the_writer_scrubs_remembers_and_stops_at_the_limit() {
        let mut out = Vec::new();
        Redacting(&mut out).write_all(b"opened https://secret.example/x\n").unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "opened <url>\n");
        assert!(RECENT.lock().unwrap().iter().any(|l| l == "opened <url>"));
        let before = WRITTEN.swap(MAX_LOG_BYTES_PER_RUN - 3, Ordering::SeqCst);
        let mut out = Vec::new();
        Redacting(&mut out).write_all(b"last line\n").unwrap();
        Redacting(&mut out).write_all(b"dropped\n").unwrap();
        WRITTEN.store(before, Ordering::SeqCst);
        let text = String::from_utf8(out).unwrap();
        assert!(
            text.starts_with("last line\n[the log limit") && !text.contains("dropped"),
            "{text}"
        );
    }

    #[test]
    fn each_window_has_its_own_error_budget() {
        let a = (0..60).filter(|_| ui_error_allowed("test-a")).count();
        assert_eq!(a, MAX_UI_ERRORS as usize);
        assert!(ui_error_allowed("test-b"), "another window still logs");
    }

    #[test]
    fn old_logs_are_removed_and_others_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("decay.2026-01-01.log");
        let new = dir.path().join("decay.2026-11-15.log");
        let other = dir.path().join("notes.log");
        for p in [&old, &new, &other] {
            std::fs::write(p, "x").unwrap();
        }
        let long_ago = std::time::SystemTime::now() - std::time::Duration::from_hours(30 * 24);
        std::fs::File::options().write(true).open(&old).unwrap().set_modified(long_ago).unwrap();
        std::fs::File::options().write(true).open(&other).unwrap().set_modified(long_ago).unwrap();
        prune_old_logs(dir.path());
        assert!(!old.exists() && new.exists() && other.exists());
    }

    #[test]
    fn a_long_log_exports_its_end_only() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("decay.2026-11-15.log");
        let line = "INFO é filler line\n";
        let n = usize::try_from(MAX_LOG_EXPORT).unwrap() / line.len() + 10;
        let mut text = line.repeat(n);
        text.push_str("INFO the end\n");
        std::fs::write(&log, &text).unwrap();
        let tail = log_tail(&log).unwrap();
        assert!(tail.ends_with("INFO the end"));
        assert!(tail.len() as u64 <= MAX_LOG_EXPORT);
        assert!(tail.starts_with("INFO é filler"), "the cut first line is dropped");
    }

    #[test]
    fn the_zip_has_the_report_and_scrubbed_logs() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("decay.2026-11-15.log");
        std::fs::write(&log, "INFO opened https://secret.example/x\nINFO ok\n").unwrap();
        let other = dir.path().join("notes.txt");
        std::fs::write(&other, "x").unwrap();
        let files = log_files(dir.path());
        assert_eq!(files, vec![log.clone()], "only Decay's logs");
        let report = json!({"format": "decay-diagnostics"});
        let bytes = build_zip(&report, &files).unwrap();
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut names: Vec<String> = zip.file_names().map(|n| n.unwrap().into_owned()).collect();
        names.sort();
        assert_eq!(names, ["diagnostics.json", "logs/decay.2026-11-15.log"]);
        let mut text = String::new();
        std::io::Read::read_to_string(
            &mut zip.by_name("logs/decay.2026-11-15.log").unwrap(),
            &mut text,
        )
        .unwrap();
        assert_eq!(text, "INFO opened <url>\nINFO ok");
    }

    #[test]
    fn the_report_has_no_content() {
        let status = decay_engine::EngineStatus {
            mode: decay_core::Mode::Shadow,
            hold_until: None,
            stale_count: 2,
            live_tabs: 9,
            browsers: vec![decay_engine::BrowserStatus {
                browser: decay_bridge::protocol::BrowserKind::Chrome,
                profile_id: "person@example.com".into(),
                tabs: 9,
            }],
            sweeping: false,
            undoable_sweep: Some("sweep-secret".into()),
            accessibility: Some(true),
            in_meeting: false,
        };
        let dir = tempfile::tempdir().unwrap();
        let paths = decay_store::StorePaths::in_dir(dir.path());
        let (store, _) =
            decay_store::Store::open(paths, decay_store::DbKey::generate().unwrap(), 1_000)
                .unwrap();
        let counts = store.diagnostics(1_000).unwrap();
        let settings = store.settings_snapshot().unwrap();
        let r = report(Some(&status), &settings, &counts, &["decay.log".into()], 1_000);
        let text = r.to_string();
        assert!(!text.contains("person@") && !text.contains("sweep-secret"), "{text}");
        assert_eq!(r["engine"]["browsers"][0]["tabs"], 9);
        assert_eq!(r["format"], "decay-diagnostics");
    }
}
