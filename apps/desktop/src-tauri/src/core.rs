//! The engine inside the app: opening the store, starting the runtime, and calling into
//! the engine thread from Tauri commands and the tray.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU8;
use std::sync::atomic::Ordering::SeqCst;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use decay_bridge::ipc::{Endpoint, IpcServer};
use decay_engine::runtime::{Runtime, now_ms};
use decay_engine::{Engine, EngineConfig, EngineEvent, EngineStatus, Output};
use decay_store::{Store, StorePaths};
use serde::Serialize;

use crate::keys::{self, KeySource};

/// How long a command waits for the engine thread.
const CALL_TIMEOUT: Duration = Duration::from_secs(5);

/// API Design §1 `AppError`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, thiserror::Error)]
#[error("{code}: {message}")]
pub struct AppError {
    pub code: &'static str,
    pub message: String,
}

impl AppError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }

    pub fn db(e: &dyn std::fmt::Display) -> Self {
        Self::new("DB_ERROR", e.to_string())
    }
}

/// Where the app keeps its data, and its key.
#[derive(Debug, Clone)]
pub struct DataDir {
    /// The support folder (IPC endpoint, host config, quit marker).
    pub support: PathBuf,
    /// The database folder: the support folder, or `dev-app/` in debug builds.
    pub data: PathBuf,
    pub key: KeySource,
}

impl DataDir {
    pub fn for_build(support: PathBuf) -> Self {
        let keychain = std::env::var_os("DECAY_USE_KEYCHAIN").is_some_and(|v| v == "1");
        if cfg!(debug_assertions) {
            let data = support.join("dev-app");
            let key =
                if keychain { KeySource::Keychain } else { KeySource::File(data.join("dev.key")) };
            Self { support, data, key }
        } else {
            Self { data: support.clone(), support, key: KeySource::Keychain }
        }
    }
}

/// The running engine, shared by commands and the tray.
#[derive(Debug, Default)]
pub struct Core {
    runtime: Mutex<Option<Runtime>>,
    status: Mutex<Option<EngineStatus>>,
}

impl Core {
    /// Opens the store and starts the engine, serving `endpoint` (the app's:
    /// `Endpoint::new_for_app()`). `events` runs on the engine thread.
    pub fn start(
        &self,
        dir: &DataDir,
        endpoint: Endpoint,
        events: impl FnMut(EngineEvent) + Send + 'static,
    ) -> Result<(), String> {
        // The endpoint first: if another copy serves it, the data isn't touched at all.
        let mut server = IpcServer::bind(endpoint)
            .map_err(|e| format!("another copy of Decay (or decay-engine-dev) is running: {e}"))?;
        let paths = StorePaths::in_dir(&dir.data);
        let key = keys::load_or_create(&dir.key, &paths).map_err(|e| e.to_string())?;
        let (store, report) = Store::open(paths, key, now_ms())
            .map_err(|e| format!("the database can't be opened: {e}"))?;
        tracing::info!(?report, data = %dir.data.display(), "store open");
        let engine = Engine::new(store, EngineConfig::default(), now_ms())
            .map_err(|e| format!("the engine can't start: {e}"))?;
        server.publish(&dir.support).map_err(|e| e.to_string())?;
        let runtime = Runtime::start(server, engine, decay_platform::native(), events)
            .map_err(|e| e.to_string())?;
        // App windows (P2.1): listed only with Accessibility and window handling on.
        runtime.list_windows_with(decay_platform::window_lister()).map_err(|e| e.to_string())?;
        runtime.act_on_windows_with(decay_platform::window_actor()).map_err(|e| e.to_string())?;
        *self.runtime.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(runtime);
        Ok(())
    }

    /// Stops the engine (quit). Safe to call twice.
    pub fn stop(&self) {
        let runtime = self.runtime.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take();
        if let Some(rt) = runtime {
            rt.stop();
        }
    }

    /// Remembers the latest status (the engine thread calls it before the tray updates).
    pub fn set_status(&self, status: EngineStatus) {
        *self.status.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(status);
    }

    pub fn status(&self) -> Option<EngineStatus> {
        self.status.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
    }

    /// Runs `f` on the engine thread without waiting (tray actions).
    pub fn send(&self, f: impl FnOnce(&mut Engine, i64) -> Vec<Output> + Send + 'static) {
        let guard = self.runtime.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if !guard.as_ref().is_some_and(|rt| rt.call(Box::new(f))) {
            tracing::warn!("the engine isn't running");
        }
    }

