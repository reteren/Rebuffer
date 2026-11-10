//! The popup: the Alt+V window, ported from `src/routes/Popup.svelte`.
//!
//! Window behaviour mirrors `src-tauri/src/window/mod.rs`: the foreground
//! window is cached as the paste target before the popup takes focus (never
//! our own), Esc and copy give focus back, a click elsewhere leaves it where
//! the user put it. Grid behaviour mirrors the Svelte popup: 200-item pages
//! loaded as the grid scrolls, day groups under the two date sorts, a plain
//! click copies when nothing is selected, Ctrl/Shift select, double click and
//! Enter copy the selection.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;
use std::time::Duration;

use rebuffer_core::api::Core;
use rebuffer_core::model::{Filter, ItemDto, Kind, Sort, StorageWarning, SubKind};
use rebuffer_core::{paste, placement};
use slint::winit_030::{winit, EventResult, WinitWindowAccessor};
use slint::{ComponentHandle, Model, ModelNotify, ModelRc, ModelTracker, SharedString, VecModel};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowThreadProcessId, SetForegroundWindow,
};

use crate::i18n::{tr, tr_with};
use crate::thumbs::{Picture, Thumbs};

pub use crate::ui::*;

/// Items fetched per page, as in the Svelte store.
const PAGE: u32 = 200;
/// Tile size per zoom step (1-5), logical pixels. From Grid.svelte.
const TILE_W: [f32; 5] = [72.0, 92.0, 116.0, 148.0, 188.0];
const TILE_H: [f32; 5] = [96.0, 122.0, 154.0, 196.0, 250.0];
const GAP: f32 = 10.0;
const PAD: f32 = 16.0;
const HEADER_H: f32 = 32.0;
/// The ListView's scrollbar, kept clear of the last column.
const SCROLLBAR: f32 = 12.0;
/// .grid-viewport's left and right padding.
const VIEWPORT_SIDE: f32 = 12.0;

/// The tabs, in display order; each is also its locale key under `tabs.`.
const TABS: [&str; 7] = [
    "all",
    "images",
    "text",
    "links",
    "files",
    "references",
    "pinned",
];

fn tab_filter(tab: usize) -> Filter {
    let mut f = Filter::default();
    match TABS[tab] {
        "images" => f.kind = Some(Kind::Image),
        "text" => f.kind = Some(Kind::Text),
        "links" => f.sub_kind = Some(SubKind::Link),
        "files" => f.kind = Some(Kind::File),
        "references" => f.references_only = true,
        "pinned" => f.pinned_only = true,
        _ => {}
    }
    f
}

const SORTS: [(Sort, &str); 6] = [
    (Sort::Newest, "toolbar.sortNewest"),
    (Sort::Oldest, "toolbar.sortOldest"),
    (Sort::NameAsc, "toolbar.sortNameAsc"),
    (Sort::NameDesc, "toolbar.sortNameDesc"),
    (Sort::SizeAsc, "toolbar.sortSizeAsc"),
    (Sort::SizeDesc, "toolbar.sortSizeDesc"),
];

/// One row of the grid.
#[derive(Clone)]
enum Row {
    Header(String),
    /// Indices into `View::items`.
    Cards(std::ops::Range<usize>),
}

#[derive(Default)]
struct View {
    tab: usize,
    /// The tab's preset, refined by the toolbar filter.
    filter: Filter,
    sort: usize,
    query: String,
    items: Vec<ItemDto>,
    has_more: bool,
    zoom: u32,
    columns: usize,
    rows: Vec<Row>,
    /// Item id -> row it sits in, for redrawing one card.
    row_of: HashMap<i64, usize>,
    selected: BTreeSet<i64>,
    anchor: Option<i64>,
    focused: Option<i64>,
    menu_item: Option<i64>,
    rename_item: Option<i64>,
}

/// The grid model: `View::rows`, rendered on demand.
struct Rows {
    view: Rc<RefCell<View>>,
    thumbs: Rc<Thumbs>,
    animate: Cell<bool>,
    show_age: Cell<bool>,
    clock: Cell<Option<bool>>,
    notify: ModelNotify,
    /// Set while a next-page load is queued, so scrolling does not queue many.
    more_queued: Cell<bool>,
}

impl Model for Rows {
    type Data = RowData;

    fn row_count(&self) -> usize {
        self.view.borrow().rows.len()
    }

    fn row_data(&self, row: usize) -> Option<RowData> {
        let view = self.view.borrow();
        let spec = view.rows.get(row)?.clone();
        // Near the end of what is loaded: fetch the next page, outside this
        // call (the model must not change while it is being read).
        if view.has_more && row + 4 >= view.rows.len() && !self.more_queued.replace(true) {
            let _ = slint::invoke_from_event_loop(|| {
                with(|p| p.load_more());
            });
        }
        Some(match spec {
            Row::Header(label) => RowData {
                header: label.into(),
                first: row == 0,
                cards: ModelRc::default(),
            },
            Row::Cards(range) => {
                self.thumbs.focus(row);
                let mut cards: Vec<CardData> = view.items[range]
                    .iter()
                    .map(|it| self.card(&view, it, row))
                    .collect();
                // Right to left: the newest card sits at the right edge.
                if crate::i18n::code() == "ar" {
                    cards.reverse();
                }
                RowData {
                    header: SharedString::new(),
                    first: row == 0,
                    cards: ModelRc::new(VecModel::from(cards)),
                }
            }
        })
    }

    fn model_tracker(&self) -> &dyn ModelTracker {
        &self.notify
    }
}

impl Rows {
    fn card(&self, view: &View, item: &ItemDto, row: usize) -> CardData {
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
        let face = face(item);
        let (mono_a, mono_b) = monogram_colors(face.domain.as_deref().unwrap_or("rebuffer"));
        CardData {
            id: item.id as i32,
            kind: face.kind.into(),
            text: face.text.into(),
            image,
            has_image,
            monogram: face
                .domain
                .as_deref()
                .and_then(|d| d.chars().next())
                .map(|c| c.to_uppercase().to_string())
                .unwrap_or_else(|| "•".into())
                .into(),
            mono_a,
            mono_b,
            swatch: face.swatch.unwrap_or_default(),
            format_label: face.label.clone().into(),
            ext_color: ext_color(&face.label),
            age: if self.show_age.get() {
                age_label(item.created_at, self.clock.get()).into()
            } else {
                SharedString::new()
            },
            multi: item.kind == Kind::File && item.file_names.len() > 1,
            pinned: item.pinned,
            missing: item.missing,
            current: rebuffer_core::current_clipboard_id() == Some(item.id),
            selected: view.selected.contains(&item.id),
            focused: view.focused == Some(item.id),
        }
    }

