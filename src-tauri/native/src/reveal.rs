//! Showing a window without the flash of an empty frame.
//!
//! A GPU window is on screen before its first frame is drawn, and Windows
//! fills that gap with white. The WebView build revealed its windows only
//! once the page had painted; this does the same with DWM's cloak: the window
//! is shown cloaked, and uncloaked after the renderer has finished a frame.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use slint::RenderingState;
use windows::core::BOOL;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_CLOAK};

/// The longest a window stays cloaked if no frame is reported (the software
/// renderer has no rendering notifier): long enough for one frame.
const FALLBACK: Duration = Duration::from_millis(40);
/// After a frame is drawn, the swap that puts it on screen.
const AFTER_FRAME: Duration = Duration::from_millis(16);

/// One window's reveal state: the handle waiting to be uncloaked, if any.
#[derive(Clone, Default)]
pub struct Reveal {
    pending: Rc<Cell<Option<isize>>>,
    notified: Rc<Cell<bool>>,
}

impl Reveal {
    /// Hooks the window's renderer so a finished frame uncloaks it. Call
    /// once per window.
    pub fn install(window: &slint::Window) -> Self {
        let reveal = Reveal::default();
        let pending = reveal.pending.clone();
        let hooked = window.set_rendering_notifier(move |state, _| {
            if matches!(state, RenderingState::AfterRendering) {
                if let Some(hwnd) = pending.take() {
                    slint::Timer::single_shot(AFTER_FRAME, move || cloak(hwnd, false));
                }
            }
        });
        reveal.notified.set(hooked.is_ok());
        reveal
    }

    /// Shows `window` (through `show`) cloaked, to be uncloaked once a frame
    /// is on screen.
    pub fn show<C: slint::ComponentHandle + 'static>(
        &self,
        ui: &C,
        show: impl FnOnce() -> bool,
    ) -> bool {
        // A window that exists already is cloaked before it is shown; a new
        // one only gets a handle once winit has made it, and is cloaked then.
        if let Some(hwnd) = hwnd(ui.window()) {
            cloak(hwnd, true);
        }
        if !show() {
            return false;
        }
        let this = self.clone();
        crate::theme::when_created(ui, move |ui| this.cloak_until_drawn(ui.window()));
        true
    }

    fn cloak_until_drawn(&self, window: &slint::Window) {
        let Some(hwnd) = hwnd(window) else {
            return;
        };
        cloak(hwnd, true);
        self.pending.set(Some(hwnd));
        window.request_redraw();
        // Without a notifier, or if no frame comes, uncloak anyway: a window
        // that never appears is worse than a flash.
        let pending = self.pending.clone();
        let wait = if self.notified.get() {
            FALLBACK * 8
        } else {
            FALLBACK
        };
        slint::Timer::single_shot(wait, move || {
            if let Some(hwnd) = pending.take() {
                cloak(hwnd, false);
            }
        });
    }
}

fn hwnd(window: &slint::Window) -> Option<isize> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let handle = window.window_handle();
    match handle.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(h.hwnd.get()),
        _ => None,
    }
}

fn cloak(hwnd: isize, on: bool) {
    let value = BOOL::from(on);
    // FFI: our own live window; DWMWA_CLOAK takes a BOOL of the given size.
    let r = unsafe {
        DwmSetWindowAttribute(
            HWND(hwnd as *mut _),
            DWMWA_CLOAK,
            &value as *const BOOL as *const _,
            std::mem::size_of::<BOOL>() as u32,
        )
    };
    if let Err(e) = r {
        tracing::debug!("DWMWA_CLOAK {on} failed: {e}");
    }
}
