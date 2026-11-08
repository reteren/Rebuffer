//! The popup window: show at the cursor, copy on click, hide on Esc or when
//! focus leaves, paste into the window that was in front.
//!
//! The behaviour mirrors `src-tauri/src/window/mod.rs` for the WebView popup:
//! the foreground window is cached as the paste target before the popup takes
//! focus, our own window is never cached, Esc and copy restore the previous
//! window, a click elsewhere leaves focus where the user put it.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use rebuffer_core::api::Core;
use rebuffer_core::model::{Filter, ItemDto, Sort};
use rebuffer_core::{paste, placement};
use slint::winit_030::{winit, EventResult, WinitWindowAccessor};
use slint::{ComponentHandle, Model, ModelNotify, ModelRc, ModelTracker, VecModel};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowThreadProcessId, SetForegroundWindow,
};

use crate::thumbs::{Picture, Thumbs};

slint::include_modules!();

/// How many items the popup loads at once. Phase 3 pages through the store
/// as the grid scrolls; until then the newest this many are shown.
const PAGE: u32 = 2000;
/// Card width plus the gap between cards, in logical pixels. Mirrors
/// `Metrics` in popup.slint.
const CARD_STRIDE: f32 = 176.0 + 8.0;
/// Horizontal padding of the popup's layout, both sides together.
const SIDE_PADDING: f32 = 20.0;

/// The grid as rows of cards over the loaded items.
struct Rows {
    items: RefCell<Vec<ItemDto>>,
    columns: Cell<usize>,
    animate: Cell<bool>,
    thumbs: Rc<Thumbs>,
    notify: ModelNotify,
}

impl Rows {
    fn card(&self, item: &ItemDto, row: usize) -> CardData {
        let animated = if self.animate.get() {
            item.animated_file()
        } else {
            None
        };
        let picture = if item.missing {
            Picture::Pending
        } else {
            self.thumbs
                .picture(item.id, row, item.thumb_file(), animated)
        };
        let (has_image, image) = match picture {
            Picture::Ready(image) => (true, image),
            Picture::Pending => (false, Default::default()),
        };
        CardData {
            id: item.id as i32,
            label: label(item).into(),
            image,
            has_image,
            kind: kind_label(item).into(),
            pinned: item.pinned,
            current: rebuffer_core::current_clipboard_id() == Some(item.id),
        }
    }

    fn rows_holding(&self, id: i64) -> Vec<usize> {
        let columns = self.columns.get().max(1);
        self.items
            .borrow()
            .iter()
            .enumerate()
            .filter(|(_, it)| it.id == id)
            .map(|(pos, _)| pos / columns)
            .collect()
    }
}

impl Model for Rows {
    type Data = RowData;

    fn row_count(&self) -> usize {
        self.items
            .borrow()
            .len()
            .div_ceil(self.columns.get().max(1))
    }

    fn row_data(&self, row: usize) -> Option<RowData> {
        let items = self.items.borrow();
        let columns = self.columns.get().max(1);
        let start = row * columns;
        if start >= items.len() {
            return None;
        }
        self.thumbs.focus(row);
        let end = (start + columns).min(items.len());
        let cards: Vec<CardData> = items[start..end]
            .iter()
            .map(|it| self.card(it, row))
            .collect();
        Some(RowData {
            cards: ModelRc::new(VecModel::from(cards)),
        })
    }

    fn model_tracker(&self) -> &dyn ModelTracker {
        &self.notify
    }
}

struct Popup {
    ui: PopupWindow,
    core: Rc<Core>,
    rows: Rc<Rows>,
    query: RefCell<String>,
    visible: Cell<bool>,
    /// History changed while hidden: reload on the next show.
    stale: Cell<bool>,
    animation: slint::Timer,
}

thread_local! {
    static POPUP: RefCell<Option<Rc<Popup>>> = const { RefCell::new(None) };
}

fn with<R>(f: impl FnOnce(&Rc<Popup>) -> R) -> Option<R> {
    let popup = POPUP.with(|p| p.borrow().clone());
    popup.as_ref().map(f)
}