    fn redraw(&self, id: i64) {
        let row = self.view.borrow().row_of.get(&id).copied();
        if let Some(row) = row {
            self.notify.row_changed(row);
        }
    }
}

struct Popup {
    ui: PopupWindow,
    /// Shows the window only once a frame is drawn (no white flash).
    reveal: crate::reveal::Reveal,
    core: Rc<Core>,
    view: Rc<RefCell<View>>,
    rows: Rc<Rows>,
    visible: Cell<bool>,
    /// History changed while hidden: reload on the next show.
    stale: Cell<bool>,
    /// A reload is already scheduled; events arriving meanwhile join it.
    reload_queued: Cell<bool>,
    ctrl_down: Cell<bool>,
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
    let settings = core.settings.get();
    let view = Rc::new(RefCell::new(View {
        zoom: settings.window.zoom_step.clamp(1, 5),
        columns: 4,
        ..View::default()
    }));
    let rows = Rc::new(Rows {
        view: view.clone(),
        thumbs,
        animate: Cell::new(settings.appearance.animate_gifs),
        show_age: Cell::new(settings.appearance.show_age),
        clock: Cell::new(clock_mode(&settings.appearance.time_format)),
        notify: ModelNotify::default(),
        more_queued: Cell::new(false),
    });
    ui.set_rows(ModelRc::from(rows.clone()));

    wire_callbacks(&ui);
    wire_window_events(&ui);

    let reveal = crate::reveal::Reveal::install(ui.window());
    let popup = Rc::new(Popup {
        ui,
        reveal,
        core,
        view,
        rows,
        visible: Cell::new(false),
        stale: Cell::new(true),
        reload_queued: Cell::new(false),
        ctrl_down: Cell::new(false),
        animation: slint::Timer::default(),
    });
    POPUP.with(|p| *p.borrow_mut() = Some(popup.clone()));
    popup.apply_settings();
    Ok(())
}

fn wire_callbacks(ui: &PopupWindow) {
    ui.on_search(|text| {
        with(|p| {
            p.view.borrow_mut().query = text.to_string();
            p.clear_selection();
            p.reload(false);
        });
    });
    ui.on_select_tab(|i| {
        with(|p| {
            {
                let mut v = p.view.borrow_mut();
                v.tab = (i as usize).min(TABS.len() - 1);
                v.filter = tab_filter(v.tab);
            }
            p.clear_selection();
            p.ui.set_scroll_y(0.0);
            p.reload(false);
        });
    });
    ui.on_filter(|key| {
        with(|p| p.apply_filter(key.as_str()));
    });
    ui.on_sort(|i| {
        with(|p| {
            p.view.borrow_mut().sort = (i as usize).min(SORTS.len() - 1);
            p.ui.set_scroll_y(0.0);
            p.reload(false);
        });
    });
    ui.on_add_files(|| {
        with(|p| p.add_files());
    });
    ui.on_card_clicked(|id, ctrl, shift| {
        with(|p| p.card_clicked(i64::from(id), ctrl, shift));
    });
    ui.on_card_activated(|id| {
        with(|p| {
            let ids = p.ids_for(i64::from(id));
            p.copy(&ids, None);
        });
    });
    ui.on_card_context(|id| {
        with(|p| p.open_menu(i64::from(id)));
    });
    ui.on_card_drag(|id| {
        with(|p| {
            let ids = p.ids_for(i64::from(id));
            if let Err(e) = rebuffer_core::shell::begin_drag(&p.core.store, &ids) {
                p.show_error(&e.to_string());
            }
        });
    });
    ui.on_menu_action(|action| {
        with(|p| p.menu_action(action.as_str()));
    });
    ui.on_rename_commit(|text| {
        with(|p| p.rename_commit(text.as_str()));
    });
    ui.on_zoom_to(|step| {
        with(|p| p.set_zoom(step as u32));
    });
    ui.on_open_settings(|| {
        with(|p| {
            p.hide(false);
            crate::settings::open(p.core.clone());
        });
    });
    ui.on_dismiss_banner(|| {
        with(|p| p.ui.set_banner(SharedString::new()));
    });
    ui.on_start_window_drag(|| {
        with(|p| {
            p.ui.window().with_winit_window(|w| {
                let _ = w.drag_window();
            });
        });
    });
}

fn wire_window_events(ui: &PopupWindow) {
    use winit::event::{ElementState, MouseScrollDelta, WindowEvent};
    use winit::keyboard::{Key, NamedKey};

    ui.window().on_winit_window_event(|_, event| match event {
        WindowEvent::Focused(false) => {
            let _ = slint::invoke_from_event_loop(|| {
                with(|p| p.focus_lost());
            });
            EventResult::Propagate
        }
        WindowEvent::Resized(_) => {
            let _ = slint::invoke_from_event_loop(|| {
                with(|p| p.reflow());
            });
            EventResult::Propagate
        }
        WindowEvent::ModifiersChanged(m) => {
            let ctrl = m.state().control_key();
            with(|p| p.ctrl_down.set(ctrl));
            EventResult::Propagate
        }
        // Ctrl+wheel zooms, as in the WebView popup.
        WindowEvent::MouseWheel { delta, .. } => {
            let ctrl = with(|p| p.ctrl_down.get()).unwrap_or(false);
            if !ctrl {
                return EventResult::Propagate;
            }
            let up = match delta {
                MouseScrollDelta::LineDelta(_, y) => *y > 0.0,
                MouseScrollDelta::PixelDelta(p) => p.y > 0.0,
            };
            let _ = slint::invoke_from_event_loop(move || {
                with(|p| {
                    let z = p.view.borrow().zoom;
                    p.set_zoom(if up { z + 1 } else { z.saturating_sub(1) });
                });
            });
            EventResult::PreventDefault
        }
        WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
            let key = match &event.logical_key {
                Key::Named(NamedKey::Escape) => "esc",
                Key::Named(NamedKey::Enter) => "enter",
                Key::Named(NamedKey::ArrowUp) => "up",
                Key::Named(NamedKey::ArrowDown) => "down",
                Key::Named(NamedKey::ArrowLeft) => "left",
                Key::Named(NamedKey::ArrowRight) => "right",
                Key::Character(c) if c.eq_ignore_ascii_case("a") => "a",
                _ => return EventResult::Propagate,
            };
            match with(|p| p.key(key)) {
                Some(true) => EventResult::PreventDefault,
                _ => EventResult::Propagate,
            }
        }
        _ => EventResult::Propagate,
    });
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

