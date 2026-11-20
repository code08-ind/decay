//! Windows (P3.1): idle time (`GetLastInputInfo`), the session lock (WTS), the displays'
//! power state (a power-setting notification), the window in front, audio sessions on the
//! default output device, and a clock that runs through sleep (`GetTickCount64`).
//!
//! Everything but the display state is polled from the engine's sampling thread. The
//! display state only arrives as a window message, so one hidden message-only window on
//! its own thread listens for it (started with the first platform).
//!
//! Windows has no "time since the last key press" without a low-level keyboard hook, which
//! security tools flag as a keylogger: Decay never installs one. The time since any input
//! stands in for it — the cautious direction, as a sweep then waits for a minute with no
//! input at all. Minimising other apps' windows needs no permission on Windows, so
//! `accessibility` is always granted here.

#![allow(unsafe_code)]

use std::cell::RefCell;
use std::sync::Once;
use std::sync::atomic::{AtomicU32, Ordering};

use ::windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use ::windows::Win32::Media::Audio::{
    AudioSessionStateActive, IAudioSessionManager2, IMMDeviceEnumerator, MMDeviceEnumerator,
    eConsole, eRender,
};
use ::windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
};
use ::windows::Win32::System::LibraryLoader::GetModuleHandleW;
use ::windows::Win32::System::Power::{POWERBROADCAST_SETTING, RegisterPowerSettingNotification};
use ::windows::Win32::System::RemoteDesktop::{
    ProcessIdToSessionId, WTS_CURRENT_SESSION, WTSFreeMemory, WTSGetActiveConsoleSessionId,
    WTSINFOEXW, WTSQuerySessionInformationW, WTSSessionInfoEx,
};
use ::windows::Win32::System::SystemInformation::GetTickCount64;
use ::windows::Win32::System::SystemServices::GUID_CONSOLE_DISPLAY_STATE;
use ::windows::Win32::System::Threading::{
    GetCurrentProcessId, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    QueryFullProcessImageNameW,
};
use ::windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use ::windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DEVICE_NOTIFY_WINDOW_HANDLE, DefWindowProcW, DispatchMessageW,
    GetForegroundWindow, GetMessageW, GetWindowThreadProcessId, HWND_MESSAGE, MSG,
    PBT_POWERSETTINGCHANGE, RegisterClassW, WINDOW_EX_STYLE, WINDOW_STYLE, WM_POWERBROADCAST,
    WNDCLASSW,
};
use ::windows::core::{PWSTR, w};

use crate::win_logic::{display_asleep, exe_app, idle_secs, session_locked};
use crate::{FrontApp, Platform, PlatformSample};

/// The last `GUID_CONSOLE_DISPLAY_STATE` value (`UNKNOWN` until the first notification).
static DISPLAY_STATE: AtomicU32 = AtomicU32::new(UNKNOWN);
const UNKNOWN: u32 = u32::MAX;

/// Reads the OS state on Windows.
#[derive(Debug)]
pub struct WinPlatform;

impl WinPlatform {
    pub fn new() -> Self {
        start_display_watch();
        Self
    }
}

impl Default for WinPlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl Platform for WinPlatform {
    fn sample(&mut self) -> PlatformSample {
        let idle = idle();
        PlatformSample {
            idle_secs: idle,
            // No keyboard-only timer without a hook: any input counts (see the module docs).
            secs_since_key: idle,
            locked: locked(),
            display_asleep: display_asleep(DISPLAY_STATE.load(Ordering::Relaxed)),
            audio_playing: audio_playing(),
            // P3.5 (CapabilityAccessManager).
            mic_in_use: None,
            camera_in_use: None,
            frontmost: front_app(),
            accessibility: Some(true),
            continuous_ms: Some(now_ms()),
        }
    }

    fn request_accessibility(&mut self) -> bool {
        true
    }
}

/// Milliseconds since boot, sleep included; never moved by wall-clock changes.
fn now_ms() -> u64 {
    // SAFETY: a pure query with no arguments.
    unsafe { GetTickCount64() }
}

