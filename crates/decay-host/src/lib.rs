//! `decay-host`: the native messaging host the browser starts when the extension calls
//! `chrome.runtime.connectNative('com.decay.host')` (Software Design §4.8).
//!
//! - [`relay`]: checks the caller and relays messages between the extension and the app.
//! - [`install`]: writes the per-browser host manifests (called by the app at first run).
//! - [`config`]: the allowed extension IDs and the app's path.
//!
//! The process talks the native messaging protocol on stdin / stdout, so it must never
//! print anything else there.

pub mod config;
pub mod install;
pub mod relay;

/// The app's bundle id (macOS), used to start it.
pub const APP_BUNDLE_ID: &str = "com.decay.app";

/// `<support dir>/run/<QUIT_MARKER>`: written by the app when the user quits it (and
/// removed when it starts), so the host doesn't start it again.
pub const QUIT_MARKER: &str = "quit-by-user";
