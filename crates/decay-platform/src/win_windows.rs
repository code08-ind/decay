//! Windows app windows (P3.1): listing (`EnumWindows`, in z-order) and minimising /
//! restoring single windows (`ShowWindowAsync`, read back with `IsIconic`).
//!
//! An app window is what Alt+Tab shows ([`is_app_window`]). Windows on another virtual
//! desktop are cloaked: they aren't listed as windows, but their numbers are kept in
//! `all_window_ids` so the engine doesn't take them for closed (like macOS Spaces). The
//! app id is the executable's lower-cased file name (`chrome.exe`). Windows can't say
//! whether a window has unsaved changes or which document it shows: both stay unknown, so
//! such windows are only ever minimised, never closed. The exception (P4.1) is Explorer:
//! the Shell names the folder each of its windows shows, and a folder has nothing to save,
//! so a one-tab folder window may be closed (`WindowOp::Close`) and reopened. Window
//! numbers are `HWND`s, which are 32-bit values on 32- and 64-bit Windows alike.

#![allow(unsafe_code)]

use std::collections::HashMap;
use std::time::{Duration, Instant};

use ::windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, RECT, WPARAM};
use ::windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
use ::windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
};
use ::windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
};
use ::windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use ::windows::Win32::System::Variant::VARIANT;
use ::windows::Win32::UI::Shell::{IShellWindows, IWebBrowser2, ShellWindows};
use ::windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GW_OWNER, GWL_EXSTYLE, GWL_STYLE, GetForegroundWindow, GetWindow, GetWindowLongW,
    GetWindowRect, GetWindowTextLengthW, GetWindowTextW, IsIconic, IsWindow, PostMessageW,
    SW_RESTORE, SW_SHOWMINNOACTIVE, SetForegroundWindow, ShowWindowAsync, WM_CLOSE,
};
use ::windows::core::{BOOL, Interface};

use crate::win::{process_path, window_pid};
use crate::win_logic::{
    TopLevel, can_minimize, covers_monitor, exe_app, explorer_folders, is_app_window,
};
use crate::windows::{
    AppWindow, AppWindows, WindowActor, WindowList, WindowLister, WindowOp, file_url_path,
};

/// Explorer's executable (its app id): the only app whose windows may be closed (P4.1).
const EXPLORER: &str = "explorer.exe";
/// How long a window may take to close.
const CLOSE_SETTLE: Duration = Duration::from_millis(1500);

/// The folder each Explorer window shows (`file://` URL), by window; see
/// [`explorer_folders`]. Empty when the Shell can't be asked.
fn shell_folders() -> HashMap<isize, Option<String>> {
    // SAFETY: COM for this thread (a second call on the same thread is harmless).
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    // SAFETY: a COM object created and released by its smart pointer.
    let Ok(shell) =
        (unsafe { CoCreateInstance::<_, IShellWindows>(&ShellWindows, None, CLSCTX_ALL) })
    else {
        return HashMap::new();
    };
    let mut tabs = Vec::new();
    // SAFETY: calls on a live COM object; every result is checked.
    unsafe {
        for i in 0..shell.Count().unwrap_or(0) {
            let Ok(item) = shell.Item(&VARIANT::from(i)) else { continue };
            let Ok(browser) = item.cast::<IWebBrowser2>() else { continue };
            let Ok(hwnd) = browser.HWND() else { continue };
            let url = browser.LocationURL().map(|u| u.to_string()).unwrap_or_default();
            tabs.push((hwnd.0, url));
        }
    }
    explorer_folders(&tabs)
}

/// An Explorer window's folder, when it shows exactly one that exists.
fn folder_url(folders: &HashMap<isize, Option<String>>, hwnd: HWND) -> Option<String> {
    folders
        .get(&(hwnd.0 as isize))
        .cloned()
        .flatten()
        .filter(|u| file_url_path(u).is_some_and(|p| p.is_dir()))
}

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
        // Asked once per list, and only when an Explorer window is listed.
        let mut folders: Option<HashMap<isize, Option<String>>> = None;
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
            // An Explorer folder window: its folder, and nothing unsaved (P4.1).
            let folder = (entry.bundle_id == EXPLORER)
                .then(|| folder_url(folders.get_or_insert_with(shell_folders), seen.hwnd))
                .flatten();
            let edited = folder.is_some().then_some(false);
            entry.windows.push(AppWindow {
                window_id: Some(id),
                title: seen.title,
                document: folder,
                edited,
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
            WindowOp::Close => close_folder(hwnd),
        }
    }
}

/// Closes an Explorer window showing one existing folder (nothing else), and waits until
/// it's gone.
fn close_folder(hwnd: HWND) -> Result<(), String> {
    let app = window_pid(hwnd).and_then(process_path).and_then(|p| exe_app(&p));
    if app.map(|(id, _)| id).as_deref() != Some(EXPLORER) {
        return Err("only Explorer's folder windows are closed".into());
    }
    if folder_url(&shell_folders(), hwnd).is_none() {
        return Err("the window doesn't show one folder that can be reopened".into());
    }
    // SAFETY: posting a message to a window; any handle value is safe to pass.
    unsafe { PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)) }
        .map_err(|e| e.to_string())?;
    let start = Instant::now();
    // SAFETY: a plain query.
    while unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        if start.elapsed() >= CLOSE_SETTLE {
            return Err("the window didn't close".into());
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    Ok(())
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

/// Asks the window manager for rounded corners on a window of Decay's own (Windows 11:
/// frameless windows that can't be resized are otherwise left square). `hwnd` is the
/// window's handle as a number. False where it isn't supported (Windows 10).
pub fn round_corners(hwnd: isize) -> bool {
    use ::windows::Win32::Graphics::Dwm::{
        DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute,
    };
    let preference = DWMWCP_ROUND;
    // SAFETY: the attribute is a `DWM_WINDOW_CORNER_PREFERENCE` passed by pointer with its
    // size; an invalid or foreign `hwnd` only makes the call fail.
    unsafe {
        DwmSetWindowAttribute(
            HWND(hwnd as *mut core::ffi::c_void),
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&raw const preference).cast(),
            size_of_val(&preference) as u32,
        )
        .is_ok()
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
        assert_eq!(actor.act(WindowOp::Close, 1, 0), Err("the window is gone".into()));
        // Explorer folders: whatever the Shell lists (a service session: maybe nothing) is
        // one entry per window; a window that isn't Explorer's is never closed.
        let folders = shell_folders();
        assert!(folders.values().flatten().all(|u| u.starts_with("file:///")), "{folders:?}");
        assert_eq!(folder_url(&folders, HWND::default()), None);
        let not_explorer = top_level().into_iter().find(|h| {
            window_pid(*h)
                .and_then(process_path)
                .and_then(|p| exe_app(&p))
                .is_some_and(|(id, _)| id != EXPLORER)
        });
        if let Some(h) = not_explorer {
            assert_eq!(close_folder(h), Err("only Explorer's folder windows are closed".into()));
        }
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
