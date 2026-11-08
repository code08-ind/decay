//! `decay-host` entry point. The browser starts it with the calling extension's origin as
//! the first argument (Windows adds `--parent-window=<id>`).

use std::path::Path;
use std::process::{Command, ExitCode, Stdio};
use std::time::Duration;

use decay_bridge::ipc::{Endpoint, support_dir};
use decay_host::config::HostConfig;
use decay_host::relay::{Outcome, Relay};
use decay_host::{APP_BUNDLE_ID, QUIT_MARKER};

/// Starts the app in the background, detached from this process and from the browser
/// (whose job object would otherwise end it with the host). Not after the user quit it.
fn launch_app(config: &HostConfig, support: &Path) {
    if support.join("run").join(QUIT_MARKER).exists() {
        return;
    }
    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.args(["-g", "-b", APP_BUNDLE_ID]);
        c
    } else if let Some(path) = &config.app_path {
        let mut c = Command::new(path);
        c.arg("--background");
        c
    } else {
        return;
    };
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB);
        if cmd.spawn().is_ok() {
            return;
        }
        // The browser's job may not allow breaking away: start it inside the job instead.
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    let _ = cmd.spawn();
}

fn main() -> ExitCode {
    let origin = std::env::args().nth(1).unwrap_or_default();
    let Some(support) = support_dir() else { return ExitCode::FAILURE };
    let config = HostConfig::load(&support);
    let launch_config = config.clone();
    let relay = Relay {
        origin,
        allowed: config.allowed(),
        endpoint: Endpoint::default_for_user(),
        launch_app: Box::new(move || launch_app(&launch_config, &support)),
        connect_timeout: Duration::from_secs(3),
        pid: std::process::id(),
    };
    let finished = relay.run(std::io::stdin(), std::io::stdout());
    // Relay threads may still be blocked on stdin or the socket. Exit while holding the
    // output lock, so none of them is cut off in the middle of a frame.
    let _output = finished.output.lock();
    let code = match finished.outcome {
        Outcome::BrowserClosed | Outcome::AppClosed | Outcome::AppUnavailable => 0,
        Outcome::RejectedOrigin | Outcome::BadHello => 1,
    };
    std::process::exit(code)
}
