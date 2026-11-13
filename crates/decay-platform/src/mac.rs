//! macOS: `CoreGraphics` (idle time, session lock, displays), the Process Manager (the active app), `CoreAudio`
//! (the default output device in use) and Accessibility (`AXIsProcessTrusted`).
//!
//! Every call here is a plain C function of a system framework. They are thread-safe and
//! need no run loop, so the engine thread can call them. The active app comes from the
//! Process Manager (PID and process name only): no window titles, no Screen Recording.

#![allow(unsafe_code)]

use std::ffi::c_void;

use core_foundation::base::{CFType, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::number::CFNumber;
use core_foundation::string::{CFString, CFStringRef};

use crate::{FrontApp, Platform, PlatformSample};

type CGDirectDisplayId = u32;

/// `kCGEventSourceStateCombinedSessionState`: input from every source in this session.
const COMBINED_SESSION_STATE: i32 = 0;
/// `kCGAnyInputEventType`.
const ANY_INPUT_EVENT: u32 = u32::MAX;
/// `kCGEventKeyDown`.
const KEY_DOWN_EVENT: u32 = 10;

#[repr(C)]
struct AudioObjectPropertyAddress {
    selector: u32,
    scope: u32,
    element: u32,
}

const fn fourcc(code: [u8; 4]) -> u32 {
    u32::from_be_bytes(code)
}
const AUDIO_SYSTEM_OBJECT: u32 = 1;
const AUDIO_DEFAULT_OUTPUT_DEVICE: u32 = fourcc(*b"dOut");
const AUDIO_RUNNING_SOMEWHERE: u32 = fourcc(*b"gone");
const AUDIO_SCOPE_GLOBAL: u32 = fourcc(*b"glob");
const AUDIO_ELEMENT_MAIN: u32 = 0;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventSourceSecondsSinceLastEventType(state: i32, event_type: u32) -> f64;
    fn CGSessionCopyCurrentDictionary() -> CFDictionaryRef;
    fn CGMainDisplayID() -> CGDirectDisplayId;
    fn CGDisplayIsAsleep(display: CGDirectDisplayId) -> u32;
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> u8;
    fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> u8;
    static kAXTrustedCheckOptionPrompt: CFStringRef;
}

#[repr(C)]
struct ProcessSerialNumber {
    high: u32,
    low: u32,
}

// Deprecated since macOS 10.9 but still provided (also on Apple silicon); the only
// public call that names the active app without a run loop or a permission.
#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn GetFrontProcess(psn: *mut ProcessSerialNumber) -> i16;
    fn GetProcessPID(psn: *const ProcessSerialNumber, pid: *mut i32) -> i32;
}

// libproc and the clocks (libSystem).
unsafe extern "C" {
    fn proc_name(pid: i32, buffer: *mut c_void, size: u32) -> i32;
    fn clock_gettime_nsec_np(clock_id: u32) -> u64;
}

/// `CLOCK_MONOTONIC` on macOS: keeps counting through sleep, never set back or forward.
const CLOCK_MONOTONIC: u32 = 6;

/// The continuous clock in milliseconds, or `None` if it can't be read (the call returns 0).
fn continuous_ms() -> Option<u64> {
    // SAFETY: a pure query with a valid clock id; no pointers involved.
    let ns = unsafe { clock_gettime_nsec_np(CLOCK_MONOTONIC) };
    (ns != 0).then_some(ns / 1_000_000)
}

#[link(name = "CoreAudio", kind = "framework")]
unsafe extern "C" {
    fn AudioObjectGetPropertyData(
        object: u32,
        address: *const AudioObjectPropertyAddress,
        qualifier_size: u32,
        qualifier: *const c_void,
        data_size: *mut u32,
        data: *mut c_void,
    ) -> i32;
}

/// Seconds since the last event of `event_type`, or `None` if the answer isn't usable.
fn seconds_since(event_type: u32) -> Option<u64> {
    // SAFETY: a pure query with plain integer arguments; no pointers involved.
    let secs =
        unsafe { CGEventSourceSecondsSinceLastEventType(COMBINED_SESSION_STATE, event_type) };
    (secs.is_finite() && secs >= 0.0).then_some(secs as u64)
}

/// A flag in the session dictionary. macOS has stored some of them as `CFBoolean` and
/// others as a `CFNumber` 0 / 1, so both are accepted.
fn bool_value(dict: &CFDictionary<CFString, CFType>, key: &str) -> Option<bool> {
    let value = dict.find(CFString::new(key))?;
    if let Some(b) = value.downcast::<CFBoolean>() {
        return Some(b.into());
    }
    value.downcast::<CFNumber>().and_then(|n| n.to_i64()).map(|n| n != 0)
}

/// Locked, or not this user's session on the console (fast user switching). `None`
/// outside a GUI session (e.g. over SSH or on a headless runner).
fn session_locked() -> Option<bool> {
    // SAFETY: returns a new reference (Create rule) or null; null is checked before use.
    let raw = unsafe { CGSessionCopyCurrentDictionary() };
    if raw.is_null() {
        return None;
    }
    // SAFETY: `raw` is a non-null CFDictionary we own (Create rule); wrapping it transfers
    // that ownership, so it's released exactly once.
    let dict: CFDictionary<CFString, CFType> = unsafe { CFDictionary::wrap_under_create_rule(raw) };
    let locked = bool_value(&dict, "CGSSessionScreenIsLocked").unwrap_or(false);
    let on_console = bool_value(&dict, "kCGSSessionOnConsoleKey").unwrap_or(true);
    Some(locked || !on_console)
}

fn display_asleep() -> bool {
    // SAFETY: plain integer queries about the main display.
    unsafe { CGDisplayIsAsleep(CGMainDisplayID()) != 0 }
}

