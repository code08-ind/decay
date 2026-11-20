//! Windows app windows (P3.1): listing (`EnumWindows`, in z-order) and minimising /
//! restoring single windows (`ShowWindowAsync`, read back with `IsIconic`).
//!
//! An app window is what Alt+Tab shows ([`is_app_window`]). Windows on another virtual
//! desktop are cloaked: they aren't listed as windows, but their numbers are kept in
//! `all_window_ids` so the engine doesn't take them for closed (like macOS Spaces). The
//! app id is the executable's lower-cased file name (`chrome.exe`). Windows can't say
//! whether a window has unsaved changes or which document it shows: both stay unknown, so
//! such windows are only ever minimised, never closed. Window numbers are `HWND`s, which
//! are 32-bit values on 32- and 64-bit Windows alike.

#![allow(unsafe_code)]

use std::collections::HashMap;
use std::time::{Duration, Instant};

use ::windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, RECT};
use ::windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
use ::windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
};
use ::windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use ::windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GW_OWNER, GWL_EXSTYLE, GWL_STYLE, GetForegroundWindow, GetWindow, GetWindowLongW,
    GetWindowRect, GetWindowTextLengthW, GetWindowTextW, IsIconic, IsWindow, SW_RESTORE,
    SW_SHOWMINNOACTIVE, SetForegroundWindow, ShowWindowAsync,
};
use ::windows::core::BOOL;

use crate::win::{process_path, window_pid};
use crate::win_logic::{TopLevel, can_minimize, covers_monitor, exe_app, is_app_window};
use crate::windows::{AppWindow, AppWindows, WindowActor, WindowList, WindowLister, WindowOp};

/// How long a minimise or restore may take to show (the app's own thread does it).
const SETTLE: Duration = Duration::from_millis(800);

/// What one top-level window looks like now.
struct Seen {
    hwnd: HWND,
    pid: u32,
    top: TopLevel,
    title: String,
}

fn hwnd_of(id: u32) -> HWND {
    HWND(id as usize as *mut core::ffi::c_void)
}

fn id_of(hwnd: HWND) -> Option<u32> {
    u32::try_from(hwnd.0 as usize).ok()
}

fn read(hwnd: HWND) -> Option<Seen> {
    // SAFETY: plain queries on a window handle; a stale handle gives 0 / an error.
    unsafe {
        let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
        let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        let owned = GetWindow(hwnd, GW_OWNER).is_ok_and(|o| !o.is_invalid());
        let mut cloaked = 0u32;
        let cloaked = DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            (&raw mut cloaked).cast(),
            size_of::<u32>() as u32,
        )
        .is_ok()
            && cloaked != 0;
        let len = GetWindowTextLengthW(hwnd);
        let title = if len > 0 {
            let mut buf = vec![0u16; len as usize + 1];
            let n = GetWindowTextW(hwnd, &mut buf);
            String::from_utf16_lossy(&buf[..n.max(0) as usize])
        } else {
            String::new()
        };
        let pid = window_pid(hwnd)?;
        let has_title = !title.trim().is_empty();
        Some(Seen {
            hwnd,
            pid,
            top: TopLevel { style, ex_style, owned, cloaked, has_title },
            title,
        })
    }
}

fn minimized(hwnd: HWND) -> bool {
    // SAFETY: a plain query.
    unsafe { IsIconic(hwnd) }.as_bool()
}

fn full_screen(hwnd: HWND) -> bool {
    let mut rect = RECT::default();
    // SAFETY: valid out-pointers; failures leave the defaults (not full screen).
    unsafe {
        if GetWindowRect(hwnd, &raw mut rect).is_err() {
            return false;
        }
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut info =
            MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(monitor, &raw mut info).as_bool() {
            return false;
        }
        let m = info.rcMonitor;
        covers_monitor(
            (rect.left, rect.top, rect.right, rect.bottom),
            (m.left, m.top, m.right, m.bottom),
        )
    }
}

/// Every top-level window, front to back.
fn top_level() -> Vec<HWND> {
    unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: `lparam` is the Vec passed below, alive for the whole EnumWindows call.
        unsafe { (*(lparam.0 as *mut Vec<HWND>)).push(hwnd) };
        BOOL(1)
    }
    let mut all: Vec<HWND> = Vec::new();
    // SAFETY: the callback only pushes into `all`, which outlives the call.
    let _ = unsafe { EnumWindows(Some(collect), LPARAM((&raw mut all) as isize)) };
    all
}

/// Lists app windows by process.
#[derive(Debug, Default)]
pub struct WinWindowLister {
    /// Process id → (app id, name), as long as the process lives.
    apps: HashMap<u32, Option<(String, String)>>,
}

impl WinWindowLister {
    pub fn new() -> Self {
        Self::default()
    }
}