/// Renders the popup into a PNG without activating it, then quits. See
/// `crate::Snapshot`.
pub fn snapshot(spec: crate::Snapshot) {
    with(|p| {
        if let Some(theme) = &spec.theme {
            crate::theme::apply(&p.ui, theme, "");
        }
        if let Some(lang) = &spec.lang {
            crate::i18n::apply(&p.ui, lang);
        }
        {
            let mut v = p.view.borrow_mut();
            v.tab = (spec.section.max(0) as usize).min(TABS.len() - 1);
            v.filter = tab_filter(v.tab);
        }
        p.reload(false);
        // The newest item stands in for "what the clipboard holds", as the
        // reference page does, so the in-buffer ring is in the picture too.
        let first = p.view.borrow().items.first().map(|i| i.id);
        if let Some(id) = first {
            rebuffer_core::set_current_clipboard_id(p.core.host(), id);
        }
        let window = p.ui.window();
        window.set_size(slint::LogicalSize::new(1000.0, 680.0));
        if let Err(e) = p.ui.show() {
            tracing::error!("snapshot window did not show: {e}");
        }
        p.visible.set(true);
    });
    // Sized once the window exists: winit sizes a new window itself.
    slint::Timer::single_shot(Duration::from_millis(300), || {
        with(|p| {
            p.ui.window()
                .set_size(slint::LogicalSize::new(1000.0, 680.0));
            p.reflow();
        });
    });
    if let Some(menu) = spec.menu.clone() {
        slint::Timer::single_shot(Duration::from_millis(1500), move || {
            with(|p| match menu.as_str() {
                "sort" => p.ui.invoke_open_for_snapshot("sort".into(), 762.0, 0.0),
                "filter" => p.ui.invoke_open_for_snapshot("filter".into(), 889.0, 0.0),
                _ => {
                    // The second card of the first row, as a right-click there.
                    let id = p.view.borrow().items.get(1).map(|i| i.id);
                    if let Some(id) = id {
                        p.ui.invoke_open_for_snapshot("card".into(), 300.0, 230.0);
                        p.open_menu(id);
                    }
                }
            });
        });
    }
    // Long enough for the visible thumbnails to decode.
    slint::Timer::single_shot(Duration::from_millis(2500), move || {
        with(|p| match p.ui.window().take_snapshot() {
            Ok(buf) => {
                let saved =
                    image::RgbaImage::from_raw(buf.width(), buf.height(), buf.as_bytes().to_vec())
                        .map(|img| img.save(&spec.path));
                match saved {
                    Some(Ok(())) => tracing::info!("snapshot saved to {}", spec.path.display()),
                    Some(Err(e)) => tracing::error!("snapshot not saved: {e}"),
                    None => tracing::error!("snapshot buffer has the wrong size"),
                }
            }
            Err(e) => tracing::error!("snapshot failed: {e}"),
        });
        let _ = slint::quit_event_loop();
    });
}

/// Something in the history changed (a capture, a delete, a new thumbnail).
pub fn history_changed() {
    with(|p| {
        if !p.visible.get() {
            p.stale.set(true);
            return;
        }
        // Events come in bursts (a capture, then its thumbnail, then its link
        // preview); one reload covers the burst.
        if !p.reload_queued.replace(true) {
            slint::Timer::single_shot(Duration::from_millis(40), || {
                with(|p| {
                    p.reload_queued.set(false);
                    p.reload(true);
                });
            });
        }
    });
}

pub fn settings_changed() {
    with(|p| p.apply_settings());
}

pub fn storage_warning(w: StorageWarning) {
    with(|p| {
        p.ui.set_banner(crate::notify::storage_text(&w).into());
        p.ui.set_banner_is_error(false);
        let enabled = p.core.settings.get().storage.notify_when_full;
        crate::notify::storage_warning(&w, enabled);
    });
    history_changed();
}

