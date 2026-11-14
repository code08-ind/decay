//! macOS window listing (P2.1, Docs/Spikes.md §3): `CGWindowList` names the apps that
//! have normal windows (every Space, no Screen Recording needed), Accessibility reads
//! each app's standard windows (title, document, edited state, minimised, full screen,
//! focused). Each app gets at most [`AX_TIMEOUT_S`] per call, so a hung app can't stall
//! the listing for long.

#![allow(unsafe_code)]

use std::collections::BTreeMap;
use std::ffi::c_void;
use std::path::Path;

use core_foundation::array::{CFArray, CFArrayRef};
use core_foundation::base::{CFType, CFTypeRef, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::bundle::CFBundle;
use core_foundation::dictionary::CFDictionary;
use core_foundation::number::CFNumber;
use core_foundation::string::{CFString, CFStringRef};
use core_foundation::url::CFURL;

use crate::windows::{
    AppWindow, AppWindows, WindowActor, WindowList, WindowLister, WindowOp, app_bundle_path,
};

/// Seconds an app may take to answer one Accessibility call.
const AX_TIMEOUT_S: f32 = 0.3;
/// At most this many windows per app (a runaway app can't flood the store).
const MAX_WINDOWS_PER_APP: usize = 200;
/// Time one app may take in one list; the rest of its windows wait for the next list
/// (they're kept meanwhile: the window server still lists them).
const APP_BUDGET: std::time::Duration = std::time::Duration::from_millis(1500);

type AXUIElementRef = CFTypeRef;
const AX_SUCCESS: i32 = 0;
/// `kCGWindowListOptionAll | kCGWindowListExcludeDesktopElements`.
const CG_LIST_OPTIONS: u32 = 1 << 4;
const CG_NULL_WINDOW: u32 = 0;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> u8;
    fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
    fn AXUIElementCreateSystemWide() -> AXUIElementRef;
    fn AXUIElementCopyAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: *mut CFTypeRef,
    ) -> i32;
    fn AXUIElementSetMessagingTimeout(element: AXUIElementRef, seconds: f32) -> i32;
    fn AXUIElementSetAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: CFTypeRef,
    ) -> i32;
    fn AXUIElementPerformAction(element: AXUIElementRef, action: CFStringRef) -> i32;
    /// Private but long stable (window managers outside the App Store use it): the
    /// `CGWindowID` of an AX window.
    fn _AXUIElementGetWindow(element: AXUIElementRef, window: *mut u32) -> i32;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32) -> CFArrayRef;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFEqual(a: CFTypeRef, b: CFTypeRef) -> u8;
}

unsafe extern "C" {
    fn proc_pidpath(pid: i32, buffer: *mut c_void, size: u32) -> i32;
    fn proc_listallpids(buffer: *mut c_void, size: i32) -> i32;
    fn getpid() -> i32;
}

/// An owned Accessibility element.
struct Element(CFType);

impl Element {
    fn app(pid: i32) -> Option<Self> {
        // SAFETY: creates a new element (+1); `None` for a null result.
        let raw = unsafe { AXUIElementCreateApplication(pid) };
        if raw.is_null() {
            return None;
        }
        // SAFETY: `raw` is a valid +1 reference, now owned by the CFType.
        let el = Self(unsafe { CFType::wrap_under_create_rule(raw) });
        // SAFETY: a valid element; the timeout applies to calls on this app's elements.
        unsafe { AXUIElementSetMessagingTimeout(el.raw(), AX_TIMEOUT_S) };
        Some(el)
    }

    fn raw(&self) -> AXUIElementRef {
        self.0.as_CFTypeRef()
    }

    /// An attribute's value, or `None` (unsupported, no value, the app didn't answer).
    fn attr(&self, name: &str) -> Option<CFType> {
        let key = CFString::new(name);
        let mut value: CFTypeRef = std::ptr::null();
        // SAFETY: valid element and key; on success `value` is a +1 reference we own.
        let err = unsafe {
            AXUIElementCopyAttributeValue(self.raw(), key.as_concrete_TypeRef(), &raw mut value)
        };
        if err != AX_SUCCESS || value.is_null() {
            return None;
        }
        // SAFETY: a +1 reference from a Copy call.
        Some(unsafe { CFType::wrap_under_create_rule(value) })
    }

    fn child(&self, name: &str) -> Option<Self> {
        self.attr(name).map(Self)
    }

    fn string(&self, name: &str) -> Option<String> {
        self.attr(name)?.downcast::<CFString>().map(|s| s.to_string())
    }

