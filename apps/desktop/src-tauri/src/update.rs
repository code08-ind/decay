//! Updates (P1.13, PRD F18, API Design §2.x / §7.1): a daily check of the channel's
//! `latest.json` (stable / beta, public releases repo), a background download when
//! "Download updates automatically" is on, then "Restart now" or "Install on quit".
//! Every package is checked against the Ed25519 updater key in `tauri.conf.json` when
//! it's downloaded and again when it's installed (tauri-plugin-updater); a bad signature
//! is an error, never installed.
//!
//! Every decision is made by [`Slot`] (pure, tested); the Tauri glue below only carries
//! out what it returns ([`Next`]). Every change is broadcast as `update:state`, and a
//! newly found version once as `update:available`.

#![allow(clippy::needless_pass_by_value, reason = "Tauri commands take their arguments by value")]

use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::commands::{Res, blocking};
use crate::core::AppError;

/// Release files live in a public repo (the source repo is private).
pub const STABLE_URL: &str =
    "https://github.com/code08-ind/decay-releases/releases/latest/download/latest.json";
/// The `beta` release's manifest: every beta, and every stable release too.
pub const BETA_URL: &str =
    "https://github.com/code08-ind/decay-releases/releases/download/beta/latest.json";

/// First check this long after start-up (let the engine and browsers settle).
const FIRST_CHECK: Duration = Duration::from_secs(60);
/// The checker wakes this often and checks when the last check is a day old (a sleeping
/// Mac doesn't advance a plain 24 h sleep).
const WAKE_EVERY: Duration = Duration::from_hours(1);
const DAY_MS: i64 = 24 * 60 * 60 * 1000;
/// Progress events at most every this many bytes.
const PROGRESS_STEP: u64 = 256 * 1024;

pub fn endpoint(channel: &str) -> &'static str {
    if channel == "beta" { BETA_URL } else { STABLE_URL }
}

/// Automatic checks (start-up, daily, a new channel): release builds, or debug builds
/// with `DECAY_UPDATE_CHECK=1`. "Check now" always checks.
fn automatic_checks() -> bool {
    !cfg!(debug_assertions) || std::env::var_os("DECAY_UPDATE_CHECK").is_some()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    /// Up to date, or not checked yet.
    Idle,
    Checking,
    /// A newer version exists; not downloaded.
    Available,
    Downloading,
    /// Downloaded and verified: installs on "Restart now" or at quit.
    Ready,
    Error,
}

/// What failed (`phase: error`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Failed {
    Check,
    Download,
    Install,
}

/// What the Updates tab shows (`update:state`, `get_update_state`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateState {
    pub current: String,
    pub phase: Phase,
    pub version: Option<String>,
    pub notes: Option<String>,
    /// RFC 3339.
    pub date: Option<String>,
    pub downloaded: u64,
    pub total: Option<u64>,
    /// Last finished check (ms), successful or not.
    pub checked_at: Option<i64>,
    pub error: Option<String>,
    pub failed: Option<Failed>,
    pub install_on_quit: bool,
    /// "Restart now" was asked; it happens as soon as the download is ready.
    pub install_now: bool,
}

impl UpdateState {
    fn new(current: &str) -> Self {
        Self {
            current: current.to_owned(),
            phase: Phase::Idle,
            version: None,
            notes: None,
            date: None,
            downloaded: 0,
            total: None,
            checked_at: None,
            error: None,
            failed: None,
            install_on_quit: false,
            install_now: false,
        }
    }

    fn fail(&mut self, what: Failed, why: String) {
        self.phase = Phase::Error;
        self.failed = Some(what);
        self.error = Some(why);
    }
}

/// What a check found, without the plugin's types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub version: String,
    pub notes: Option<String>,
    pub date: Option<String>,
}

/// What to do after a decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Next {
    Nothing,
    Download,
    Install,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum When {
    Now,
    OnQuit,
}