/// The decoder finished a picture: redraw its card, and start the frame
/// timer if it is the first GIF on screen.
fn picture_ready(id: i64) {
    with(|p| {
        p.rows.redraw(id);
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

    // -- window --------------------------------------------------------------

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

        // Reload before sizing: new rows change the layout's constraints, and
        // a hidden window resized by them came up at its minimum height.
        if self.stale.replace(false) {
            self.reload(true);
        }
        let settings = self.core.settings.get();
        let rect = match placement::popup_rect(&settings.window) {
            Ok(rect) => Some(rect),
            Err(e) => {
                tracing::warn!("popup placement failed, showing where it was: {e}");
                None
            }
        };
        let window = self.ui.window();
        if let Some((x, y, w, h)) = rect {
            window.set_position(slint::PhysicalPosition::new(x, y));
            window.set_size(slint::PhysicalSize::new(w, h));
        }
        let shown = self.reveal.show(&self.ui, || match self.ui.show() {
            Ok(()) => true,
            Err(e) => {
                tracing::error!("popup did not show: {e}");
                false
            }
        });
        if !shown {
            return;
        }
        crate::theme::frame(&self.ui, self.ui.global::<Theme>().get_light());
        // Showing applies the size winit kept for the hidden window; put the
        // placed one back if that differs.
        if let Some((x, y, w, h)) = rect {
            if window.size() != slint::PhysicalSize::new(w, h) {
                tracing::info!("popup came up at {:?}, resizing", window.size());
                window.set_position(slint::PhysicalPosition::new(x, y));
                window.set_size(slint::PhysicalSize::new(w, h));
            }
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
        self.ui.set_renaming(false);
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
            return;
        }
        // Our own context or sort menu is a separate window of this process;
        // focus moving there is not the user leaving.
        let mut pid = 0u32;
        // FFI: a live window handle and a valid out-parameter.
        unsafe {
            GetWindowThreadProcessId(foreground, Some(&mut pid));
        }
        if pid == std::process::id() {
            return;
        }
        tracing::info!("popup dismissed: focus moved to process {pid}");
        self.hide(false);
    }

    // -- settings ------------------------------------------------------------

    fn apply_settings(&self) {
        let s = self.core.settings.get();
        crate::theme::apply(&self.ui, &s.appearance.theme, &s.appearance.accent);
        self.ui
            .global::<Theme>()
            .set_reduce_motion(s.appearance.reduce_motion);
        crate::i18n::apply(&self.ui, &s.appearance.language);
        self.ui.set_drag_bar(s.window.drag_bar);

        let layout = self.ui.global::<Layout>();
        let (show_label, label_size) = match s.appearance.format_label_size.as_str() {
            // --fs-2xs, --fs-sm, --fs-lg, as Card.svelte picks them.
            "off" => (false, 10.0),
            "small" => (true, 10.0),
            "large" => (true, 15.0),
            _ => (true, 12.0),
        };
        layout.set_show_label(show_label);
        layout.set_label_size(label_size);
        layout.set_show_age(s.appearance.show_age);

        self.rows.animate.set(s.appearance.animate_gifs);
        self.rows.show_age.set(s.appearance.show_age);
        self.rows.clock.set(clock_mode(&s.appearance.time_format));

        let options: Vec<SharedString> = SORTS.iter().map(|(_, k)| tr(k).into()).collect();
        self.ui
            .set_sort_widest(longest(options.iter().map(|o| o.to_string())).into());
        self.ui
            .set_sort_options(ModelRc::new(VecModel::from(options)));
        self.apply_zoom(s.window.zoom_step);
        self.refresh_chrome();
        self.rebuild_rows();
    }

    fn apply_zoom(&self, zoom: u32) {
        let z = zoom.clamp(1, 5);
        self.view.borrow_mut().zoom = z;
        let layout = self.ui.global::<Layout>();
        layout.set_tile_w(TILE_W[(z - 1) as usize]);
        layout.set_tile_h(TILE_H[(z - 1) as usize]);
        layout.set_gap(GAP);
        layout.set_pad(PAD);
        layout.set_header_h(HEADER_H);
        layout.set_text_size(7.5 + (z as f32 - 1.0) * 1.6);
        self.ui.set_zoom(z as i32);
    }

    fn set_zoom(&self, zoom: u32) {
        let z = zoom.clamp(1, 5);
        if z == self.view.borrow().zoom {
            return;
        }
        self.apply_zoom(z);
        self.reflow();
        // Remembered like the WebView popup does; settings-changed comes back
        // and finds nothing left to change.
        if let Err(e) = self
            .core
            .update_settings(serde_json::json!({ "window": { "zoomStep": z } }))
        {
            tracing::warn!("zoom not saved: {e}");
        }
    }

    // -- data ----------------------------------------------------------------

    /// Re-queries the current view. `keep`: keep as many items as are loaded
    /// now (a refresh after an event), rather than starting from one page.
    fn reload(&self, keep: bool) {
        let (filter, sort, query, loaded) = {
            let v = self.view.borrow();
            (
                v.filter.clone(),
                SORTS[v.sort].0,
                v.query.trim().to_string(),
                v.items.len() as u32,
            )
        };
        let want = if keep { loaded.max(PAGE) } else { PAGE };
        let result = if query.is_empty() {
            self.core.store.list(&filter, sort, 0, want + 1)
        } else {
            self.core.store.search(&query, &filter, want + 1)
        };
        match result {
            Ok(mut items) => {
                let has_more = items.len() as u32 > want && query.is_empty();
                items.truncate(want as usize);
                let mut v = self.view.borrow_mut();
                let alive: BTreeSet<i64> = items.iter().map(|i| i.id).collect();
                v.selected.retain(|id| alive.contains(id));
                if v.focused.is_some_and(|f| !alive.contains(&f)) {
                    v.focused = None;
                }
                v.items = items;
                v.has_more = has_more;
            }
            Err(e) => {
                tracing::error!("loading the history failed: {e}");
                self.show_error(&e.to_string());
            }
        }
        self.rows.more_queued.set(false);
        self.refresh_chrome();
        self.rebuild_rows();
    }

    fn load_more(&self) {
        let (filter, sort, offset, query_empty, has_more) = {
            let v = self.view.borrow();
            (
                v.filter.clone(),
                SORTS[v.sort].0,
                v.items.len() as u32,
                v.query.trim().is_empty(),
                v.has_more,
            )
        };
        if !has_more || !query_empty {
            self.rows.more_queued.set(false);
            return;
        }
        match self.core.store.list(&filter, sort, offset, PAGE) {
            Ok(more) => {
                let mut v = self.view.borrow_mut();
                v.has_more = more.len() as u32 >= PAGE;
                v.items.extend(more);
            }
            Err(e) => tracing::warn!("next page failed: {e}"),
        }
        self.rows.more_queued.set(false);
        self.rebuild_rows();
    }

    /// Tabs, sort label, status line, empty state.
    fn refresh_chrome(&self) {
        let counts = self.core.store.tab_counts().unwrap_or_default();
        let v = self.view.borrow();
        let mut tabs: Vec<TabData> = TABS
            .iter()
            .enumerate()
            .map(|(i, key)| {
                let n = match *key {
                    "all" => counts.all,
                    "images" => counts.images,
                    "text" => counts.text,
                    "links" => counts.links,
                    "files" => counts.files,
                    "references" => counts.references,
                    _ => counts.pinned,
                };
                TabData {
                    index: i as i32,
                    label: tr(&format!("tabs.{key}")).into(),
                    count: if n > 0 {
                        group_thousands(n).into()
                    } else {
                        SharedString::new()
                    },
                    active: i == v.tab,
                }
            })
            .collect();
        if crate::i18n::code() == "ar" {
            tabs.reverse();
        }
        self.ui.set_tabs(ModelRc::new(VecModel::from(tabs)));
        self.ui.set_sort_label(tr(SORTS[v.sort].1).into());
        self.ui.set_sort_index(v.sort as i32);
        self.refresh_filter_menu(&v.filter);

        let total = self.core.store.stats().map(|s| s.total_bytes).unwrap_or(0);
        self.ui.set_status(
            tr_with(
                "status.items",
                &[
                    ("count", &group_thousands(counts.all)),
                    ("size", &status_bytes(total)),
                ],
            )
            .into(),
        );

        let empty = v.items.is_empty();
        self.ui.set_empty(empty);
        if empty {
            let key = TABS[v.tab];
            self.ui
                .set_empty_title(tr(&format!("empty.{key}Title")).into());
            self.ui
                .set_empty_body(tr(&format!("empty.{key}Body")).into());
        }
    }

    /// The toolbar filter: a kind, links, or one extension, on top of what
    /// the tab already restricts (pinned, added files). Picking the filter
    /// a tab stands for selects that tab, as in the WebView toolbar.
    fn apply_filter(&self, key: &str) {
        {
            let mut v = self.view.borrow_mut();
            let f = &mut v.filter;
            f.kind = None;
            f.sub_kind = None;
            f.ext = None;
            match key {
                "all" => {}
                "k:link" => f.sub_kind = Some(SubKind::Link),
                "k:image" => f.kind = Some(Kind::Image),
                "k:text" => f.kind = Some(Kind::Text),
                "k:file" => f.kind = Some(Kind::File),
                "k:video" => f.kind = Some(Kind::Video),
                "k:other" => f.kind = Some(Kind::Other),
                other => f.ext = other.strip_prefix("e:").map(str::to_string),
            }
            let filter = v.filter.clone();
            if let Some(tab) = (0..TABS.len()).find(|&t| same_filter(&tab_filter(t), &filter)) {
                v.tab = tab;
            }
        }
        self.clear_selection();
        self.ui.set_scroll_y(0.0);
        self.reload(false);
    }

    fn refresh_filter_menu(&self, filter: &Filter) {
        let current = if let Some(ext) = &filter.ext {
            format!("e:{ext}")
        } else if filter.sub_kind == Some(SubKind::Link) {
            "k:link".into()
        } else if let Some(kind) = filter.kind {
            format!("k:{}", kind.as_str())
        } else {
            "all".into()
        };
        let option = |key: &str, label: String| FilterEntry {
            checked: key == current,
            key: key.into(),
            label: label.into(),
            header: false,
        };
        let header = |key: &str| FilterEntry {
            key: SharedString::new(),
            label: tr(key).into(),
            header: true,
            checked: false,
        };
        let mut entries = vec![
            header("toolbar.groupType"),
            option("all", tr("toolbar.filterAll")),
            option("k:image", tr("toolbar.kindImages")),
            option("k:text", tr("toolbar.kindText")),
            option("k:link", tr("toolbar.kindLinks")),
            option("k:file", tr("toolbar.kindFiles")),
            option("k:video", tr("toolbar.kindVideo")),
            option("k:other", tr("toolbar.kindOther")),
        ];
        // Extensions present under the tab's own restriction, without the
        // extension itself (or the list would shrink to the one picked).
        let mut base = filter.clone();
        base.ext = None;
        let facets = self.core.store.ext_facets(&base).unwrap_or_default();
        if !facets.is_empty() {
            entries.push(header("toolbar.groupExtension"));
            for f in facets {
                let key = format!("e:{}", f.ext);
                entries.push(option(&key, format!("{} ({})", f.ext, f.count)));
            }
        }
        let label = entries
            .iter()
            .find(|e| e.checked)
            .map(|e| e.label.to_string())
            .unwrap_or_else(|| tr("toolbar.filterAll"));
        let label = if current == "all" {
            tr("toolbar.filterAll")
        } else {
            label
        };
        self.ui.set_filter_label(label.into());
        self.ui.set_filter_widest(
            longest(
                entries
                    .iter()
                    .filter(|e| !e.header)
                    .map(|e| e.label.to_string()),
            )
            .into(),
        );
        self.ui
            .set_filter_entries(ModelRc::new(VecModel::from(entries)));
    }

    /// Recomputes how many cards fit across, and re-lays the rows if that
    /// changed.
    fn reflow(&self) {
        let window = self.ui.window();
        let width = window.size().to_logical(window.scale_factor()).width;
        let zoom = self.view.borrow().zoom;
        let pitch = TILE_W[(zoom - 1) as usize] + GAP;
        // Grid.svelte: the viewport's 12px sides, the scrollbar, then
        // --grid-pad on both sides of the cards.
        let inner = width - VIEWPORT_SIDE * 2.0 - SCROLLBAR - PAD * 2.0 + GAP;
        let columns = ((inner / pitch).floor() as usize).max(1);
        if self.view.borrow().columns != columns {
            self.view.borrow_mut().columns = columns;
            self.rebuild_rows();
        } else {
            self.rows.notify.reset();
        }
    }

    /// Lays the loaded items out as rows: under day headers for the two date
    /// sorts, as a plain grid otherwise.
    fn rebuild_rows(&self) {
        {
            let mut v = self.view.borrow_mut();
            let columns = v.columns.max(1);
            let grouped = matches!(SORTS[v.sort].0, Sort::Newest | Sort::Oldest);
            let mut rows = Vec::new();
            let mut row_of = HashMap::new();
            let mut start = 0;
            while start < v.items.len() {
                let (label, end) = if grouped {
                    let label = day_label(v.items[start].created_at);
                    let mut end = start + 1;
                    while end < v.items.len() && day_label(v.items[end].created_at) == label {
                        end += 1;
                    }
                    (Some(label), end)
                } else {
                    (None, v.items.len())
                };
                if let Some(label) = label {
                    rows.push(Row::Header(label));
                }
                let mut s = start;
                while s < end {
                    let e = (s + columns).min(end);
                    for item in &v.items[s..e] {
                        row_of.insert(item.id, rows.len());
                    }
                    rows.push(Row::Cards(s..e));
                    s = e;
                }
                start = end;
            }
            v.rows = rows;
            v.row_of = row_of;
        }
        self.rows.notify.reset();
    }

    // -- selection and keys ----------------------------------------------------

    fn clear_selection(&self) {
        let mut v = self.view.borrow_mut();
        v.selected.clear();
        v.anchor = None;
        v.focused = None;
    }

    /// The ids an action on `id` applies to: the selection when `id` is part
    /// of it, `id` alone otherwise.
    fn ids_for(&self, id: i64) -> Vec<i64> {
        let v = self.view.borrow();
        if v.selected.contains(&id) {
            v.selected.iter().copied().collect()
        } else {
            vec![id]
        }
    }

    fn card_clicked(&self, id: i64, ctrl: bool, shift: bool) {
        if !ctrl && !shift {
            // A plain click with nothing selected is the quick-copy gesture;
            // with a selection it single-selects without copying.
            if self.view.borrow().selected.is_empty() {
                self.copy(&[id], None);
                return;
            }
            let mut v = self.view.borrow_mut();
            v.selected = BTreeSet::from([id]);
            v.anchor = Some(id);
            v.focused = Some(id);
        } else if ctrl {
            let mut v = self.view.borrow_mut();
            if !v.selected.remove(&id) {
                v.selected.insert(id);
            }
            v.anchor = Some(id);
            v.focused = Some(id);
        } else {
            let mut v = self.view.borrow_mut();
            let anchor = v.anchor.unwrap_or(id);
            let pos = |x: i64| v.items.iter().position(|i| i.id == x);
            if let (Some(a), Some(b)) = (pos(anchor), pos(id)) {
                let (lo, hi) = (a.min(b), a.max(b));
                let ids: Vec<i64> = v.items[lo..=hi].iter().map(|i| i.id).collect();
                v.selected.extend(ids);
            }
            v.focused = Some(id);
        }
        self.rows.notify.reset();
    }

    /// Returns true when the key was used.
    fn key(&self, key: &str) -> bool {
        if self.ui.get_renaming() {
            if key == "esc" {
                self.ui.set_renaming(false);
                return true;
            }
            return false;
        }
        let search_empty = self.ui.invoke_search_text().is_empty();
        match key {
            "esc" => {
                self.hide(true);
                true
            }
            "enter" => {
                let ids: Vec<i64> = {
                    let v = self.view.borrow();
                    if !v.selected.is_empty() {
                        v.selected.iter().copied().collect()
                    } else if let Some(f) = v.focused {
                        vec![f]
                    } else {
                        v.items.first().map(|i| vec![i.id]).unwrap_or_default()
                    }
                };
                if !ids.is_empty() {
                    self.copy(&ids, None);
                }
                true
            }
            "a" if self.ctrl_down.get() && search_empty => {
                let mut v = self.view.borrow_mut();
                v.selected = v.items.iter().map(|i| i.id).collect();
                drop(v);
                self.rows.notify.reset();
                true
            }
            "up" | "down" => {
                self.move_focus(0, if key == "up" { -1 } else { 1 });
                true
            }
            // Left/right belong to the caret while there is text to move in.
            "left" | "right" if search_empty => {
                self.move_focus(if key == "left" { -1 } else { 1 }, 0);
                true
            }
            _ => false,
        }
    }

    fn move_focus(&self, dx: i32, dy: i32) {
        let target_row = {
            let mut v = self.view.borrow_mut();
            if v.items.is_empty() {
                return;
            }
            let rtl = crate::i18n::code() == "ar";
            let dx = if rtl { -dx } else { dx };
            // Card rows with the item index range each covers.
            let card_rows: Vec<std::ops::Range<usize>> = v
                .rows
                .iter()
                .filter_map(|r| match r {
                    Row::Cards(range) => Some(range.clone()),
                    Row::Header(_) => None,
                })
                .collect();
            let current = v
                .focused
                .and_then(|f| v.items.iter().position(|i| i.id == f));
            let next = match current {
                None => 0,
                Some(idx) => {
                    let ri = card_rows.iter().position(|r| r.contains(&idx)).unwrap_or(0);
                    let col = idx - card_rows[ri].start;
                    if dy != 0 {
                        let nr = (ri as i32 + dy).clamp(0, card_rows.len() as i32 - 1) as usize;
                        let r = &card_rows[nr];
                        (r.start + col).min(r.end - 1)
                    } else {
                        (idx as i32 + dx).clamp(0, v.items.len() as i32 - 1) as usize
                    }
                }
            };
            let id = v.items[next].id;
            v.focused = Some(id);
            v.row_of.get(&id).copied()
        };
        self.rows.notify.reset();
        if let Some(row) = target_row {
            self.scroll_to_row(row);
        }
    }

    /// Scrolls just enough to show `row`.
    fn scroll_to_row(&self, row: usize) {
        let (top, bottom) = {
            let v = self.view.borrow();
            let tile_h = TILE_H[(v.zoom - 1) as usize];
            let height = |r: &Row| match r {
                Row::Header(_) => HEADER_H,
                Row::Cards(_) => tile_h + GAP,
            };
            // The first row also carries the grid's top padding.
            let top: f32 = PAD + v.rows[..row].iter().map(height).sum::<f32>();
            (top, top + height(&v.rows[row]))
        };
        let view_h = self.ui.get_list_height();
        let y = -self.ui.get_scroll_y();
        if top < y {
            self.ui.set_scroll_y(-top);
        } else if bottom > y + view_h {
            self.ui.set_scroll_y(-(bottom - view_h));
        }
    }

    // -- actions ---------------------------------------------------------------

    /// The one copy path. With closeOnCopy the popup goes back to the window
    /// it was opened over, and autoPaste pastes there; without it, only the
    /// clipboard is written. `plain` defaults to pasteAsPlainText.
    fn copy(&self, ids: &[i64], plain: Option<bool>) {
        let behavior = self.core.settings.get().behavior;
        let plain = plain.unwrap_or(behavior.paste_as_plain_text);
        if let Err(e) = self.core.copy_items(ids, plain) {
            self.show_error(&e.to_string());
            return;
        }
        if behavior.close_on_copy {
            self.hide(true);
            if behavior.auto_paste {
                if let Err(e) = paste::send_paste() {
                    tracing::warn!("auto-paste skipped: {e}");
                }
            }
        } else {
            self.rows.notify.reset();
        }
    }

    fn open_menu(&self, id: i64) {
        let Some(item) = self.item(id) else { return };
        let mut entries = Vec::new();
        // A card can only show a link as its site; the menu says where it
        // actually points. The WebView popup hangs this over the card.
        if item.sub_kind == Some(SubKind::Link) {
            if let Some(url) = item.preview_text.as_deref().filter(|u| !u.is_empty()) {
                entries.push(MenuEntry {
                    action: SharedString::new(),
                    header: true,
                    label: url.into(),
                    danger: false,
                    separator_before: false,
                });
            }
        }
        entries.push(entry("copy", "menu.copy", false, !entries.is_empty()));
        if item.sub_kind == Some(SubKind::Rich) {
            entries.push(entry("copyPlain", "menu.copyPlain", false, false));
        }
        if matches!(item.kind, Kind::Image | Kind::Video | Kind::File) {
            entries.push(entry("open", "menu.open", false, true));
            entries.push(entry("openWith", "menu.openWith", false, false));
        }
        entries.push(if item.pinned {
            entry("unpin", "menu.unpin", false, true)
        } else {
            entry("pin", "menu.pin", false, true)
        });
        entries.push(entry("saveAs", "menu.saveAs", false, false));
        entries.push(entry("reveal", "menu.showInFolder", false, false));
        entries.push(entry("rename", "menu.rename", false, false));
        entries.push(entry("delete", "menu.delete", true, true));
        self.view.borrow_mut().menu_item = Some(id);
        self.ui
            .set_menu_entries(ModelRc::new(VecModel::from(entries)));
        self.ui.invoke_open_menu();
    }

    fn menu_action(&self, action: &str) {
        let Some(id) = self.view.borrow_mut().menu_item.take() else {
            return;
        };
        let ids = self.ids_for(id);
        let store = &self.core.store;
        let result = match action {
            "copy" => {
                self.copy(&ids, None);
                Ok(())
            }
            "copyPlain" => self.core.copy_items(&ids, true),
            "pin" => self.core.set_pinned(&ids, true),
            "unpin" => self.core.set_pinned(&ids, false),
            "open" => rebuffer_core::shell::open_item(store, id),
            "openWith" => rebuffer_core::shell::open_item_with(store, id),
            "reveal" => rebuffer_core::shell::show_item_in_folder(store, id),
            "saveAs" => {
                self.save_as(id);
                Ok(())
            }
            "rename" => {
                let title = self.item(id).and_then(|i| i.title).unwrap_or_default();
                self.view.borrow_mut().rename_item = Some(id);
                self.ui.set_rename_text(title.into());
                self.ui.set_renaming(true);
                self.ui.invoke_focus_rename();
                Ok(())
            }
            "delete" => {
                let r = self.core.delete(&ids);
                self.clear_selection();
                r
            }
            other => {
                tracing::warn!("unknown menu action {other}");
                Ok(())
            }
        };
        if let Err(e) = result {
            self.show_error(&e.to_string());
        }
    }

    fn rename_commit(&self, text: &str) {
        self.ui.set_renaming(false);
        let Some(id) = self.view.borrow_mut().rename_item.take() else {
            return;
        };
        let title = text.trim();
        if self.item(id).and_then(|i| i.title).as_deref().unwrap_or("") == title {
            return;
        }
        if let Err(e) = self.core.rename(id, title) {
            self.show_error(&e.to_string());
        }
    }

    fn save_as(&self, id: i64) {
        let Some(item) = self.item(id) else { return };
        let base = item
            .title
            .clone()
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| format!("rebuffer-{id}"));
        let ext = item
            .ext
            .as_deref()
            .map(str::to_lowercase)
            .or_else(|| (item.kind == Kind::Text).then(|| "txt".to_string()));
        let name = match ext {
            Some(e) if !e.is_empty() => format!("{base}.{e}"),
            _ => base,
        };
        // A modal dialog: the popup must not vanish under it.
        let target = self.modal(|| {
            rfd::FileDialog::new()
                .set_title(tr("popup.saveItemAs"))
                .set_file_name(name)
                .save_file()
        });
        if let Some(target) = target {
            if let Err(e) = self.core.save_item_as(id, &target) {
                self.show_error(&e.to_string());
            }
        }
    }

    fn add_files(&self) {
        let picked = self.modal(|| {
            rfd::FileDialog::new()
                .set_title(tr("popup.addToRebuffer"))
                .pick_files()
        });
        let Some(paths) = picked else { return };
        let paths: Vec<String> = paths
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        if let Err(e) = self.core.add_files(&paths) {
            self.show_error(&e.to_string());
        }
    }

    /// Runs a modal dialog while the popup stays open: the dialog takes focus,
    /// which would otherwise read as the user clicking away.
    fn modal<T>(&self, f: impl FnOnce() -> T) -> T {
        self.visible.set(false);
        let out = f();
        self.visible.set(true);
        if let Some(hwnd) = self.hwnd() {
            // FFI: our own window, given focus back after its own dialog.
            unsafe {
                let _ = SetForegroundWindow(hwnd);
            }
        }
        out
    }

    fn show_error(&self, text: &str) {
        tracing::warn!("{text}");
        self.ui.set_banner(text.into());
        self.ui.set_banner_is_error(true);
    }

    fn item(&self, id: i64) -> Option<ItemDto> {
        self.view
            .borrow()
            .items
            .iter()
            .find(|i| i.id == id)
            .cloned()
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
                        p.rows.redraw(id);
                    }
                });
            },
        );
    }
}