/// Builds the popup (hidden). Call once, on the main thread, after the Slint
/// backend is selected.
pub fn init(core: Rc<Core>) -> Result<(), slint::PlatformError> {
    let thumbs = Rc::new(Thumbs::start(picture_ready));
    crate::thumbs::install(thumbs.clone());

    let ui = PopupWindow::new()?;
    let rows = Rc::new(Rows {
        items: RefCell::new(Vec::new()),
        columns: Cell::new(4),
        animate: Cell::new(core.settings.get().appearance.animate_gifs),
        thumbs,
        notify: ModelNotify::default(),
    });
    ui.set_rows(ModelRc::from(rows.clone()));

    ui.on_search(|text| {
        with(|p| {
            *p.query.borrow_mut() = text.to_string();
            p.reload();
        });
    });
    ui.on_activate(|id| {
        with(|p| p.activate(i64::from(id)));
    });
    ui.on_dismiss(|| {
        with(|p| p.hide(true));
    });

    // Focus leaving the popup is the user clicking elsewhere: hide, and
    // leave focus where they put it. A resize re-flows the grid.
    ui.window().on_winit_window_event(|_, event| {
        match event {
            winit::event::WindowEvent::Focused(false) => {
                let _ = slint::invoke_from_event_loop(|| {
                    with(|p| p.focus_lost());
                });
            }
            // Esc closes the popup whatever has focus inside it: the search
            // field takes the key for itself, so a FocusScope never sees it.
            winit::event::WindowEvent::KeyboardInput { event, .. }
                if event.state == winit::event::ElementState::Pressed
                    && event.logical_key
                        == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape) =>
            {
                let _ = slint::invoke_from_event_loop(|| {
                    with(|p| p.hide(true));
                });
                return EventResult::PreventDefault;
            }
            winit::event::WindowEvent::Resized(_) => {
                let _ = slint::invoke_from_event_loop(|| {
                    with(|p| p.reflow());
                });
            }
            _ => {}
        }
        EventResult::Propagate
    });

    let popup = Rc::new(Popup {
        ui,
        core,
        rows,
        query: RefCell::new(String::new()),
        visible: Cell::new(false),
        stale: Cell::new(true),
        animation: slint::Timer::default(),
    });
    POPUP.with(|p| *p.borrow_mut() = Some(popup));
    Ok(())
}

/// Shows the popup if hidden, hides it if shown (the hotkey toggles).
pub fn toggle() {
    with(|p| {
        if p.visible.get() {
            p.hide(true);
        } else {
            p.show();
        }
    });
}

pub fn show() {
    with(|p| p.show());
}

/// Something in the history changed (a capture, a delete, a new thumbnail).
pub fn history_changed() {
    with(|p| {
        if p.visible.get() {
            p.reload();
        } else {
            p.stale.set(true);
        }
    });
}

pub fn settings_changed() {
    with(|p| {
        let animate = p.core.settings.get().appearance.animate_gifs;
        if p.rows.animate.replace(animate) != animate {
            p.rows.notify.reset();
        }
    });
}

/// The decoder finished a picture: redraw the rows that show it.
fn picture_ready(id: i64) {
    with(|p| {
        for row in p.rows.rows_holding(id) {
            p.rows.notify.row_changed(row);
        }
        // The first GIF on screen starts the frame timer.
        p.start_animation();
    });
}

impl Popup {
    fn hwnd(&self) -> Option<HWND> {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let handle = self.ui.window().window_handle();
        match handle.window_handle().ok()?.as_raw() {
            RawWindowHandle::Win32(h) => Some(HWND(h.hwnd.get() as *mut _)),
            _ => None,
        }
    }

    fn show(&self) {
        if self.visible.get() {
            return;
        }
        // Cache the paste target BEFORE the popup takes focus, and never our
        // own window: pasting into ourselves is the destructive case.
        // FFI: GetForegroundWindow takes no arguments; null is checked.
        let foreground = unsafe { GetForegroundWindow() };
        if !foreground.is_invalid() && Some(foreground) != self.hwnd() {
            paste::set_target(foreground);
        } else {
            paste::clear_target();
        }

        let settings = self.core.settings.get();
        match placement::popup_rect(&settings.window) {
            Ok((x, y, w, h)) => {
                let window = self.ui.window();
                window.set_position(slint::PhysicalPosition::new(x, y));
                window.set_size(slint::PhysicalSize::new(w, h));
            }
            Err(e) => tracing::warn!("popup placement failed, showing where it was: {e}"),
        }

        if self.stale.replace(false) {
            self.reload();
        }
        if let Err(e) = self.ui.show() {
            tracing::error!("popup did not show: {e}");
            return;
        }
        self.visible.set(true);
        tracing::info!("popup shown");
        if let Some(hwnd) = self.hwnd() {
            // FFI: our own, just-shown window. The hotkey press is the input
            // that entitles this process to take the foreground.
            unsafe {
                let _ = SetForegroundWindow(hwnd);
            }
        }
        self.ui.invoke_focus_search();
        self.reflow();
        self.start_animation();
    }