/// The updater's state and the update itself (`U`: the plugin's `Update`; tests use a
/// stand-in). `bytes` present ⇔ ready to install.
#[derive(Debug, Clone)]
pub struct Slot<U> {
    pub state: UpdateState,
    update: Option<U>,
    bytes: Option<Vec<u8>>,
    /// Bytes since the last progress event.
    unreported: u64,
}

impl<U: Clone> Slot<U> {
    pub fn new(current: &str) -> Self {
        Self { state: UpdateState::new(current), update: None, bytes: None, unreported: 0 }
    }

    /// Starts a check unless one is running or a download is under way.
    pub fn begin_check(&mut self) -> bool {
        if matches!(self.state.phase, Phase::Checking | Phase::Downloading) {
            return false;
        }
        self.state.phase = Phase::Checking;
        self.state.error = None;
        self.state.failed = None;
        true
    }

    /// A check's result. Returns whether the version is new to this run (announce it)
    /// and what to do next.
    pub fn after_check(
        &mut self,
        result: Result<Option<(Found, U)>, String>,
        auto: bool,
        now: i64,
    ) -> (bool, Next) {
        let s = &mut self.state;
        s.checked_at = Some(now);
        let mut is_new = false;
        match result {
            Err(why) => {
                if self.bytes.is_some() {
                    // Offline later: the download stays ready.
                    s.phase = Phase::Ready;
                    s.error = Some(why);
                } else if self.update.is_some() && s.version.is_some() {
                    s.phase = Phase::Available;
                    s.error = Some(why);
                } else {
                    s.fail(Failed::Check, why);
                }
            }
            Ok(None) => {
                let checked = s.checked_at;
                *s = UpdateState::new(&s.current);
                s.checked_at = checked;
                self.update = None;
                self.bytes = None;
            }
            Ok(Some((found, update))) => {
                is_new = s.version.as_deref() != Some(found.version.as_str());
                if is_new {
                    // A newer version replaces one found (or downloaded) earlier, and a
                    // "Restart now" meant for that one.
                    self.bytes = None;
                    s.downloaded = 0;
                    s.total = None;
                    s.install_now = false;
                }
                s.version = Some(found.version);
                s.notes = found.notes;
                s.date = found.date;
                s.phase = if self.bytes.is_some() { Phase::Ready } else { Phase::Available };
                self.update = Some(update);
            }
        }
        let s = &self.state;
        let next = match s.phase {
            Phase::Ready if s.install_now => Next::Install,
            Phase::Available if auto || s.install_now || s.install_on_quit => Next::Download,
            _ => Next::Nothing,
        };
        (is_new, next)
    }

    /// "Restart now" / "Install on quit".
    pub fn request_install(&mut self, when: When) -> Result<Next, AppError> {
        if self.update.is_none() || self.state.version.is_none() {
            return Err(AppError::new("NOT_FOUND", "no update to install; check first"));
        }
        let s = &mut self.state;
        match when {
            When::OnQuit => s.install_on_quit = true,
            When::Now if self.bytes.is_some() => return Ok(Next::Install),
            When::Now => s.install_now = true,
        }
        if self.bytes.is_some() {
            return Ok(Next::Nothing);
        }
        Ok(match s.phase {
            // A check or download under way picks the request up when it ends.
            Phase::Checking | Phase::Downloading => Next::Nothing,
            _ => {
                s.phase = Phase::Available;
                s.error = None;
                s.failed = None;
                Next::Download
            }
        })
    }

    /// The update to download, when one is waiting.
    pub fn start_download(&mut self) -> Option<U> {
        if self.state.phase != Phase::Available || self.bytes.is_some() {
            return None;
        }
        let u = self.update.clone()?;
        self.state.phase = Phase::Downloading;
        self.state.downloaded = 0;
        self.state.total = None;
        self.unreported = 0;
        Some(u)
    }

    /// A downloaded chunk; `true` when it's time for a progress event.
    pub fn progress(&mut self, chunk: usize, total: Option<u64>) -> bool {
        let n = chunk as u64;
        self.state.downloaded = self.state.downloaded.saturating_add(n);
        self.state.total = total;
        self.unreported = self.unreported.saturating_add(n);
        if self.unreported >= PROGRESS_STEP {
            self.unreported = 0;
            return true;
        }
        false
    }