impl WindowLister for WinWindowLister {
    fn list(&mut self) -> Option<WindowList> {
        // SAFETY: a plain query.
        let front = unsafe { GetForegroundWindow() };
        let mut by_pid: HashMap<u32, AppWindows> = HashMap::new();
        let mut order: Vec<u32> = Vec::new();
        let mut live = std::collections::HashSet::new();
        for hwnd in top_level() {
            let Some(seen) = read(hwnd) else { continue };
            let Some(id) = id_of(seen.hwnd) else { continue };
            let shown_somewhere = is_app_window(&TopLevel { cloaked: false, ..seen.top });
            if !shown_somewhere {
                continue;
            }
            live.insert(seen.pid);
            let app = self
                .apps
                .entry(seen.pid)
                .or_insert_with(|| process_path(seen.pid).and_then(|p| exe_app(&p)))
                .clone();
            let Some((app_id, name)) = app else { continue };
            let entry = by_pid.entry(seen.pid).or_insert_with(|| {
                order.push(seen.pid);
                AppWindows {
                    pid: i32::try_from(seen.pid).unwrap_or(i32::MAX),
                    bundle_id: app_id,
                    name,
                    windows: Vec::new(),
                    all_window_ids: Vec::new(),
                }
            });
            entry.all_window_ids.push(id);
            // On another virtual desktop: it exists, but isn't shown or touched.
            if seen.top.cloaked {
                continue;
            }
            let is_min = minimized(seen.hwnd);
            // The app's focused window: the window in front, else its frontmost one (z-order).
            let focused = seen.hwnd == front
                || (front_pid(front) != Some(seen.pid)
                    && !is_min
                    && entry.windows.iter().all(|w| !w.focused && w.minimized));
            entry.windows.push(AppWindow {
                window_id: Some(id),
                title: seen.title,
                document: None,
                edited: None,
                minimized: is_min,
                full_screen: !is_min && full_screen(seen.hwnd),
                can_minimize: can_minimize(seen.top.style),
                focused,
            });
        }
        // Forget processes that are gone (a pid can be reused by another app).
        self.apps.retain(|pid, _| live.contains(pid));
        let apps = order.into_iter().filter_map(|pid| by_pid.remove(&pid)).collect();
        Some(WindowList { apps, started_at: None })
    }
}

fn front_pid(front: HWND) -> Option<u32> {
    (!front.is_invalid()).then(|| window_pid(front)).flatten()
}

/// Minimises and restores single windows.
#[derive(Debug, Default, Clone, Copy)]
pub struct WinWindowActor;

impl WindowActor for WinWindowActor {
    fn act(&mut self, op: WindowOp, pid: i32, window_id: u32) -> Result<(), String> {
        let hwnd = hwnd_of(window_id);
        // SAFETY: a plain query; any value is safe to pass.
        if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
            return Err("the window is gone".into());
        }
        if window_pid(hwnd).and_then(|p| i32::try_from(p).ok()) != Some(pid) {
            return Err("the window belongs to another process now".into());
        }
        match op {
            WindowOp::Hide => hide(hwnd),
            WindowOp::Restore => restore(hwnd),
        }
    }
}

fn hide(hwnd: HWND) -> Result<(), String> {
    let seen = read(hwnd).ok_or("the window can't be read")?;
    if !is_app_window(&seen.top) {
        return Err("not an app window".into());
    }
    if !can_minimize(seen.top.style) {
        return Err("the window can't be minimised".into());
    }
    if minimized(hwnd) {
        return Ok(());
    }
    if full_screen(hwnd) {
        return Err("the window is full screen".into());
    }
    // SAFETY: `front` is a plain query.
    if unsafe { GetForegroundWindow() } == hwnd {
        return Err("the window is in front".into());
    }
    // Minimised without activating anything: the window in front stays in front.
    // Asynchronous, so a hung app can't block Decay; the result is read back.
    // SAFETY: a valid window handle (checked above); the command is a constant.
    let _ = unsafe { ShowWindowAsync(hwnd, SW_SHOWMINNOACTIVE) };
    settle(|| minimized(hwnd)).then_some(()).ok_or_else(|| "the window didn't minimise".into())
}

fn restore(hwnd: HWND) -> Result<(), String> {
    // SAFETY: a valid window handle (checked by the caller).
    let _ = unsafe { ShowWindowAsync(hwnd, SW_RESTORE) };
    if !settle(|| !minimized(hwnd)) {
        return Err("the window didn't come back".into());
    }
    // Brought to the front if Windows allows it (it may flash in the taskbar instead).
    // SAFETY: a valid window handle.
    let _ = unsafe { SetForegroundWindow(hwnd) };
    Ok(())
}

/// Waits up to [`SETTLE`] for `done`.
fn settle(done: impl Fn() -> bool) -> bool {
    let start = Instant::now();
    loop {
        if done() {
            return true;
        }
        if start.elapsed() >= SETTLE {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// An app (by executable name, e.g. `chrome.exe`) has a running process.
pub fn app_running(exe: &str) -> Option<bool> {
    // SAFETY: a snapshot handle, closed below; the entry has its size set.
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).ok()?;
        let mut entry =
            PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut found = false;
        let mut more = Process32FirstW(snap, &raw mut entry).is_ok();
        while more {
            let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
            if String::from_utf16_lossy(&entry.szExeFile[..len]).eq_ignore_ascii_case(exe) {
                found = true;
                break;
            }
            more = Process32NextW(snap, &raw mut entry).is_ok();
        }
        let _ = CloseHandle(snap);
        Some(found)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// On a Windows runner: listing works (a service session may have no windows), the
    /// actor refuses windows that don't exist, and our own process is running.
    #[test]
    fn lists_acts_and_finds_processes() {
        let mut lister = WinWindowLister::new();
        let list = lister.list().expect("windows can always be listed");
        for app in &list.apps {
            assert_ne!(app.bundle_id, "");
            assert!(app.windows.len() <= app.all_window_ids.len());
            assert!(
                app.windows.iter().all(|w| w.window_id.is_some() && !w.title.trim().is_empty())
            );
        }
        let mut actor = WinWindowActor;
        assert_eq!(actor.act(WindowOp::Hide, 1, 0), Err("the window is gone".into()));
        assert_eq!(actor.act(WindowOp::Restore, 1, 0x7fff_fff0), Err("the window is gone".into()));
        let me = std::env::current_exe().unwrap();
        let exe = me.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(app_running(&exe), Some(true));
        assert_eq!(app_running("no-such-app-decay.exe"), Some(false));
        assert!(settle(|| true));
        assert!(!full_screen(HWND::default()));
        assert!(read(HWND::default()).is_none());
        assert_eq!(id_of(hwnd_of(42)), Some(42));
    }
}