/// The active app (the one whose menu bar is shown), from the Process Manager: it answers
/// live without a run loop or any permission, unlike the window list (an active app may
/// have no window in front; Stage Manager owns some) or `NSWorkspace` (only updated by a
/// main run loop).
fn frontmost_app() -> Option<FrontApp> {
    let mut psn = ProcessSerialNumber { high: 0, low: 0 };
    // SAFETY: `psn` is a valid out-parameter for the call.
    if unsafe { GetFrontProcess(&raw mut psn) } != 0 {
        return None;
    }
    let mut pid: i32 = -1;
    // SAFETY: `psn` was filled in above; `pid` is a valid out-parameter.
    if unsafe { GetProcessPID(&raw const psn, &raw mut pid) } != 0 || pid <= 0 {
        return None;
    }
    let mut buf = [0u8; 256];
    // SAFETY: `buf` is writable for its full length, which is passed as the size.
    let len = unsafe { proc_name(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
    let name = usize::try_from(len)
        .ok()
        .and_then(|n| buf.get(..n))
        .map(|b| String::from_utf8_lossy(b).into_owned())
        .unwrap_or_default();
    Some(FrontApp { pid, name })
}

/// Reads a `u32` property of a `CoreAudio` object.
fn audio_u32(object: u32, selector: u32) -> Option<u32> {
    let address = AudioObjectPropertyAddress {
        selector,
        scope: AUDIO_SCOPE_GLOBAL,
        element: AUDIO_ELEMENT_MAIN,
    };
    let mut value: u32 = 0;
    let mut size = u32::try_from(std::mem::size_of::<u32>()).ok()?;
    // SAFETY: `address` and `value` live for the call; `size` says how much may be written
    // to `value` (4 bytes, its size); no qualifier is passed (size 0, null pointer).
    let status = unsafe {
        AudioObjectGetPropertyData(
            object,
            &raw const address,
            0,
            std::ptr::null(),
            &raw mut size,
            (&raw mut value).cast::<c_void>(),
        )
    };
    (status == 0 && size as usize == std::mem::size_of::<u32>()).then_some(value)
}

/// The default output device is running for some process (audio or video playing).
fn audio_playing() -> Option<bool> {
    let device = audio_u32(AUDIO_SYSTEM_OBJECT, AUDIO_DEFAULT_OUTPUT_DEVICE)?;
    if device == 0 {
        return Some(false); // no output device
    }
    audio_u32(device, AUDIO_RUNNING_SOMEWHERE).map(|running| running != 0)
}

fn accessibility_trusted() -> bool {
    // SAFETY: no arguments; reads this process's trust state.
    unsafe { AXIsProcessTrusted() != 0 }
}

/// The macOS implementation of [`Platform`].
#[derive(Debug, Default)]
pub struct MacPlatform;

impl MacPlatform {
    pub fn new() -> Self {
        Self
    }
}

impl Platform for MacPlatform {
    fn sample(&mut self) -> PlatformSample {
        PlatformSample {
            idle_secs: seconds_since(ANY_INPUT_EVENT),
            secs_since_key: seconds_since(KEY_DOWN_EVENT),
            locked: session_locked(),
            display_asleep: Some(display_asleep()),
            audio_playing: audio_playing(),
            frontmost: frontmost_app(),
            accessibility: Some(accessibility_trusted()),
            continuous_ms: continuous_ms(),
        }
    }

    fn request_accessibility(&mut self) -> bool {
        // SAFETY: `kAXTrustedCheckOptionPrompt` is a constant CFString owned by the
        // framework; Get rule: we take our own reference.
        let key = unsafe { CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt) };
        let options = CFDictionary::from_CFType_pairs(&[(
            key.as_CFType(),
            CFBoolean::true_value().as_CFType(),
        )]);
        // SAFETY: `options` is a valid CFDictionary that outlives the call.
        unsafe { AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef()) != 0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_machine_state() {
        let s = MacPlatform::new().sample();
        assert!(s.idle_secs.is_some());
        assert!(s.display_asleep.is_some());
        assert!(s.accessibility.is_some());
        assert!(s.continuous_ms.is_some_and(|ms| ms > 0));
        // A GUI session has a frontmost window; a headless runner may not.
        if let Some(front) = &s.frontmost {
            assert!(front.pid > 0);
        }
    }

    #[test]
    fn session_flags_may_be_booleans_or_numbers() {
        let dict: CFDictionary<CFString, CFType> = CFDictionary::from_CFType_pairs(&[
            (CFString::new("bool"), CFBoolean::true_value().as_CFType()),
            (CFString::new("one"), CFNumber::from(1i32).as_CFType()),
            (CFString::new("zero"), CFNumber::from(0i32).as_CFType()),
            (CFString::new("text"), CFString::new("yes").as_CFType()),
        ]);
        assert_eq!(bool_value(&dict, "bool"), Some(true));
        assert_eq!(bool_value(&dict, "one"), Some(true));
        assert_eq!(bool_value(&dict, "zero"), Some(false));
        assert_eq!(bool_value(&dict, "text"), None);
        assert_eq!(bool_value(&dict, "missing"), None);
    }

    #[test]
    fn the_continuous_clock_moves_forward() {
        let a = continuous_ms().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert!(continuous_ms().unwrap() >= a + 10);
    }

    #[test]
    fn fourcc_codes_match_coreaudio() {
        assert_eq!(AUDIO_DEFAULT_OUTPUT_DEVICE, 0x644F_7574);
        assert_eq!(AUDIO_SCOPE_GLOBAL, 0x676C_6F62);
    }
}