    pub fn finish_download(&mut self, result: Result<Vec<u8>, String>) -> Next {
        let s = &mut self.state;
        match result {
            Ok(bytes) => {
                s.downloaded = bytes.len() as u64;
                s.total = Some(s.downloaded);
                s.phase = Phase::Ready;
                s.error = None;
                s.failed = None;
                self.bytes = Some(bytes);
                if s.install_now { Next::Install } else { Next::Nothing }
            }
            Err(why) => {
                s.downloaded = 0;
                s.total = None;
                s.install_now = false;
                s.fail(Failed::Download, why);
                Next::Nothing
            }
        }
    }

    /// The update and its bytes, taken (never installed twice).
    pub fn take_install(&mut self) -> Option<(U, Vec<u8>)> {
        let u = self.update.clone()?;
        let b = self.bytes.take()?;
        self.state.install_now = false;
        Some((u, b))
    }

    /// At quit: what to install, when "Install on quit" was chosen.
    pub fn take_quit_install(&mut self) -> Option<(U, Vec<u8>)> {
        if self.state.install_on_quit { self.take_install() } else { None }
    }

    /// Installing failed: the bytes are gone, so "Restart now" downloads again.
    pub fn install_failed(&mut self, why: String) {
        self.state.install_now = false;
        self.state.downloaded = 0;
        self.state.total = None;
        self.state.fail(Failed::Install, why);
    }
}

/// The managed updater state.
pub struct Updates {
    slot: Mutex<Option<Slot<Update>>>,
}

impl Default for Updates {
    fn default() -> Self {
        Self { slot: Mutex::new(None) }
    }
}

impl Updates {
    fn with<T>(&self, current: &str, f: impl FnOnce(&mut Slot<Update>) -> T) -> T {
        let mut guard = self.slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        f(guard.get_or_insert_with(|| Slot::new(current)))
    }
}

fn current_version(app: &AppHandle) -> String {
    app.package_info().version.to_string()
}

fn now_ms() -> i64 {
    jiff::Timestamp::now().as_millisecond()
}

fn with<T>(app: &AppHandle, f: impl FnOnce(&mut Slot<Update>) -> T) -> T {
    app.state::<Updates>().with(&current_version(app), f)
}

fn publish(app: &AppHandle) {
    let state = with(app, |s| s.state.clone());
    let _ = app.emit("update:state", state);
}

/// The channel and "download automatically" from the settings.
async fn prefs(app: &AppHandle) -> (String, bool) {
    let read = blocking(app.clone(), |core| {
        core.ask(|e, _| {
            let s = e.store().settings_snapshot().unwrap_or_default();
            let channel = s.get("update_channel").and_then(Value::as_str).unwrap_or("stable");
            let auto = s.get("auto_update").and_then(Value::as_bool).unwrap_or(true);
            ((channel.to_owned(), auto), Vec::new())
        })
    })
    .await;
    read.unwrap_or_else(|_| ("stable".into(), true))
}

fn found_of(u: &Update) -> Found {
    Found {
        version: u.version.clone(),
        notes: u.body.clone().filter(|n| !n.trim().is_empty()),
        date: u
            .date
            .and_then(|d| jiff::Timestamp::from_second(d.unix_timestamp()).ok())
            .map(|t| t.to_string()),
    }
}

async fn fetch(channel: &str, app: &AppHandle) -> Result<Option<Update>, String> {
    // Debug builds only: a test manifest (QA against a local server).
    let url = std::env::var("DECAY_UPDATE_URL")
        .ok()
        .filter(|_| cfg!(debug_assertions))
        .unwrap_or_else(|| endpoint(channel).to_owned());
    let url = url::Url::parse(&url).map_err(|e| e.to_string())?;
    let checker = app
        .updater_builder()
        .endpoints(vec![url])
        .and_then(|b| b.timeout(Duration::from_secs(30)).build())
        .map_err(|e| e.to_string())?;
    checker.check().await.map_err(|e| e.to_string())
}