    /// Runs `f` on the engine thread and waits for its answer (commands; call it off the
    /// main thread). If the engine doesn't get to it within 5 s, it's withdrawn and never
    /// runs (`TIMEOUT` means nothing changed); once it has started, its answer is awaited.
    pub fn ask<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Engine, i64) -> (T, Vec<Output>) + Send + 'static,
    ) -> Result<T, AppError> {
        const QUEUED: u8 = 0;
        const STARTED: u8 = 1;
        const WITHDRAWN: u8 = 2;
        let state = Arc::new(AtomicU8::new(QUEUED));
        let (tx, rx) = mpsc::sync_channel(1);
        {
            let state = Arc::clone(&state);
            let guard = self.runtime.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let sent = guard.as_ref().is_some_and(|rt| {
                rt.call(Box::new(move |engine, now| {
                    if state.compare_exchange(QUEUED, STARTED, SeqCst, SeqCst).is_err() {
                        return Vec::new(); // withdrawn: the caller already got TIMEOUT
                    }
                    let (answer, out) = f(engine, now);
                    let _ = tx.send(answer);
                    out
                }))
            });
            if !sent {
                return Err(AppError::new("ADAPTER_UNAVAILABLE", "the engine isn't running"));
            }
        }
        let gone = || AppError::new("ADAPTER_UNAVAILABLE", "the engine stopped");
        match rx.recv_timeout(CALL_TIMEOUT) {
            Ok(answer) => Ok(answer),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(gone()),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if state.compare_exchange(QUEUED, WITHDRAWN, SeqCst, SeqCst).is_ok() {
                    Err(AppError::new("TIMEOUT", "the engine is busy; nothing was changed"))
                } else {
                    rx.recv().map_err(|_| gone()) // running now: its result is coming
                }
            }
        }
    }

    /// Asks the system for the Accessibility permission; `false` if the engine isn't
    /// running (nothing was asked).
    pub fn request_accessibility(&self) -> bool {
        let guard = self.runtime.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.as_ref().is_some_and(Runtime::request_accessibility)
    }
}

/// `<support>/run/quit-by-user`: the host doesn't start an app the user quit.
pub fn quit_marker(support: &Path) -> PathBuf {
    support.join("run").join(decay_host::QUIT_MARKER)
}

/// The user's home folder.
pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

/// Why the native host couldn't be installed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HostInstallError {
    /// This build has no `decay-host` next to it (development builds).
    #[error("this build of Decay doesn't include its browser connector")]
    NotBundled,
    /// Installing failed for every detected browser, or for all of them at once.
    #[error("{0}")]
    Failed(String),
}

impl From<HostInstallError> for AppError {
    fn from(e: HostInstallError) -> Self {
        let code = match e {
            HostInstallError::NotBundled => "NOT_FOUND",
            HostInstallError::Failed(_) => "PERMISSION_DENIED",
        };
        Self::new(code, e.to_string())
    }
}

/// One install at a time (start-up and onboarding can ask together).
static HOST_INSTALL: Mutex<()> = Mutex::new(());

/// Installs the native messaging host for every detected browser, when the host binary
/// sits next to the app (bundled builds; Software Design §4.8 "install at every start").
/// Returns the browsers it's installed for; fails when it couldn't be installed for any
/// detected browser.
pub fn install_host(support: &Path) -> Result<Vec<&'static str>, HostInstallError> {
    let _one_at_a_time = HOST_INSTALL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let failed = |why: String| HostInstallError::Failed(why);
    let exe = std::env::current_exe().map_err(|e| failed(e.to_string()))?;
    let name = if cfg!(windows) { "decay-host.exe" } else { "decay-host" };
    let Some(host) = exe.parent().map(|d| d.join(name)).filter(|p| p.is_file()) else {
        tracing::info!("no bundled decay-host next to the app; host install skipped");
        return Err(HostInstallError::NotBundled);
    };
    let home = home_dir().ok_or_else(|| failed("no home folder".into()))?;
    let config = decay_host::config::HostConfig::load(support);
    let opts = decay_host::install::InstallOptions {
        home,
        support_dir: support.to_owned(),
        host_path: host,
        origins: config.allowed_origins,
        app_path: Some(exe),
        registry_root: "Software".into(),
    };
    let report = decay_host::install::install(&opts).map_err(|e| {
        tracing::warn!("native host not installed: {e}");
        failed(e.to_string())
    })?;
    for done in &report.done {
        tracing::info!(browser = done.browser, "native host installed");
    }
    for (browser, e) in &report.failed {
        tracing::warn!(browser, "native host install failed: {e}");
    }
    install_outcome(&report)
}

