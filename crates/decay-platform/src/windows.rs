//! App windows (P2.1, Docs/Spikes.md): what's open, in which app, showing which document,
//! with unsaved changes or not. Listed on its own thread every few seconds (Accessibility
//! calls go to each app and can be slow), never on the engine thread.
//!
//! Only standard windows are listed (`AXStandardWindow`: no panels, sheets, dialogs,
//! palettes). Titles come from Accessibility; Decay never asks for Screen Recording.

use std::path::{Path, PathBuf};

/// One standard window of an app.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[allow(clippy::struct_excessive_bools, reason = "independent facts the OS reports")]
pub struct AppWindow {
    /// The system's window number (`CGWindowID`) while the window lives; `None` if it
    /// couldn't be read (such a window is tracked by title but never hidden).
    pub window_id: Option<u32>,
    pub title: String,
    /// The document it shows (`AXDocument`, a `file://` URL), if the app says.
    pub document: Option<String>,
    /// Unsaved changes (the close button's `AXEdited`); `None` = can't tell.
    pub edited: Option<bool>,
    pub minimized: bool,
    pub full_screen: bool,
    /// It has an enabled minimise button.
    pub can_minimize: bool,
    /// The app's focused window.
    pub focused: bool,
}

/// One app's standard windows.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AppWindows {
    pub pid: i32,
    /// The app's bundle identifier (e.g. `com.apple.TextEdit`).
    pub bundle_id: String,
    pub name: String,
    /// The standard windows Accessibility reported (often only those on the current
    /// Space; empty if the app didn't answer in time).
    pub windows: Vec<AppWindow>,
    /// Every normal window number the window server knows for the app, on every Space
    /// (`CGWindowList`): a window in here still exists even when `windows` misses it.
    pub all_window_ids: Vec<u32>,
}

/// Every app's windows at one moment.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WindowList {
    pub apps: Vec<AppWindows>,
    /// When listing started (epoch ms, set by the runtime): a list started before a
    /// window was hidden can't say it was brought back.
    pub started_at: Option<i64>,
}

/// Lists app windows.
pub trait WindowLister: Send {
    /// The windows now, or `None` when they can't be listed (no Accessibility access, or
    /// not supported here). `None` never means "every window closed".
    fn list(&mut self) -> Option<WindowList>;
}

/// What to do to one window (P2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowOp {
    /// Minimise it to the Dock ("hidden").
    Hide,
    /// Un-minimise it, raise it and bring its app to the front.
    Restore,
}

/// Acts on single windows. Each call checks what it can before acting (a standard
/// window, with an enabled minimise button, not full screen) and confirms the result by
/// reading it back; anything unexpected is an error and nothing is changed.
pub trait WindowActor: Send {
    /// `Ok(())` once the window is minimised (`Hide`) or back on screen (`Restore`).
    fn act(&mut self, op: WindowOp, pid: i32, window_id: u32) -> Result<(), String>;
}

/// An actor that can't do anything.
#[derive(Debug, Default, Clone, Copy)]
pub struct NullWindowActor;

impl WindowActor for NullWindowActor {
    fn act(&mut self, _: WindowOp, _: i32, _: u32) -> Result<(), String> {
        Err("windows can't be hidden on this platform".into())
    }
}

/// The window actor for this OS.
pub fn window_actor() -> Box<dyn WindowActor> {
    #[cfg(target_os = "macos")]
    {
        Box::new(crate::mac_windows::MacWindowActor)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Box::new(NullWindowActor)
    }
}

/// A lister that can't list anything.
#[derive(Debug, Default, Clone, Copy)]
pub struct NullWindowLister;

impl WindowLister for NullWindowLister {
    fn list(&mut self) -> Option<WindowList> {
        None
    }
}

/// The window lister for this OS.
pub fn window_lister() -> Box<dyn WindowLister> {
    #[cfg(target_os = "macos")]
    {
        Box::new(crate::mac_windows::MacWindowLister::new())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Box::new(NullWindowLister)
    }
}

/// The app bundle an executable belongs to: the innermost `*.app` folder on its path
/// (an app shipped inside another, like Simulator inside Xcode, is its own app; the
/// processes that own windows are apps' main executables, not their helpers). `None`
/// for a plain executable.
pub fn app_bundle_path(exe: &Path) -> Option<PathBuf> {
    let mut path = PathBuf::new();
    let mut found = None;
    for part in exe.components() {
        path.push(part);
        let is_app = part
            .as_os_str()
            .to_str()
            .is_some_and(|s| s.len() > 4 && s.to_ascii_lowercase().ends_with(".app"));
        if is_app {
            found = Some(path.clone());
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_innermost_app_bundle_owns_an_executable() {
        let p = |s: &str| app_bundle_path(Path::new(s));
        assert_eq!(
            p("/Applications/TextEdit.app/Contents/MacOS/TextEdit"),
            Some(PathBuf::from("/Applications/TextEdit.app"))
        );
        assert_eq!(
            p(
                "/Applications/Visual Studio Code.app/Contents/Frameworks/Code Helper.app/Contents/MacOS/Code Helper"
            ),
            Some(PathBuf::from(
                "/Applications/Visual Studio Code.app/Contents/Frameworks/Code Helper.app"
            ))
        );
        assert_eq!(
            p(
                "/Applications/Xcode.app/Contents/Developer/Applications/Simulator.app/Contents/MacOS/Simulator"
            ),
            Some(PathBuf::from(
                "/Applications/Xcode.app/Contents/Developer/Applications/Simulator.app"
            ))
        );
        assert_eq!(p("/usr/bin/vim"), None);
        assert_eq!(p("/Users/me/.app/bin/tool"), None, "a bare `.app` isn't a bundle name");
    }

    #[test]
    fn the_null_lister_lists_nothing_and_the_null_actor_does_nothing() {
        assert_eq!(NullWindowLister.list(), None);
        assert!(NullWindowActor.act(WindowOp::Hide, 1, 1).is_err());
    }
}