/// Checks the channel's manifest, then does what the slot decides.
pub async fn check(app: &AppHandle) -> UpdateState {
    if !with(app, Slot::begin_check) {
        return with(app, |s| s.state.clone());
    }
    publish(app);
    let (channel, auto) = prefs(app).await;
    let result = fetch(&channel, app).await;
    if let Err(e) = &result {
        tracing::warn!(channel, "update check failed: {e}");
    }
    let result = result.map(|u| u.map(|u| (found_of(&u), u)));
    let (is_new, next) = with(app, |s| s.after_check(result, auto, now_ms()));
    publish(app);
    if is_new {
        let (version, notes) = with(app, |s| (s.state.version.clone(), s.state.notes.clone()));
        let _ = app.emit("update:available", json!({ "version": version, "notes": notes }));
    }
    carry_out(app, next);
    with(app, |s| s.state.clone())
}

fn carry_out(app: &AppHandle, next: Next) {
    match next {
        Next::Nothing => {}
        Next::Download => {
            let handle = app.clone();
            tauri::async_runtime::spawn(async move { download(&handle).await });
        }
        Next::Install => {
            let handle = app.clone();
            // Installing (unpacking, replacing the bundle) and stopping block.
            tauri::async_runtime::spawn_blocking(move || install_and_restart(&handle));
        }
    }
}

/// Downloads (and verifies) the update found by the last check.
pub async fn download(app: &AppHandle) {
    let Some(update) = with(app, Slot::start_download) else { return };
    publish(app);
    let handle = app.clone();
    let progress = move |chunk: usize, total: Option<u64>| {
        if with(&handle, |s| s.progress(chunk, total)) {
            publish(&handle);
        }
    };
    let result = update.download(progress, || {}).await.map_err(|e| {
        tracing::warn!("update download failed: {e}");
        e.to_string()
    });
    let next = with(app, |s| s.finish_download(result));
    if let Some(v) = with(app, |s| (s.state.phase == Phase::Ready).then(|| s.state.version.clone()))
    {
        tracing::info!(version = v.as_deref().unwrap_or("?"), "update downloaded and verified");
    }
    publish(app);
    carry_out(app, next);
}

/// Installs the downloaded update and starts the new version. Returns only on failure.
fn install_and_restart(app: &AppHandle) {
    let Some((update, bytes)) = with(app, Slot::take_install) else { return };
    if let Err(e) = update.install(bytes) {
        tracing::warn!("update not installed: {e}");
        with(app, |s| s.install_failed(e.to_string()));
        publish(app);
        return;
    }
    tracing::info!(version = update.version, "update installed; restarting");
    app.state::<crate::core::Core>().stop();
    app.restart();
}

/// At quit: installs a downloaded update when "Install on quit" was chosen.
pub fn install_on_quit(app: &AppHandle) {
    let Some((update, bytes)) = with(app, Slot::take_quit_install) else { return };
    match update.install(bytes) {
        Ok(()) => tracing::info!(version = update.version, "update installed at quit"),
        Err(e) => tracing::warn!("update not installed at quit: {e}"),
    }
}

/// A check soon after start-up, then whenever the last one is a day old.
pub fn start(app: &AppHandle) {
    if !automatic_checks() {
        return;
    }
    let handle = app.clone();
    let spawned = std::thread::Builder::new().name("decay-updates".into()).spawn(move || {
        std::thread::sleep(FIRST_CHECK);
        loop {
            let last = with(&handle, |s| s.state.checked_at);
            if last.is_none_or(|t| now_ms() - t >= DAY_MS) {
                tauri::async_runtime::block_on(check(&handle));
            }
            std::thread::sleep(WAKE_EVERY);
        }
    });
    if let Err(e) = spawned {
        tracing::warn!("update checks not started: {e}");
    }
}