fn idle() -> Option<u64> {
    let mut info = LASTINPUTINFO { cbSize: size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
    // SAFETY: `info` is a valid LASTINPUTINFO with its size set, as the call requires.
    let ok = unsafe { GetLastInputInfo(&raw mut info) }.as_bool();
    ok.then(|| idle_secs(now_ms(), info.dwTime))
}

fn locked() -> Option<bool> {
    // SAFETY: a pure query with no arguments.
    let console = unsafe { WTSGetActiveConsoleSessionId() };
    let mut own = 0u32;
    // SAFETY: `own` is a valid out-pointer for the session id.
    let own =
        unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &raw mut own) }.ok().map(|()| own);
    session_locked(session_flags(), Some(console), own)
}

/// This session's `SessionFlags` (locked / unlocked).
fn session_flags() -> Option<i32> {
    let mut buf = PWSTR::null();
    let mut bytes = 0u32;
    // SAFETY: both out-pointers are valid; on success `buf` holds a WTSINFOEXW the system
    // allocated, which is freed below with WTSFreeMemory.
    unsafe {
        WTSQuerySessionInformationW(
            None,
            WTS_CURRENT_SESSION,
            WTSSessionInfoEx,
            &raw mut buf,
            &raw mut bytes,
        )
    }
    .ok()?;
    if buf.is_null() {
        return None;
    }
    let flags = if bytes as usize >= size_of::<WTSINFOEXW>() {
        // SAFETY: the buffer holds at least one WTSINFOEXW (size checked above), read
        // without assuming its alignment; level 1 is the union member the system fills.
        unsafe {
            let info = buf.0.cast::<WTSINFOEXW>().read_unaligned();
            (info.Level == 1).then_some(info.Data.WTSInfoExLevel1.SessionFlags)
        }
    } else {
        None
    };
    // SAFETY: `buf` came from WTSQuerySessionInformationW and is freed once.
    unsafe { WTSFreeMemory(buf.0.cast()) };
    flags
}

/// The full path of a process's executable.
pub(crate) fn process_path(pid: u32) -> Option<String> {
    // SAFETY: a query-only handle, closed below.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    // SAFETY: `buf` holds `len` u16s; the call writes at most that many and updates `len`.
    let ok = unsafe {
        QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &raw mut len,
        )
    };
    // SAFETY: the handle is ours and closed once.
    let _ = unsafe { CloseHandle(handle) };
    ok.ok()?;
    Some(String::from_utf16_lossy(&buf[..len.min(buf.len() as u32) as usize]))
}

/// The process that owns a window, `None` for no window.
pub(crate) fn window_pid(hwnd: HWND) -> Option<u32> {
    let mut pid = 0u32;
    // SAFETY: `pid` is a valid out-pointer; an invalid window gives 0.
    unsafe { GetWindowThreadProcessId(hwnd, Some(&raw mut pid)) };
    (pid != 0).then_some(pid)
}

fn front_app() -> Option<FrontApp> {
    // SAFETY: a pure query with no arguments.
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.is_invalid() {
        return None;
    }
    let pid = window_pid(hwnd)?;
    let (_, name) = exe_app(&process_path(pid)?)?;
    Some(FrontApp { pid: i32::try_from(pid).ok()?, name })
}

thread_local! {
    /// The audio device enumerator of this (sampling) thread.
    static AUDIO: RefCell<Option<IMMDeviceEnumerator>> = const { RefCell::new(None) };
}

/// Some app has an active audio session on the default output device (playing, or
/// holding it open: the engine caps OS audio at 2 h of idle for that).
fn audio_playing() -> Option<bool> {
    AUDIO.with(|cell| {
        let mut cell = cell.borrow_mut();
        if cell.is_none() {
            // SAFETY: joins this thread to the multithreaded apartment; S_FALSE (already)
            // and a different apartment both leave COM usable.
            let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            // SAFETY: creates the system's enumerator; failure is returned, not unsafe.
            *cell = unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }.ok();
        }
        let enumerator = cell.as_ref()?;
        // SAFETY: COM calls on a live interface; each result is checked.
        unsafe {
            // No output device at all: nothing can play.
            let Ok(device) = enumerator.GetDefaultAudioEndpoint(eRender, eConsole) else {
                return Some(false);
            };
            let manager: IAudioSessionManager2 = device.Activate(CLSCTX_ALL, None).ok()?;
            let sessions = manager.GetSessionEnumerator().ok()?;
            let count = sessions.GetCount().ok()?;
            for i in 0..count {
                let active = sessions
                    .GetSession(i)
                    .and_then(|s| s.GetState())
                    .is_ok_and(|state| state == AudioSessionStateActive);
                if active {
                    return Some(true);
                }
            }
            Some(false)
        }
    })
}

