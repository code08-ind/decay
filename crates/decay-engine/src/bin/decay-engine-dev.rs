//! `decay-engine-dev`: runs the engine without the desktop shell, for development and
//! end-to-end tests with a real browser and `decay-host` (debug builds only; the app's
//! shell, P1.7, keeps the key in the Keychain / Credential Manager).
//!
//! - Data: `$DECAY_SUPPORT_DIR` (else the app's support folder) `/dev-engine/`.
//! - Key: `$DECAY_DEV_KEY` (64 hex digits), else `dev.key` in that folder (created once).
//! - Commands on stdin: `sweep` (Sweep now), `age <hours>` (adds active hours to every open
//!   tab, to test sweeps without waiting), `mode shadow|automatic|paused`, `ax` (shows the
//!   Accessibility prompt), `status` (also the latest OS sample), `quit`.

#[cfg(not(debug_assertions))]
fn main() {
    eprintln!("decay-engine-dev only runs in debug builds");
    std::process::exit(2);
}

#[cfg(debug_assertions)]
fn main() {
    if let Err(e) = dev::run() {
        eprintln!("decay-engine-dev: {e}");
        std::process::exit(1);
    }
}

#[cfg(debug_assertions)]
mod dev {
    use std::io::BufRead;

    use decay_bridge::ipc::{Endpoint, IpcServer, support_dir};
    use decay_engine::runtime::{Runtime, now_ms};
    use decay_engine::{Engine, EngineConfig, Output};
    use decay_store::{DbKey, Store, StorePaths};

    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
        let support = support_dir().ok_or("no support folder")?;
        let dir = support.join("dev-engine");
        std::fs::create_dir_all(&dir)?;
        let key = if let Ok(hex) = std::env::var("DECAY_DEV_KEY") {
            DbKey::from_hex(hex.trim()).ok_or("DECAY_DEV_KEY isn't 64 hex digits")?
        } else {
            let file = dir.join("dev.key");
            if let Ok(hex) = std::fs::read_to_string(&file) {
                DbKey::from_hex(hex.trim()).ok_or("bad dev.key")?
            } else {
                let key = DbKey::generate()?;
                std::fs::write(&file, key.to_hex())?;
                key
            }
        };
        let (store, report) = Store::open(StorePaths::in_dir(&dir), key, now_ms())?;
        eprintln!("store: {} ({report:?})", dir.display());
        let engine = Engine::new(store, EngineConfig::default(), now_ms())?;
        let mut server = IpcServer::bind(Endpoint::new_for_app()?)?;
        server.publish(&support)?;
        eprintln!("listening on {:?}", server.endpoint());
        let runtime = Runtime::start(server, engine, decay_platform::native(), |event| {
            eprintln!("event: {event:?}");
        })?;
        runtime.list_windows_with(decay_platform::window_lister())?;

        for line in std::io::stdin().lock().lines() {
            let line = line?;
            let mut words = line.split_whitespace();
            match (words.next(), words.next()) {
                (Some("sweep"), _) => {
                    runtime.call(Box::new(|engine, now| match engine.sweep_now(now) {
                        Ok(out) => out,
                        Err(e) => {
                            eprintln!("sweep refused: {e}");
                            Vec::new()
                        }
                    }));
                }
                (Some("age"), Some(hours)) => {
                    let hours: f64 = hours.parse()?;
                    let secs = (hours * 3600.0).max(0.0) as u64;
                    runtime.call(Box::new(move |engine, now| {
                        let n = engine.dev_add_age(secs, now);
                        eprintln!("aged {n} tabs by {hours} h");
                        Vec::<Output>::new()
                    }));
                }
                (Some("mode"), Some(mode)) => {
                    let mode: decay_core::Mode = serde_json::from_value(serde_json::json!(mode))?;
                    runtime.call(Box::new(move |engine, now| {
                        match engine.store().set_setting("mode", &mode, now) {
                            Ok(()) => eprintln!("mode {mode:?}"),
                            Err(e) => eprintln!("couldn't set the mode: {e}"),
                        }
                        Vec::new()
                    }));
                }
                (Some("ax"), _) => {
                    runtime.request_accessibility();
                }
                (Some("status"), _) => {
                    runtime.call(Box::new(|engine, _| {
                        eprintln!("{}", engine.status_line());
                        Vec::new()
                    }));
                }
                (Some("quit"), _) => break,
                (Some(other), _) => eprintln!("unknown command {other:?}"),
                (None, _) => {}
            }
        }
        runtime.stop();
        Ok(())
    }
}