/// The channel changed: look there now (when checks are automatic).
pub fn channel_changed(app: &AppHandle) {
    if automatic_checks() {
        let handle = app.clone();
        tauri::async_runtime::spawn(async move { check(&handle).await });
    }
}

#[tauri::command]
pub fn get_update_state(app: AppHandle) -> UpdateState {
    with(&app, |s| s.state.clone())
}

#[tauri::command]
pub async fn check_for_updates(app: AppHandle) -> Res<UpdateState> {
    Ok(check(&app).await)
}

/// "Restart now" (downloads first when needed) or "Install on quit".
#[tauri::command]
pub fn install_update(app: AppHandle, when: When) -> Res<UpdateState> {
    let next = with(&app, |s| s.request_install(when))?;
    publish(&app);
    carry_out(&app, next);
    Ok(with(&app, |s| s.state.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    type S = Slot<&'static str>;

    fn found(v: &str) -> (Found, &'static str) {
        (Found { version: v.into(), notes: Some("Faster search".into()), date: None }, "u")
    }

    fn available(auto: bool) -> S {
        let mut s = S::new("0.1.0");
        assert!(s.begin_check());
        let (new, next) = s.after_check(Ok(Some(found("0.2.0"))), auto, 1);
        assert!(new);
        assert_eq!(next, if auto { Next::Download } else { Next::Nothing });
        s
    }

    fn ready() -> S {
        let mut s = available(true);
        assert_eq!(s.start_download(), Some("u"));
        assert_eq!(s.finish_download(Ok(vec![1, 2, 3])), Next::Nothing);
        assert_eq!((s.state.phase, s.state.total), (Phase::Ready, Some(3)));
        s
    }

    #[test]
    fn channels_have_their_own_manifest() {
        assert_eq!(endpoint("stable"), STABLE_URL);
        assert_eq!(endpoint("beta"), BETA_URL);
        assert_eq!(endpoint("anything"), STABLE_URL);
        assert!(STABLE_URL.starts_with("https://") && BETA_URL.starts_with("https://"));
    }

    #[test]
    fn one_check_at_a_time_and_none_while_downloading() {
        let mut s = available(true);
        assert!(s.begin_check());
        assert!(!s.begin_check());
        let mut s = available(true);
        s.start_download();
        assert!(!s.begin_check());
    }

    #[test]
    fn a_new_version_is_announced_once_and_replaces_an_older_download() {
        let mut s = ready();
        s.begin_check();
        assert_eq!(s.after_check(Ok(Some(found("0.2.0"))), true, 2), (false, Next::Nothing));
        assert_eq!(s.state.phase, Phase::Ready);
        s.begin_check();
        assert_eq!(s.after_check(Ok(Some(found("0.3.0"))), false, 3), (true, Next::Nothing));
        assert_eq!((s.state.phase, s.state.downloaded), (Phase::Available, 0));
        assert_eq!(s.take_install(), None, "the older download is gone");
    }

    #[test]
    fn failed_and_empty_checks() {
        let mut s = S::new("0.1.0");
        s.begin_check();
        s.after_check(Err("offline".into()), true, 1);
        assert_eq!((s.state.phase, s.state.failed), (Phase::Error, Some(Failed::Check)));
        // A ready download survives a failed check.
        let mut s = ready();
        s.begin_check();
        assert_eq!(s.after_check(Err("offline".into()), true, 2).1, Next::Nothing);
        assert_eq!((s.state.phase, s.state.error.as_deref()), (Phase::Ready, Some("offline")));
        // Up to date: everything found before is dropped.
        s.begin_check();
        s.after_check(Ok(None), true, 3);
        assert_eq!(s.state.phase, Phase::Idle);
        assert_eq!(s.state.checked_at, Some(3));
        assert_eq!(s.take_install(), None);
    }

    #[test]
    fn restart_now_installs_a_ready_download_at_once() {
        let mut s = ready();
        assert_eq!(s.request_install(When::Now).unwrap(), Next::Install);
        assert_eq!(s.take_install(), Some(("u", vec![1, 2, 3])));
        assert_eq!(s.take_install(), None, "never twice");
    }

    #[test]
    fn restart_now_downloads_first_and_then_installs() {
        let mut s = available(false);
        assert_eq!(s.request_install(When::Now).unwrap(), Next::Download);
        s.start_download();
        assert_eq!(s.finish_download(Ok(vec![9])), Next::Install);
    }

    #[test]
    fn restart_now_during_a_check_is_kept_not_lost() {
        // Ready, a daily check starts, the user clicks "Restart now".
        let mut s = ready();
        s.begin_check();
        assert_eq!(s.request_install(When::Now).unwrap(), Next::Install);
        // Available (no auto download), a check runs, the user clicks: the check's end
        // starts the download, whose end installs.
        let mut s = available(false);
        s.begin_check();
        assert_eq!(s.request_install(When::Now).unwrap(), Next::Nothing);
        assert_eq!(s.after_check(Ok(Some(found("0.2.0"))), false, 2).1, Next::Download);
        s.start_download();
        assert_eq!(s.finish_download(Ok(vec![1])), Next::Install);
    }

    #[test]
    fn a_restart_meant_for_an_older_version_never_fires_later() {
        let mut s = available(false);
        s.begin_check();
        s.request_install(When::Now).unwrap();
        // The check finds a newer version: the old request is dropped.
        assert_eq!(s.after_check(Ok(Some(found("0.3.0"))), true, 2), (true, Next::Download));
        s.start_download();
        assert_eq!(s.finish_download(Ok(vec![1])), Next::Nothing);
    }

    #[test]
    fn install_on_quit_downloads_and_waits_for_quit() {
        let mut s = available(false);
        assert_eq!(s.request_install(When::OnQuit).unwrap(), Next::Download);
        assert!(s.state.install_on_quit);
        s.start_download();
        assert_eq!(s.finish_download(Ok(vec![4])), Next::Nothing);
        assert_eq!(s.take_quit_install(), Some(("u", vec![4])));
        let mut plain = ready();
        assert_eq!(plain.take_quit_install(), None, "not chosen: nothing at quit");
    }

    #[test]
    fn failures_can_be_retried() {
        let mut s = available(true);
        s.start_download();
        s.request_install(When::Now).unwrap();
        assert_eq!(s.finish_download(Err("bad signature".into())), Next::Nothing);
        assert_eq!((s.state.phase, s.state.failed), (Phase::Error, Some(Failed::Download)));
        assert!(!s.state.install_now);
        assert_eq!(s.take_install(), None, "nothing unverified is kept");
        assert_eq!(s.request_install(When::Now).unwrap(), Next::Download);
        s.start_download();
        s.finish_download(Ok(vec![1]));
        s.take_install();
        s.install_failed("disk full".into());
        assert_eq!(s.state.failed, Some(Failed::Install));
        assert_eq!(s.request_install(When::Now).unwrap(), Next::Download);
    }

    #[test]
    fn nothing_to_install_without_a_check() {
        let mut s = S::new("0.1.0");
        assert_eq!(s.request_install(When::Now).unwrap_err().code, "NOT_FOUND");
        assert_eq!(s.start_download(), None);
    }

    #[test]
    fn progress_is_reported_in_steps() {
        let mut s = available(true);
        s.start_download();
        assert!(!s.progress(100_000, Some(1_000_000)));
        assert!(!s.progress(100_000, Some(1_000_000)));
        assert!(s.progress(100_000, Some(1_000_000)));
        assert_eq!((s.state.downloaded, s.state.total), (300_000, Some(1_000_000)));
    }

    #[test]
    fn state_serializes_for_the_ui() {
        let mut s = available(true);
        s.start_download();
        s.finish_download(Err("x".into()));
        let v = serde_json::to_value(&s.state).unwrap();
        assert_eq!((v["phase"].as_str(), v["failed"].as_str()), (Some("error"), Some("download")));
        assert_eq!(v["installOnQuit"], false);
        let w: When = serde_json::from_value(json!("onQuit")).unwrap();
        assert_eq!(w, When::OnQuit);
    }
}
