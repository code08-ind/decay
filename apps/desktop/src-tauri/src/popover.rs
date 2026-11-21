//! The tray popover (Windows, UI Design Guidelines §8 / Designs/windows/02-tray): a left
//! click on the notification-area icon shows a small status window next to it (search,
//! counts, the last sweep with Undo, holds); a right click shows the native menu. It
//! hides when it loses focus, like the system's own flyouts. macOS keeps the menu.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::shell::{self, Shell};

/// The window label (the page picks its view by it).
pub const LABEL: &str = "popover";
/// Its size in logical px (the design's 360×500).
pub const SIZE: (f64, f64) = (360.0, 500.0);
/// Space between the popover and the taskbar or screen edge, logical px.
const GAP: f64 = 12.0;
/// A click on the icon within this long after a focus loss hid the popover is the click
/// that caused the loss: it closes the popover rather than showing it again.
const REOPEN_GRACE: Duration = Duration::from_millis(350);

/// When the popover last hid because it lost focus.
#[derive(Debug, Default)]
pub struct PopoverState {
    hidden_at: Mutex<Option<Instant>>,
}

/// A rectangle: x, y, width, height (one unit throughout: physical px).
pub type Rect = (f64, f64, f64, f64);

/// Where the popover goes (top-left), next to the tray icon and inside the work area
/// (the monitor less the taskbar). The taskbar's edge is where the icon lies outside the
/// work area; an auto-hidden taskbar (work area = monitor) opens towards the screen's
/// middle. `size` and `gap` in the same unit as the rectangles.
pub fn origin(icon: Rect, work: Rect, size: (f64, f64), gap: f64) -> (f64, f64) {
    let (ix, iy, iw, ih) = icon;
    let (wx, wy, ww, wh) = work;
    let (w, h) = size;
    let clamp_x = |x: f64| x.min(wx + ww - w - gap).max(wx + gap);
    let clamp_y = |y: f64| y.min(wy + wh - h - gap).max(wy + gap);
    let (cx, cy) = (ix + iw / 2.0, iy + ih / 2.0);
    if iy >= wy + wh {
        // Taskbar at the bottom (the usual place).
        (clamp_x(cx - w / 2.0), wy + wh - h - gap)
    } else if iy + ih <= wy {
        (clamp_x(cx - w / 2.0), wy + gap)
    } else if ix >= wx + ww {
        (wx + ww - w - gap, clamp_y(cy - h / 2.0))
    } else if ix + iw <= wx {
        (wx + gap, clamp_y(cy - h / 2.0))
    } else if cy > wy + wh / 2.0 {
        // Inside the work area (auto-hide): above the icon, or below it when it's high up.
        (clamp_x(cx - w / 2.0), clamp_y(iy - h - gap))
    } else {
        (clamp_x(cx - w / 2.0), clamp_y(iy + ih + gap))
    }
}

/// The page's query: Mica shows through on Windows 11 (see `shell::page_query`).
pub fn page_query(windows_build: Option<u32>) -> &'static str {
    match shell::chrome(crate::deeplink::Window::Settings, windows_build) {
        shell::Chrome::Custom { mica: true } => "?backdrop=mica",
        _ => "",
    }
}

fn build(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    let build = shell::windows_build();
    let url = WebviewUrl::App(format!("index.html{}#/", page_query(build)).into());
    let theme =
        *app.state::<Shell>().theme.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let builder = WebviewWindowBuilder::new(app, LABEL, url)
        .title("Decay")
        .inner_size(SIZE.0, SIZE.1)
        .theme(theme)
        .decorations(false)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        // The system's shadow and (Windows 11) rounded corners frame it.
        .shadow(true)
        .visible(false);
    shell::backdrop(builder, !page_query(build).is_empty(), tauri::window::Effect::Mica).build()
}

