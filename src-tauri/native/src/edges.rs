//! Resize edges a mouse can actually find.
//!
//! Undecorated with the system shadow, winit gives the client area the whole
//! window (`WM_NCCALCSIZE`), so Windows' own hit test leaves a resize border
//! about one pixel wide. This subclass answers `WM_NCHITTEST` for a band along
//! each edge and a larger square in each corner, as framed windows have; the
//! rest of the window stays client area for Slint.

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongPtrW, GetWindowRect, IsZoomed, GWL_STYLE, HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT,
    HTLEFT, HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT, WM_NCDESTROY, WM_NCHITTEST, WS_THICKFRAME,
};

/// The resize band along an edge, in logical pixels.
const EDGE: i32 = 6;
/// How far from a corner, along either edge, the diagonal resize reaches.
const CORNER: i32 = 14;
/// This subclass's id on a window.
const SUBCLASS_ID: usize = 0x5245_4247; // "REBG"

/// Widens the resize border of `hwnd`. Installing it again is a no-op.
pub fn install(hwnd: HWND) {
    // FFI: our own live window; the subclass removes itself on WM_NCDESTROY.
    let ok = unsafe { SetWindowSubclass(hwnd, Some(subclass), SUBCLASS_ID, 0) };
    if !ok.as_bool() {
        tracing::warn!("could not widen the window's resize edges");
    }
}

unsafe extern "system" fn subclass(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    match msg {
        WM_NCHITTEST => {
            if let Some(hit) = unsafe { edge_hit(hwnd, lparam) } {
                return LRESULT(hit as isize);
            }
        }
        WM_NCDESTROY => {
            let _ = unsafe { RemoveWindowSubclass(hwnd, Some(subclass), SUBCLASS_ID) };
        }
        _ => {}
    }
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

/// The resize zone under the cursor (`lparam`, screen coordinates), if any.
unsafe fn edge_hit(hwnd: HWND, lparam: LPARAM) -> Option<u32> {
    let resizable = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) } & WS_THICKFRAME.0 as isize != 0;
    if !resizable || unsafe { IsZoomed(hwnd) }.as_bool() {
        return None;
    }
    let mut rect = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut rect) }.ok()?;
    let x = i32::from((lparam.0 & 0xffff) as u16 as i16);
    let y = i32::from(((lparam.0 >> 16) & 0xffff) as u16 as i16);
    let dpi = match unsafe { GetDpiForWindow(hwnd) } {
        0 => 96,
        d => d as i32,
    };
    let edge = EDGE * dpi / 96;
    let corner = CORNER * dpi / 96;
    zone(&rect, x, y, edge, corner)
}

/// Which resize zone of `rect` the point falls in: `edge` pixels in from each
/// side, widened to `corner` pixels along the sides near each corner.
fn zone(rect: &RECT, x: i32, y: i32, edge: i32, corner: i32) -> Option<u32> {
    if x < rect.left || x >= rect.right || y < rect.top || y >= rect.bottom {
        return None;
    }
    let near_left = x < rect.left + edge;
    let near_right = x >= rect.right - edge;
    let near_top = y < rect.top + edge;
    let near_bottom = y >= rect.bottom - edge;
    if !(near_left || near_right || near_top || near_bottom) {
        return None;
    }
    let corner_left = x < rect.left + corner;
    let corner_right = x >= rect.right - corner;
    let corner_top = y < rect.top + corner;
    let corner_bottom = y >= rect.bottom - corner;
    Some(match () {
        _ if corner_top && corner_left => HTTOPLEFT,
        _ if corner_top && corner_right => HTTOPRIGHT,
        _ if corner_bottom && corner_left => HTBOTTOMLEFT,
        _ if corner_bottom && corner_right => HTBOTTOMRIGHT,
        _ if near_left => HTLEFT,
        _ if near_right => HTRIGHT,
        _ if near_top => HTTOP,
        _ => HTBOTTOM,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: RECT = RECT {
        left: 100,
        top: 100,
        right: 900,
        bottom: 700,
    };

    #[test]
    fn edges_and_corners() {
        assert_eq!(zone(&R, 103, 400, 6, 14), Some(HTLEFT));
        assert_eq!(zone(&R, 896, 400, 6, 14), Some(HTRIGHT));
        assert_eq!(zone(&R, 500, 105, 6, 14), Some(HTTOP));
        assert_eq!(zone(&R, 500, 699, 6, 14), Some(HTBOTTOM));
        // On an edge but within the corner's reach: diagonal.
        assert_eq!(zone(&R, 102, 110, 6, 14), Some(HTTOPLEFT));
        assert_eq!(zone(&R, 890, 698, 6, 14), Some(HTBOTTOMRIGHT));
    }

    #[test]
    fn the_inside_and_the_outside_are_left_alone() {
        assert_eq!(zone(&R, 110, 110, 6, 14), None);
        assert_eq!(zone(&R, 500, 400, 6, 14), None);
        assert_eq!(zone(&R, 99, 400, 6, 14), None);
        assert_eq!(zone(&R, 900, 400, 6, 14), None);
    }
}