/// Starts the thread that hears display power changes (once per process).
fn start_display_watch() {
    static START: Once = Once::new();
    START.call_once(|| {
        // If it can't start or register, the display state just stays unknown (`None`):
        // the engine then relies on idle time and the lock, as on any failure.
        let _ = std::thread::Builder::new().name("decay-display-watch".into()).spawn(|| {
            let _ = watch_display();
        });
    });
}

/// A message-only window registered for `GUID_CONSOLE_DISPLAY_STATE`: the system sends the
/// current state right away, then every change. Runs until the process ends.
fn watch_display() -> ::windows::core::Result<()> {
    let class = w!("DecayDisplayWatch");
    // SAFETY: this module's handle; registering a class with a valid window procedure and
    // a static name.
    let instance = unsafe { GetModuleHandleW(None) }?;
    let wc = WNDCLASSW {
        lpfnWndProc: Some(display_proc),
        hInstance: instance.into(),
        lpszClassName: class,
        ..Default::default()
    };
    // SAFETY: `wc` is fully initialised and outlives the call.
    if unsafe { RegisterClassW(&raw const wc) } == 0 {
        return Err(::windows::core::Error::from_thread());
    }
    // SAFETY: a message-only window (HWND_MESSAGE parent) of the class registered above.
    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            w!(""),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(instance.into()),
            None,
        )
    }?;
    // SAFETY: registers that window for the display-state setting; the GUID is static.
    unsafe {
        RegisterPowerSettingNotification(
            HANDLE(hwnd.0),
            &GUID_CONSOLE_DISPLAY_STATE,
            DEVICE_NOTIFY_WINDOW_HANDLE,
        )
    }?;
    let mut msg = MSG::default();
    // SAFETY: the standard message loop of the thread that owns the window.
    while unsafe { GetMessageW(&raw mut msg, None, 0, 0) }.as_bool() {
        // SAFETY: dispatching the message just received.
        unsafe { DispatchMessageW(&raw const msg) };
    }
    Ok(())
}

extern "system" fn display_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_POWERBROADCAST && wparam.0 == PBT_POWERSETTINGCHANGE as usize && lparam.0 != 0 {
        // SAFETY: for PBT_POWERSETTINGCHANGE, lparam points to a POWERBROADCAST_SETTING
        // followed by `DataLength` bytes of data.
        let state = unsafe {
            let setting = &*(lparam.0 as *const POWERBROADCAST_SETTING);
            (setting.PowerSetting == GUID_CONSOLE_DISPLAY_STATE && setting.DataLength >= 4)
                .then(|| setting.Data.as_ptr().cast::<u32>().read_unaligned())
        };
        if let Some(state) = state {
            DISPLAY_STATE.store(state, Ordering::Relaxed);
        }
        return LRESULT(1);
    }
    // SAFETY: everything else gets the default handling.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// On a Windows runner: idle time, the clock, the session and our own process path
    /// are always readable; audio and the front window may be unknown on a headless VM.
    #[test]
    fn samples_the_machine() {
        let mut p = WinPlatform::new();
        let s = p.sample();
        assert!(s.idle_secs.is_some(), "{s:?}");
        assert_eq!(s.secs_since_key, s.idle_secs);
        assert!(s.continuous_ms.is_some_and(|ms| ms > 0));
        assert!(s.locked.is_some(), "{s:?}");
        assert_eq!(s.accessibility, Some(true));
        assert!(p.request_accessibility());
        let _ = s.audio_playing;
        // SAFETY: a pure query.
        let me = unsafe { GetCurrentProcessId() };
        let path = process_path(me).expect("our own path");
        assert!(path.to_lowercase().ends_with(".exe"), "{path}");
        assert_eq!(window_pid(HWND::default()), None);
        assert_eq!(process_path(0), None, "the idle process has no image");
        // The watch thread started once; a second platform doesn't start another.
        let _ = <WinPlatform as Default>::default();
    }
}