/// A left click on the tray icon at `icon` (physical px): shows the popover there, or
/// hides it if it's showing (or was just hidden by this very click).
pub fn toggle(app: &AppHandle, icon: Rect) {
    let just_hidden = lock(&app.state::<PopoverState>().hidden_at)
        .take()
        .is_some_and(|at| at.elapsed() < REOPEN_GRACE);
    let existing = app.get_webview_window(LABEL);
    if just_hidden || existing.as_ref().is_some_and(|w| w.is_visible().unwrap_or(false)) {
        if let Some(w) = existing {
            let _ = w.hide();
        }
        return;
    }
    let popover = match existing {
        Some(w) => w,
        None => match build(app) {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!("can't open the tray popover: {e}");
                return;
            }
        },
    };
    let (cx, cy) = (icon.0 + icon.2 / 2.0, icon.1 + icon.3 / 2.0);
    if let Ok(Some(monitor)) = app.monitor_from_point(cx, cy) {
        let scale = monitor.scale_factor();
        let area = monitor.work_area();
        let work = (
            f64::from(area.position.x),
            f64::from(area.position.y),
            f64::from(area.size.width),
            f64::from(area.size.height),
        );
        let size = (SIZE.0 * scale, SIZE.1 * scale);
        let (x, y) = origin(icon, work, size, GAP * scale);
        let _ = popover.set_position(tauri::PhysicalPosition::new(x, y));
    }
    let _ = popover.emit("popover:open", ());
    let _ = popover.show();
    let _ = popover.set_focus();
}

/// The popover lost focus: hide it (and remember when, for `toggle`).
pub fn blurred(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(LABEL)
        && w.is_visible().unwrap_or(false)
    {
        let _ = w.hide();
        *lock(&app.state::<PopoverState>().hidden_at) = Some(Instant::now());
    }
}

/// Hides the popover (it opened another window, or the panel).
pub fn hide(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(LABEL) {
        let _ = w.hide();
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: (f64, f64) = (360.0, 500.0);
    // A 1920×1080 monitor with a 48-px taskbar at the bottom.
    const WORK: Rect = (0.0, 0.0, 1920.0, 1032.0);

    #[test]
    fn sits_above_a_bottom_taskbar_centred_on_the_icon() {
        let icon = (1000.0, 1040.0, 32.0, 32.0);
        assert_eq!(origin(icon, WORK, SIZE, 12.0), (836.0, 520.0));
    }

    #[test]
    fn stays_on_screen_near_the_corner() {
        let icon = (1900.0, 1040.0, 32.0, 32.0);
        assert_eq!(origin(icon, WORK, SIZE, 12.0), (1548.0, 520.0));
        let icon = (0.0, 1040.0, 32.0, 32.0);
        assert_eq!(origin(icon, WORK, SIZE, 12.0), (12.0, 520.0));
    }

    #[test]
    fn follows_a_taskbar_on_any_edge() {
        // Top: work area starts below it.
        let top = (0.0, 48.0, 1920.0, 1032.0);
        assert_eq!(origin((1000.0, 8.0, 32.0, 32.0), top, SIZE, 12.0), (836.0, 60.0));
        // Right.
        let right = (0.0, 0.0, 1872.0, 1080.0);
        assert_eq!(origin((1880.0, 900.0, 32.0, 32.0), right, SIZE, 12.0), (1500.0, 568.0));
        // Left.
        let left = (48.0, 0.0, 1872.0, 1080.0);
        assert_eq!(origin((8.0, 100.0, 32.0, 32.0), left, SIZE, 12.0), (60.0, 12.0));
    }

    #[test]
    fn an_auto_hidden_taskbar_opens_towards_the_middle() {
        let full = (0.0, 0.0, 1920.0, 1080.0);
        assert_eq!(origin((1000.0, 1040.0, 32.0, 32.0), full, SIZE, 12.0), (836.0, 528.0));
        assert_eq!(origin((1000.0, 4.0, 32.0, 32.0), full, SIZE, 12.0), (836.0, 48.0));
    }

    #[test]
    fn a_second_monitor_at_negative_coordinates() {
        let work = (-1920.0, 0.0, 1920.0, 1032.0);
        assert_eq!(origin((-200.0, 1040.0, 32.0, 32.0), work, SIZE, 12.0), (-372.0, 520.0));
    }

    #[test]
    fn mica_only_on_windows_11() {
        assert_eq!(page_query(None), "");
        assert_eq!(page_query(Some(19_045)), "");
        assert_eq!(page_query(Some(22_631)), "?backdrop=mica");
    }
}