    /// `restore`: give focus back to the window the popup was opened over.
    /// False when the user already moved focus elsewhere themselves.
    fn hide(&self, restore: bool) {
        if !self.visible.replace(false) {
            return;
        }
        if let Err(e) = self.ui.hide() {
            tracing::warn!("popup did not hide: {e}");
        }
        tracing::info!("popup hidden (restore focus: {restore})");
        self.animation.stop();
        self.rows.thumbs.release_animations();
        if restore {
            paste::restore_foreground_window();
        }
    }

    /// Winit reports focus loss during the show handover as well as when the
    /// user clicks elsewhere. Only the second is a dismissal, and the
    /// difference is visible afterwards: who holds the foreground now.
    fn focus_lost(&self) {
        if !self.visible.get() {
            return;
        }
        // FFI: GetForegroundWindow takes no arguments; null is checked.
        let foreground = unsafe { GetForegroundWindow() };
        if foreground.is_invalid() || Some(foreground) == self.hwnd() {
            tracing::debug!("focus loss ignored: the popup still holds the foreground");
            return;
        }
        let mut pid = 0u32;
        // FFI: a live window handle and a valid out-parameter.
        unsafe {
            GetWindowThreadProcessId(foreground, Some(&mut pid));
        }
        tracing::info!("popup dismissed: focus moved to process {pid}");
        self.hide(false);
    }

    fn reload(&self) {
        let query = self.query.borrow().trim().to_string();
        let filter = Filter::default();
        let result = if query.is_empty() {
            self.core.store.list(&filter, Sort::Newest, 0, PAGE)
        } else {
            self.core.store.search(&query, &filter, PAGE)
        };
        match result {
            Ok(items) => {
                let status = if query.is_empty() {
                    format!("{} items", items.len())
                } else {
                    format!("{} matches", items.len())
                };
                *self.rows.items.borrow_mut() = items;
                self.rows.notify.reset();
                self.ui.set_status(status.into());
            }
            Err(e) => {
                tracing::error!("loading the history failed: {e}");
                self.ui
                    .set_status(format!("Could not load the history: {e}").into());
            }
        }
    }

    /// Recomputes how many cards fit across.
    fn reflow(&self) {
        let window = self.ui.window();
        let width = window.size().to_logical(window.scale_factor()).width;
        let columns = (((width - SIDE_PADDING) / CARD_STRIDE).floor() as usize).max(1);
        if self.rows.columns.replace(columns) != columns {
            self.rows.notify.reset();
        }
    }

    /// Runs the frame timer while a decoded GIF exists and the popup is on
    /// screen, and only then: a timer ticking for nothing is what kept the
    /// first build at 2-4 % CPU with no GIF in sight.
    fn start_animation(&self) {
        if !self.rows.animate.get()
            || !self.visible.get()
            || !self.rows.thumbs.has_animations()
            || self.animation.running()
        {
            return;
        }
        self.animation.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(20),
            || {
                with(|p| {
                    if !p.rows.thumbs.has_animations() {
                        p.animation.stop();
                        return;
                    }
                    for id in p.rows.thumbs.tick() {
                        for row in p.rows.rows_holding(id) {
                            p.rows.notify.row_changed(row);
                        }
                    }
                });
            },
        );
    }

    fn activate(&self, id: i64) {
        let behavior = self.core.settings.get().behavior;
        if let Err(e) = self.core.copy_items(&[id], behavior.paste_as_plain_text) {
            tracing::error!("copy of item {id} failed: {e}");
            self.ui.set_status(format!("Could not copy: {e}").into());
            return;
        }
        if behavior.auto_paste {
            self.hide(true);
            if let Err(e) = paste::send_paste() {
                tracing::warn!("auto-paste skipped: {e}");
            }
        } else if behavior.close_on_copy {
            self.hide(true);
        } else {
            // The card that is now on the clipboard gets its marker.
            self.rows.notify.reset();
        }
    }
}

/// The text a card shows when it has no picture.
fn label(item: &ItemDto) -> String {
    if let Some(t) = item.title.as_deref().filter(|t| !t.is_empty()) {
        return t.to_string();
    }
    if let Some(t) = item.preview_text.as_deref().filter(|t| !t.is_empty()) {
        return t.to_string();
    }
    if !item.file_names.is_empty() {
        return item.file_names.join(", ");
    }
    if let Some(p) = item.ref_path.as_deref() {
        return std::path::Path::new(p)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| p.to_string());
    }
    kind_label(item)
}

fn kind_label(item: &ItemDto) -> String {
    match item.ext.as_deref().filter(|e| !e.is_empty()) {
        Some(ext) => ext.to_uppercase(),
        None => format!("{:?}", item.kind).to_lowercase(),
    }
}