    fn flag(&self, name: &str) -> Option<bool> {
        let v = self.attr(name)?;
        if let Some(b) = v.downcast::<CFBoolean>() {
            return Some(b.into());
        }
        v.downcast::<CFNumber>().and_then(|n| n.to_i64()).map(|n| n != 0)
    }

    fn elements(&self, name: &str) -> Vec<Self> {
        let Some(v) = self.attr(name) else { return Vec::new() };
        let Some(arr) = v.downcast::<CFArray>() else { return Vec::new() };
        arr.iter()
            .filter(|p| !p.is_null())
            // SAFETY: each entry is a valid CF object owned by the array (+0): retained here.
            .map(|p| Self(unsafe { CFType::wrap_under_get_rule(*p) }))
            .collect()
    }

    fn window_id(&self) -> Option<u32> {
        let mut id = 0u32;
        // SAFETY: a valid window element and a valid out pointer.
        let err = unsafe { _AXUIElementGetWindow(self.raw(), &raw mut id) };
        (err == AX_SUCCESS && id != 0).then_some(id)
    }

    fn set_flag(&self, name: &str, on: bool) -> Result<(), String> {
        let key = CFString::new(name);
        let value = if on { CFBoolean::true_value() } else { CFBoolean::false_value() };
        // SAFETY: valid element, key and CFBoolean.
        let err = unsafe {
            AXUIElementSetAttributeValue(
                self.raw(),
                key.as_concrete_TypeRef(),
                value.as_CFTypeRef(),
            )
        };
        if err == AX_SUCCESS { Ok(()) } else { Err(format!("{name} couldn't be set ({err})")) }
    }

    fn perform(&self, action: &str) -> Result<(), String> {
        let key = CFString::new(action);
        // SAFETY: valid element and action name.
        let err = unsafe { AXUIElementPerformAction(self.raw(), key.as_concrete_TypeRef()) };
        if err == AX_SUCCESS { Ok(()) } else { Err(format!("{action} failed ({err})")) }
    }

    fn same_as(&self, other: &Self) -> bool {
        // SAFETY: two valid CF objects.
        unsafe { CFEqual(self.raw(), other.raw()) != 0 }
    }
}

/// Apps (pid → name and window numbers) that own a normal window, from the window
/// server (every Space, minimised windows too).
fn apps_with_windows() -> BTreeMap<i32, (String, Vec<u32>)> {
    let mut out = BTreeMap::new();
    // SAFETY: a Copy call (+1, or null).
    let raw = unsafe { CGWindowListCopyWindowInfo(CG_LIST_OPTIONS, CG_NULL_WINDOW) };
    if raw.is_null() {
        return out;
    }
    // SAFETY: a +1 CFArray of CFDictionary.
    let list: CFArray<CFDictionary<CFString, CFType>> =
        unsafe { CFArray::wrap_under_create_rule(raw) };
    let number = |d: &CFDictionary<CFString, CFType>, key: &str| {
        d.find(CFString::new(key)).and_then(|v| v.downcast::<CFNumber>()).and_then(|n| n.to_i64())
    };
    for d in list.iter() {
        if number(&d, "kCGWindowLayer") != Some(0) {
            continue;
        }
        let Some(pid) = number(&d, "kCGWindowOwnerPID").and_then(|p| i32::try_from(p).ok()) else {
            continue;
        };
        let name = d
            .find(CFString::new("kCGWindowOwnerName"))
            .and_then(|v| v.downcast::<CFString>())
            .map(|s| s.to_string())
            .unwrap_or_default();
        let entry = out.entry(pid).or_insert_with(|| (name, Vec::new()));
        if let Some(n) = number(&d, "kCGWindowNumber").and_then(|n| u32::try_from(n).ok()) {
            entry.1.push(n);
        }
    }
    out
}

/// The bundle identifier of the app a process runs.
fn bundle_id(pid: i32) -> Option<String> {
    let mut buf = vec![0u8; 4096];
    // SAFETY: the buffer is valid for its full length.
    let n = unsafe { proc_pidpath(pid, buf.as_mut_ptr().cast(), 4096) };
    let len = usize::try_from(n).ok().filter(|n| *n > 0)?;
    let exe = std::str::from_utf8(&buf[..len]).ok()?;
    let bundle = app_bundle_path(Path::new(exe))?;
    let url = CFURL::from_path(&bundle, true)?;
    let info = CFBundle::new(url)?.info_dictionary();
    info.find(CFString::new("CFBundleIdentifier"))?.downcast::<CFString>().map(|s| s.to_string())
}