/// Success unless every browser that was tried failed.
fn install_outcome(
    report: &decay_host::install::Report,
) -> Result<Vec<&'static str>, HostInstallError> {
    if report.done.is_empty() && !report.failed.is_empty() {
        let list: Vec<String> = report.failed.iter().map(|(b, e)| format!("{b}: {e}")).collect();
        return Err(HostInstallError::Failed(list.join("; ")));
    }
    Ok(report.done.iter().map(|d| d.browser).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_builds_keep_their_own_data_and_key_file() {
        let dir = DataDir::for_build(PathBuf::from("/support"));
        assert_eq!(dir.support, PathBuf::from("/support"));
        if cfg!(debug_assertions) && std::env::var_os("DECAY_USE_KEYCHAIN").is_none() {
            assert_eq!(dir.data, PathBuf::from("/support/dev-app"));
            assert_eq!(dir.key, KeySource::File(PathBuf::from("/support/dev-app/dev.key")));
        }
        assert_eq!(quit_marker(&dir.support), PathBuf::from("/support/run/quit-by-user"));
    }

    #[test]
    fn host_install_fails_only_when_every_browser_failed() {
        use decay_host::install::{Installed, Report};
        let ok = Installed { browser: "Brave", location: "x".into() };
        let err = || std::io::Error::other("denied");
        assert_eq!(install_outcome(&Report::default()).unwrap(), Vec::<&str>::new());
        let mixed = Report { done: vec![ok], failed: vec![("Arc", err())] };
        assert_eq!(install_outcome(&mixed).unwrap(), ["Brave"]);
        let none = Report { done: vec![], failed: vec![("Arc", err())] };
        assert_eq!(
            install_outcome(&none).unwrap_err(),
            HostInstallError::Failed("Arc: denied".into())
        );
        assert_eq!(AppError::from(HostInstallError::NotBundled).code, "NOT_FOUND");
        assert_eq!(AppError::from(HostInstallError::Failed("x".into())).code, "PERMISSION_DENIED");
    }

    #[test]
    fn calls_fail_cleanly_without_an_engine() {
        let core = Core::default();
        let err = core.ask(|_, _| ((), Vec::new())).unwrap_err();
        assert_eq!(err.code, "ADAPTER_UNAVAILABLE");
        core.send(|_, _| Vec::new());
        core.stop();
        assert_eq!(core.status(), None);
    }

    #[test]
    fn the_engine_starts_answers_and_stops() {
        let tmp = tempfile::tempdir().unwrap();
        // A short support path: the socket path must fit (macOS: 104 bytes).
        let support = PathBuf::from(format!("/private/tmp/decay-app-test-{}", std::process::id()));
        std::fs::create_dir_all(&support).unwrap();
        let dir = DataDir {
            support: support.clone(),
            data: tmp.path().to_owned(),
            key: KeySource::File(tmp.path().join("k")),
        };
        let core = Core::default();
        let endpoint = if cfg!(windows) {
            Endpoint::Pipe(format!("decay-apptest-{}", std::process::id()))
        } else {
            Endpoint::Socket(support.join("run/decay.sock"))
        };
        core.start(&dir, endpoint, |_| {}).unwrap();
        let status = core.ask(|engine, now| (engine.status(now), Vec::new())).unwrap();
        assert_eq!(status.live_tabs, 0);
        // A busy engine: the call is withdrawn after 5 s and never runs.
        core.send(|_, _| {
            std::thread::sleep(Duration::from_secs(6));
            Vec::new()
        });
        let ran = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = Arc::clone(&ran);
        let err = core.ask(move |_, _| {
            flag.store(true, SeqCst);
            ((), Vec::new())
        });
        assert_eq!(err.unwrap_err().code, "TIMEOUT");
        // Once the engine is free again, later calls work and the withdrawn one didn't run.
        core.ask(|_, _| ((), Vec::new())).unwrap();
        assert!(!ran.load(SeqCst));
        core.stop();
        let _ = std::fs::remove_dir_all(&support);
    }
}
