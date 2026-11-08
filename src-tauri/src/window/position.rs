//! Popup, tray-menu and settings-window placement for the WebView windows.
//!
//! OWNER: worker W3. The geometry lives in `rebuffer_core::placement`, shared
//! with the native front end; this file only turns a Tauri window into the
//! HWND and sizes that code speaks.

use tauri::WebviewWindow;
use windows::Win32::Foundation::POINT;
use windows::Win32::Graphics::Gdi::{MonitorFromPoint, MONITOR_DEFAULTTONEAREST};

use crate::error::{AppError, AppResult};
use crate::settings::WindowSettings;

use rebuffer_core::placement::{
    clamp_pos_to_work, clamp_size_to_work, cursor_point, set_rect, work_area,
};
pub use rebuffer_core::placement::{drag_bar_pixels, DRAG_BAR_HEIGHT};

/// Sizes the popup per `settings.window` and centers it on the cursor; see
/// `rebuffer_core::placement::place_popup`.
pub fn place_popup(window: &WebviewWindow, ws: &WindowSettings) -> AppResult<()> {
    rebuffer_core::placement::place_popup(window.hwnd().map_err(tauri_err)?, ws)
}

/// Places a window at the cursor without resizing it, clamped so the whole
/// thing stays on the monitor under the pointer. Used by the tray menu, which
/// has a fixed size and should appear where the click happened.
pub fn place_at_cursor(window: &WebviewWindow) -> AppResult<()> {
    place_at(window, cursor_point()?)
}

/// The cursor position now, in physical pixels of the virtual screen. Callers
/// that build a window before they can place it capture the point first, so
/// the window still lands where the click was rather than where the pointer
/// has drifted to by the time it exists.
pub fn cursor_pos() -> AppResult<POINT> {
    cursor_point()
}

/// `place_at_cursor` for a point that was captured earlier.
pub fn place_at(window: &WebviewWindow, pt: POINT) -> AppResult<()> {
    let hwnd = window.hwnd().map_err(tauri_err)?;
    // FFI: MonitorFromPoint never fails and returns a valid HMONITOR.
    let monitor = unsafe { MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST) };
    let (work, _scale) = work_area(monitor)?;

    let size = window.outer_size().map_err(tauri_err)?;
    let (w, h) = (size.width, size.height);
    // A tray click is near an edge by definition, so the clamp is doing real
    // work here rather than guarding an unlikely case.
    let (x, y) = clamp_pos_to_work(pt.x, pt.y, w as i32, h as i32, &work);
    set_rect(hwnd, x, y, w, h)
}

/// Centers the window on the work area of the monitor under the cursor,
/// preserving its current logical size. Used by the settings window.
pub fn center_on_cursor_monitor(window: &WebviewWindow) -> AppResult<()> {
    let hwnd = window.hwnd().map_err(tauri_err)?;
    let pt = cursor_point()?;
    // FFI: MonitorFromPoint never fails and returns a valid HMONITOR.
    let monitor = unsafe { MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST) };
    let (work, scale) = work_area(monitor)?;

    // The window reports its size in physical pixels for *its current* monitor;
    // rescale to the target monitor so 960x660 stays 960x660 logical. The size
    // is then clamped to the work area so a small monitor never gets a window
    // that hangs off its edges (the centering formula is in-bounds once the
    // size fits).
    let size = window.inner_size().map_err(tauri_err)?;
    let current = window.scale_factor().map_err(tauri_err)?;
    let w = (size.width as f64 / current * scale).round() as i32;
    let h = (size.height as f64 / current * scale).round() as i32;
    let (w, h) = clamp_size_to_work(w.max(1) as u32, h.max(1) as u32, &work);
    let x = work.left + ((work.right - work.left) - w as i32) / 2;
    let y = work.top + ((work.bottom - work.top) - h as i32) / 2;
    set_rect(hwnd, x, y, w, h)
}

fn tauri_err(e: tauri::Error) -> AppError {
    AppError::Other(e.to_string())
}