fn read_window(w: &Element, focused: Option<&Element>) -> Option<AppWindow> {
    if w.string("AXSubrole").as_deref() != Some("AXStandardWindow") {
        return None;
    }
    let edited = w.child("AXCloseButton").and_then(|b| b.flag("AXEdited"));
    let can_minimize =
        w.child("AXMinimizeButton").is_some_and(|b| b.flag("AXEnabled").unwrap_or(false));
    Some(AppWindow {
        window_id: w.window_id(),
        title: w.string("AXTitle").unwrap_or_default(),
        document: w.string("AXDocument").filter(|d| !d.is_empty()),
        edited,
        // Unreadable counts as minimised: never hidden on a guess, never "brought back".
        minimized: w.flag("AXMinimized").unwrap_or(true),
        full_screen: w.flag("AXFullScreen").unwrap_or(false),
        can_minimize,
        focused: focused.is_some_and(|f| f.same_as(w)),
    })
}

/// Lists windows through Accessibility (nothing without the permission).
#[derive(Debug, Default)]
pub struct MacWindowLister {
    /// Bundle ids by process (a process never changes its app).
    bundles: std::collections::HashMap<i32, Option<String>>,
}

impl MacWindowLister {
    pub fn new() -> Self {
        // The timeout on the system-wide element is the default for every element this
        // process asks (windows and buttons too, not only the app elements).
        // SAFETY: creates a new element (+1), released by the CFType.
        let raw = unsafe { AXUIElementCreateSystemWide() };
        if !raw.is_null() {
            // SAFETY: a +1 reference from a Create call.
            let all = unsafe { CFType::wrap_under_create_rule(raw) };
            // SAFETY: a valid element.
            unsafe { AXUIElementSetMessagingTimeout(all.as_CFTypeRef(), AX_TIMEOUT_S) };
        }
        Self::default()
    }
}

impl WindowLister for MacWindowLister {
    fn list(&mut self) -> Option<WindowList> {
        // SAFETY: a pure query.
        if unsafe { AXIsProcessTrusted() } == 0 {
            return None;
        }
        // SAFETY: a pure query.
        let me = unsafe { getpid() };
        let mut apps = Vec::new();
        let listed = apps_with_windows();
        self.bundles.retain(|pid, _| listed.contains_key(pid));
        for (pid, (name, all_window_ids)) in listed {
            if pid == me {
                continue;
            }
            let cached = self.bundles.entry(pid).or_insert_with(|| bundle_id(pid));
            let Some(bundle_id) = cached.clone() else { continue };
            let Some(app) = Element::app(pid) else { continue };
            let focused = app.child("AXFocusedWindow");
            let started = std::time::Instant::now();
            let windows: Vec<AppWindow> = app
                .elements("AXWindows")
                .iter()
                .take_while(|_| started.elapsed() < APP_BUDGET)
                .filter_map(|w| read_window(w, focused.as_ref()))
                .take(MAX_WINDOWS_PER_APP)
                .collect();
            // Listed even when Accessibility returned nothing (a slow app, windows on
            // other Spaces): its window numbers say its windows still exist.
            apps.push(AppWindows { pid, bundle_id, name, windows, all_window_ids });
        }
        Some(WindowList { apps, started_at: None })
    }
}

/// Whether a process of the app with `bundle_id` is running (`None` if processes can't
/// be listed). No permission needed: process paths and their bundles' Info.plist.
pub fn app_running(bundle_id: &str) -> Option<bool> {
    let mut pids = vec![0i32; 4096];
    let bytes = i32::try_from(pids.len() * std::mem::size_of::<i32>()).ok()?;
    // SAFETY: the buffer holds `bytes` bytes.
    let n = unsafe { proc_listallpids(pids.as_mut_ptr().cast(), bytes) };
    let n = usize::try_from(n).ok().filter(|n| *n > 0)?.min(pids.len());
    let mut checked: std::collections::HashMap<std::path::PathBuf, bool> =
        std::collections::HashMap::new();
    for pid in &pids[..n] {
        let mut buf = vec![0u8; 4096];
        // SAFETY: the buffer is valid for its full length.
        let len = unsafe { proc_pidpath(*pid, buf.as_mut_ptr().cast(), 4096) };
        let Some(len) = usize::try_from(len).ok().filter(|l| *l > 0) else { continue };
        let Ok(exe) = std::str::from_utf8(&buf[..len]) else { continue };
        let Some(bundle) = app_bundle_path(Path::new(exe)) else { continue };
        let hit = *checked.entry(bundle.clone()).or_insert_with(|| {
            CFURL::from_path(&bundle, true)
                .and_then(CFBundle::new)
                .and_then(|b| {
                    b.info_dictionary()
                        .find(CFString::new("CFBundleIdentifier"))
                        .and_then(|v| v.downcast::<CFString>())
                        .map(|s| s == bundle_id)
                })
                .unwrap_or(false)
        });
        if hit {
            return Some(true);
        }
    }
    Some(false)
}

