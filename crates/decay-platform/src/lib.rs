//! Decay's OS layer (Software Design §4.6 `trait Platform`, P1.6): what the machine and the
//! user are doing, sampled about once a second by the engine.
//!
//! - **Idle time** (any input) and **time since the last key press** (a quiet moment for
//!   sweeps means no typing for a minute).
//! - **Locked** (screen lock, or another user's session on the console) and **display
//!   asleep**: the aging clock stops (PRD F1, story S1 "Monday reset").
//! - **Audio playing** anywhere (watching a video counts as being active).
//! - **The frontmost app** (an app switch postpones sweeps).
//! - **Accessibility** permission (F33: needed to hide and close app windows later; its
//!   loss is reported as a health problem).
//!
//! - **A continuous clock** that keeps running through sleep and never jumps, so the engine
//!   can tell a real sleep from the wall clock being changed.
//!
//! Every value is optional: `None` means "this platform can't tell", and the engine then
//! falls back to safe defaults (browser activity as the idle signal). Sleep itself needs
//! no signal: a gap between samples is a sleep, and the aging clock gives it no credit.
//!
//! Accessibility gates only work on app windows (hiding / closing them, P2.x). Tab sweeps
//! go through the browser extension and never need it, so a missing permission is a health
//! problem to show, not a reason to stop sweeping tabs.
//!
//! macOS reads everything without Screen Recording (no window titles are touched). Windows
//! comes with its own implementation later; until then [`NullPlatform`] is used there.

#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "macos")]
mod mac_windows;
pub mod windows;

pub use windows::{
    AppWindow, AppWindows, NullWindowActor, NullWindowLister, WindowActor, WindowList,
    WindowLister, WindowOp, window_actor, window_lister,
};

#[cfg(target_os = "macos")]
pub use mac::MacPlatform;

/// The app in front.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontApp {
    pub pid: i32,
    /// The owning process's name (e.g. "Google Chrome").
    pub name: String,
}

/// One sample of the OS state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlatformSample {
    /// Seconds since the last keyboard, mouse or trackpad input.
    pub idle_secs: Option<u64>,
    /// Seconds since the last key press.
    pub secs_since_key: Option<u64>,
    /// The screen is locked, or this user's session isn't the one on the console.
    pub locked: Option<bool>,
    /// The displays are asleep.
    pub display_asleep: Option<bool>,
    /// Some app is playing (or recording) through the default output device.
    pub audio_playing: Option<bool>,
    pub frontmost: Option<FrontApp>,
    /// Decay may use the Accessibility API.
    pub accessibility: Option<bool>,
    /// Milliseconds on a clock that keeps running while the machine sleeps and isn't moved
    /// by wall-clock changes (macOS `CLOCK_MONOTONIC`). Only differences mean anything.
    pub continuous_ms: Option<u64>,
}

/// The OS layer.
pub trait Platform: Send {
    /// What the machine and the user are doing now.
    fn sample(&mut self) -> PlatformSample;

    /// Shows the system's Accessibility prompt (System Settings › Privacy & Security ›
    /// Accessibility) if Decay isn't trusted yet. Returns whether it is trusted now.
    fn request_accessibility(&mut self) -> bool;
}

/// A platform that knows nothing (every value `None`).
#[derive(Debug, Default, Clone, Copy)]
pub struct NullPlatform;

impl Platform for NullPlatform {
    fn sample(&mut self) -> PlatformSample {
        PlatformSample::default()
    }

    fn request_accessibility(&mut self) -> bool {
        false
    }
}

/// The platform for this OS.
pub fn native() -> Box<dyn Platform> {
    #[cfg(target_os = "macos")]
    {
        Box::new(MacPlatform::new())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Box::new(NullPlatform)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_null_platform_knows_nothing() {
        let mut p = NullPlatform;
        assert_eq!(p.sample(), PlatformSample::default());
        assert!(!p.request_accessibility());
    }

    #[test]
    fn the_native_platform_samples() {
        let mut p = native();
        let s = p.sample();
        // On a Mac (even a CI runner) idle time is always known; values stay sane.
        if cfg!(target_os = "macos") {
            assert!(s.idle_secs.is_some());
        }
        if let (Some(idle), Some(key)) = (s.idle_secs, s.secs_since_key) {
            assert!(key >= idle.saturating_sub(1), "key {key} < idle {idle}");
        }
    }
}