/// The longest of some labels, by characters: what sizes a <select>-like box.
fn longest(labels: impl Iterator<Item = String>) -> String {
    labels.max_by_key(|l| l.chars().count()).unwrap_or_default()
}

fn same_filter(a: &Filter, b: &Filter) -> bool {
    a.kind == b.kind
        && a.ext == b.ext
        && a.pinned_only == b.pinned_only
        && a.sub_kind == b.sub_kind
        && a.references_only == b.references_only
}

fn entry(action: &str, key: &str, danger: bool, separator_before: bool) -> MenuEntry {
    MenuEntry {
        action: action.into(),
        header: false,
        label: tr(key).into(),
        danger,
        separator_before,
    }
}

// -- card faces ----------------------------------------------------------------

struct Face {
    kind: &'static str,
    text: String,
    label: String,
    domain: Option<String>,
    swatch: Option<slint::Color>,
}

fn plain_face(kind: &'static str, text: String, label: String) -> Face {
    Face {
        kind,
        text,
        label,
        domain: None,
        swatch: None,
    }
}

/// What a card shows, by kind. From Card.svelte.
fn face(item: &ItemDto) -> Face {
    let preview = item.preview_text.clone().unwrap_or_default();
    let fallback = match item.kind {
        Kind::Text => "TXT",
        Kind::Image => "IMG",
        Kind::Video => "VID",
        Kind::File => "FILE",
        Kind::Other => "BIN",
    };
    let ext_label = item
        .ext
        .clone()
        .filter(|e| !e.is_empty())
        .unwrap_or_else(|| fallback.to_string());
    match (item.kind, item.sub_kind) {
        (Kind::Text, Some(SubKind::Link)) => {
            let domain = domain_of(&preview);
            Face {
                kind: "link",
                text: item
                    .title
                    .clone()
                    .or_else(|| domain.clone())
                    .unwrap_or_default(),
                label: domain.clone().unwrap_or_else(|| "LINK".into()),
                domain,
                swatch: None,
            }
        }
        (Kind::Text, Some(SubKind::Color)) => Face {
            kind: "color",
            swatch: crate::theme::parse_color(preview.trim()),
            text: preview,
            label: "COLOR".into(),
            domain: None,
        },
        // A copy out of a terminal can open with a screenful of padding; the
        // card shows the text, not the blank run that wraps it out of view.
        (Kind::Text, Some(SubKind::Code)) => {
            plain_face("code", preview.trim_start().to_string(), "CODE".into())
        }
        (Kind::Text, _) => {
            let text = match item.title.as_deref() {
                Some(t) if !t.is_empty() => t.to_string(),
                _ => preview.trim_start().to_string(),
            };
            plain_face("text", text, ext_label)
        }
        (Kind::File, _) => {
            let text = if item.file_names.len() > 1 {
                tr_with(
                    "card.fileCount",
                    &[("n", &item.file_names.len().to_string())],
                )
            } else {
                item.file_names
                    .first()
                    .cloned()
                    .or_else(|| item.title.clone())
                    .or_else(|| {
                        item.ref_path.as_deref().map(|p| {
                            std::path::Path::new(p)
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_else(|| p.to_string())
                        })
                    })
                    .unwrap_or_else(|| "file".into())
            };
            plain_face("file", text, ext_label)
        }
        (Kind::Image, _) => plain_face("image", String::new(), ext_label),
        (Kind::Video, _) => plain_face("video", String::new(), ext_label),
        (Kind::Other, _) => plain_face("other", String::new(), ext_label),
    }
}