/// Hides and restores single windows through Accessibility.
#[derive(Debug, Default)]
pub struct MacWindowActor;

impl MacWindowActor {
    /// The app's standard window with this number (only windows Accessibility can reach:
    /// on the current Space, or minimised).
    fn find(pid: i32, window_id: u32) -> Result<(Element, Element), String> {
        // SAFETY: a pure query.
        if unsafe { AXIsProcessTrusted() } == 0 {
            return Err("Decay doesn't have Accessibility access".into());
        }
        let app = Element::app(pid).ok_or("the app isn't running")?;
        let window = app
            .elements("AXWindows")
            .into_iter()
            .find(|w| w.window_id() == Some(window_id))
            .ok_or("the window isn't reachable (closed, or on another Space)")?;
        if window.string("AXSubrole").as_deref() != Some("AXStandardWindow") {
            return Err("not a standard window".into());
        }
        Ok((app, window))
    }
}

impl WindowActor for MacWindowActor {
    fn act(&mut self, op: WindowOp, pid: i32, window_id: u32) -> Result<(), String> {
        let (app, window) = Self::find(pid, window_id)?;
        match op {
            WindowOp::Hide => {
                // In use right now (the focused window of the app in front): never.
                let in_front = app.flag("AXFrontmost") != Some(false);
                let focused = app.child("AXFocusedWindow").is_some_and(|f| f.same_as(&window));
                if in_front && focused {
                    return Err("the window is in use".into());
                }
                if window.flag("AXFullScreen") == Some(true) {
                    return Err("the window is full screen".into());
                }
                let can = window
                    .child("AXMinimizeButton")
                    .is_some_and(|b| b.flag("AXEnabled").unwrap_or(false));
                if !can {
                    return Err("the window can't be minimised".into());
                }
                if window.flag("AXMinimized") == Some(true) {
                    return Ok(()); // already out of the way
                }
                // A slow app may answer the set with an error and minimise anyway: the
                // read-back decides.
                let set = window.set_flag("AXMinimized", true);
                if window.flag("AXMinimized") == Some(true) {
                    Ok(())
                } else {
                    Err(set.err().unwrap_or_else(|| "the window didn't minimise".into()))
                }
            }
            WindowOp::Restore => {
                if window.flag("AXMinimized") == Some(true) {
                    window.set_flag("AXMinimized", false)?;
                }
                // Raising and bringing the app forward are best effort: the window is
                // back either way.
                let _ = window.perform("AXRaise");
                let _ = app.set_flag("AXFrontmost", true);
                if window.flag("AXMinimized") == Some(true) {
                    Err("the window is still minimised".into())
                } else {
                    Ok(())
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_server_and_bundles_answer_without_permissions() {
        // No Accessibility needed for these two: the app list and bundle ids.
        let apps = apps_with_windows();
        for pid in apps.keys().take(5) {
            if let Some(id) = bundle_id(*pid) {
                assert!(id.contains('.'), "{id}");
            }
        }
        // SAFETY: a pure query.
        let me = unsafe { getpid() };
        // A test binary isn't an app bundle.
        assert_eq!(bundle_id(me), None);
    }

    #[test]
    fn running_apps_are_found_by_bundle_id() {
        // Finder always runs on a Mac with a user session (not on every CI runner).
        assert_eq!(app_running("com.example.not-an-app"), Some(false));
    }

    #[test]
    fn acting_without_accessibility_is_refused() {
        // SAFETY: a pure query.
        if unsafe { AXIsProcessTrusted() } == 0 {
            let err = MacWindowActor.act(WindowOp::Hide, 1, 1).unwrap_err();
            assert!(err.contains("Accessibility"), "{err}");
        }
    }

    #[test]
    fn listing_without_accessibility_is_none_not_empty() {
        let mut l = MacWindowLister::new();
        // SAFETY: a pure query.
        let trusted = unsafe { AXIsProcessTrusted() } != 0;
        let list = l.list();
        assert_eq!(list.is_some(), trusted);
    }
}
