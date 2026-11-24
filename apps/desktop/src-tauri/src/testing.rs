//! Test helpers: a real engine on a fresh database (no Tauri, no IPC), for the engine-side
//! halves of the commands.

use decay_engine::{Engine, EngineConfig};
use decay_store::{DbKey, Store, StorePaths};

pub const NOW: i64 = 1_800_000_000_000;

/// An engine on a fresh database in a temporary folder (kept alive by the returned guard).
pub fn engine() -> (Engine, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let (store, _) =
        Store::open(StorePaths::in_dir(dir.path()), DbKey::from_bytes([7; 32]), NOW).unwrap();
    (Engine::new(store, EngineConfig::default(), NOW).unwrap(), dir)
}

/// A running engine behind a `Core` (the engine thread, an IPC endpoint), for code that
/// talks to `Core`. Stopped and cleaned up on drop.
pub struct RunningCore {
    pub core: crate::core::Core,
    support: std::path::PathBuf,
    _data: tempfile::TempDir,
}

impl Drop for RunningCore {
    fn drop(&mut self) {
        self.core.stop();
        let _ = std::fs::remove_dir_all(&self.support);
    }
}

/// Starts one. Each gets its own short support folder (a socket path must fit in 104
/// bytes on macOS) and pipe name.
pub fn core() -> RunningCore {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let tag = format!("{}-{n}", std::process::id());
    let data = tempfile::tempdir().unwrap();
    let support = std::env::temp_dir().join(format!("decay-t{tag}"));
    let support = if cfg!(target_os = "macos") {
        std::path::PathBuf::from(format!("/private/tmp/decay-t{tag}"))
    } else {
        support
    };
    std::fs::create_dir_all(&support).unwrap();
    let dir = crate::core::DataDir {
        support: support.clone(),
        data: data.path().to_owned(),
        key: crate::keys::KeySource::File(data.path().join("k")),
    };
    let endpoint = if cfg!(windows) {
        decay_bridge::ipc::Endpoint::Pipe(format!("decay-t{tag}"))
    } else {
        decay_bridge::ipc::Endpoint::Socket(support.join("run/decay.sock"))
    };
    let core = crate::core::Core::default();
    core.start(&dir, endpoint, |_| {}).unwrap();
    RunningCore { core, support, _data: data }
}