/// `https://www.example.com/x` -> `example.com`.
fn domain_of(url: &str) -> Option<String> {
    let rest = url.trim().split_once("://").map(|(_, r)| r)?;
    let host = rest.split(['/', '?', '#']).next()?;
    let host = host.rsplit('@').next()?.split(':').next()?;
    if host.is_empty() {
        return None;
    }
    Some(host.trim_start_matches("www.").to_string())
}

/// The JS string hash Card.svelte uses (`h * 31 + charCode`, UTF-16 units),
/// so a format label gets the same color in both front ends.
fn js_hash(s: &str) -> u32 {
    s.encode_utf16()
        .fold(0u32, |h, c| h.wrapping_mul(31).wrapping_add(u32::from(c)))
}

fn ext_color(label: &str) -> i32 {
    (js_hash(label) % 7) as i32
}

/// The link monogram's gradient: hsl(h, 62%, 52%) to hsl(h + 40, 60%, 38%).
fn monogram_colors(domain: &str) -> (slint::Color, slint::Color) {
    let h = (js_hash(domain) % 360) as f32;
    (hsl(h, 0.62, 0.52), hsl((h + 40.0) % 360.0, 0.60, 0.38))
}

fn hsl(h: f32, s: f32, l: f32) -> slint::Color {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match h as u32 {
        0..=59 => (c, x, 0.0),
        60..=119 => (x, c, 0.0),
        120..=179 => (0.0, c, x),
        180..=239 => (0.0, x, c),
        240..=299 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let to = |v: f32| ((v + m) * 255.0).round() as u8;
    slint::Color::from_rgb_u8(to(r), to(g), to(b))
}

// -- dates and sizes -------------------------------------------------------------

/// `None`: relative ages. `Some(true)`: 24-hour clock. `Some(false)`: 12-hour.
fn clock_mode(time_format: &str) -> Option<bool> {
    match time_format {
        "clock24" => Some(true),
        "clock12" => Some(false),
        _ => None,
    }
}

fn local(ms: i64) -> chrono::DateTime<chrono::Local> {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_millis_opt(ms)
        .single()
        .unwrap_or_else(chrono::Local::now)
}

fn chrono_locale() -> chrono::Locale {
    use chrono::Locale;
    match crate::i18n::code() {
        "ru" => Locale::ru_RU,
        "de" => Locale::de_DE,
        "es" => Locale::es_ES,
        "pt" => Locale::pt_BR,
        "it" => Locale::it_IT,
        "zh" => Locale::zh_CN,
        "ja" => Locale::ja_JP,
        "fr" => Locale::fr_FR,
        "ar" => Locale::ar_SA,
        _ => Locale::en_US,
    }
}

fn age_label(created_ms: i64, clock: Option<bool>) -> String {
    if let Some(h24) = clock {
        let t = local(created_ms);
        let fmt = if h24 { "%H:%M" } else { "%-I:%M %p" };
        return t.format_localized(fmt, chrono_locale()).to_string();
    }
    let ms = (chrono::Utc::now().timestamp_millis() - created_ms).max(0);
    if ms < 60_000 {
        return tr("card.ageNow");
    }
    let m = ms / 60_000;
    if m < 60 {
        return tr_with("card.ageMinutes", &[("n", &m.to_string())]);
    }
    let h = m / 60;
    if h < 24 {
        return tr_with("card.ageHours", &[("n", &h.to_string())]);
    }
    tr_with("card.ageDays", &[("n", &(h / 24).to_string())])
}

fn day_label(created_ms: i64) -> String {
    use chrono::Datelike;
    let day = local(created_ms).date_naive();
    let today = chrono::Local::now().date_naive();
    if day == today {
        return tr("grid.today");
    }
    if Some(day) == today.pred_opt() {
        return tr("grid.yesterday");
    }
    let same_year = day.year() == today.year();
    let fmt = match (crate::i18n::code(), same_year) {
        ("ja" | "zh", true) => "%-m月%-d日",
        ("ja" | "zh", false) => "%Y年%-m月%-d日",
        ("en", true) => "%B %-d",
        ("en", false) => "%B %-d, %Y",
        (_, true) => "%-d %B",
        (_, false) => "%-d %B %Y",
    };
    day.format_localized(fmt, chrono_locale()).to_string()
}

/// `1,234,567` — counts as the WebView showed them (`toLocaleString('en-US')`).
fn group_thousands(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 {
        out.insert(0, '-');
    }
    out
}

/// The status bar's own size format (StatusBar.svelte): one decimal under
/// 100 units, none above.
fn status_bytes(n: i64) -> String {
    if n < 1024 {
        return format!("{} B", n.max(0));
    }
    let units = ["KB", "MB", "GB", "TB"];
    let mut v = n as f64 / 1024.0;
    let mut u = 0;
    while v >= 1024.0 && u < units.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if v >= 100.0 {
        format!("{} {}", v.round(), units[u])
    } else {
        format!("{v:.1} {}", units[u])
    }
}

pub(crate) fn fmt_bytes(n: i64) -> String {
    if n < 1024 {
        return format!("{n} B");
    }
    let units = ["KB", "MB", "GB", "TB"];
    let mut v = n as f64 / 1024.0;
    let mut u = 0;
    while v >= 1024.0 && u < units.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    let digits = if v >= 100.0 {
        0
    } else if v >= 10.0 {
        1
    } else {
        2
    };
    format!("{v:.digits$} {}", units[u])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_match_the_javascript_ones() {
        // Card.svelte: for (c of s) h = (h * 31 + c) >>> 0
        assert_eq!(js_hash("PNG"), 79_369);
        assert_eq!(ext_color("PNG"), 79_369 % 7);
    }

    #[test]
    fn domains_lose_scheme_www_and_path() {
        assert_eq!(
            domain_of("https://www.github.com/storytold/photocraft").as_deref(),
            Some("github.com")
        );
        assert_eq!(
            domain_of("http://user@host:8080/x").as_deref(),
            Some("host")
        );
        assert_eq!(domain_of("not a url"), None);
    }

    #[test]
    fn counts_and_status_sizes_read_like_the_webview_ones() {
        assert_eq!(group_thousands(2074), "2,074");
        assert_eq!(group_thousands(999), "999");
        assert_eq!(group_thousands(1_234_567), "1,234,567");
        assert_eq!(status_bytes(1_127_000_000), "1.0 GB");
        assert_eq!(status_bytes(150 * 1024 * 1024), "150 MB");
    }

    #[test]
    fn byte_sizes_read_like_the_webview_ones() {
        assert_eq!(fmt_bytes(512), "512 B");
        assert_eq!(fmt_bytes(1536), "1.50 KB");
        assert_eq!(fmt_bytes(1_073_741_824), "1.00 GB");
    }
}
