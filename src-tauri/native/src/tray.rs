//! Tray icon: left click opens the popup; the menu pauses capture and quits.
//!
//! `tray-icon` delivers clicks and menu picks on its own callbacks, which may
//! run outside the Slint event loop; both only post into it.

use std::cell::RefCell;
use std::rc::Rc;

use rebuffer_core::api::Core;
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

struct Tray {
    icon: TrayIcon,
    normal: Icon,
    muted: Icon,
    open: MenuId,
    settings: MenuItem,
    capture: MenuItem,
    quit: MenuId,
    core: Rc<Core>,
}

thread_local! {
    static TRAY: RefCell<Option<Tray>> = const { RefCell::new(None) };
}

/// Keeps the tray icon alive; dropping it removes the icon.
pub struct TrayGuard;

impl Drop for TrayGuard {
    fn drop(&mut self) {
        TRAY.with(|t| t.borrow_mut().take());
    }
}

/// The bundled 32x32 icon and a desaturated twin for "capture paused" —
/// derived in memory, so there is no second asset to keep in sync.
fn icons() -> Result<(Icon, Icon), String> {
    let bytes: &[u8] = include_bytes!("../../icons/32x32.png");
    let rgba = image::load_from_memory(bytes)
        .map_err(|e| e.to_string())?
        .to_rgba8();
    let (w, h) = rgba.dimensions();
    let mut muted = rgba.clone();
    for p in muted.pixels_mut() {
        let luma = (0.2126 * f32::from(p[0]) + 0.7152 * f32::from(p[1]) + 0.0722 * f32::from(p[2]))
            .round() as u8;
        p[0] = luma;
        p[1] = luma;
        p[2] = luma;
    }
    let normal = Icon::from_rgba(rgba.into_raw(), w, h).map_err(|e| e.to_string())?;
    let muted = Icon::from_rgba(muted.into_raw(), w, h).map_err(|e| e.to_string())?;
    Ok((normal, muted))
}

fn capture_label(enabled: bool) -> &'static str {
    if enabled {
        "Pause capture"
    } else {
        "Resume capture"
    }
}

pub fn install(core: Rc<Core>) -> Result<TrayGuard, String> {
    let (normal, muted) = icons()?;
    let enabled = core.clipboard.is_enabled();

    let open = MenuItem::new("Open Rebuffer", true, None);
    let settings = MenuItem::new(crate::i18n::tr("tray.settings"), true, None);
    let capture = MenuItem::new(capture_label(enabled), true, None);
    let quit = MenuItem::new("Quit", true, None);
    let menu = Menu::new();
    menu.append_items(&[
        &open,
        &settings,
        &capture,
        &PredefinedMenuItem::separator(),
        &quit,
    ])
    .map_err(|e| e.to_string())?;

    let icon = TrayIconBuilder::new()
        .with_tooltip("Rebuffer")
        .with_icon(if enabled {
            normal.clone()
        } else {
            muted.clone()
        })
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .build()
        .map_err(|e| e.to_string())?;

    TrayIconEvent::set_event_handler(Some(|event: TrayIconEvent| {
        if let TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } = event
        {
            let _ = slint::invoke_from_event_loop(crate::popup::show);
        }
    }));
    MenuEvent::set_event_handler(Some(|event: MenuEvent| {
        let id = event.id().clone();
        let _ = slint::invoke_from_event_loop(move || on_menu(&id));
    }));

    TRAY.with(|t| {
        *t.borrow_mut() = Some(Tray {
            icon,
            normal,
            muted,
            open: open.id().clone(),
            settings,
            capture,
            quit: quit.id().clone(),
            core,
        });
    });
    Ok(TrayGuard)
}

/// Makes the icon and the menu label match whether capture is on — after the
/// tray toggle, the settings window, or a hand edit of settings.json.
pub fn sync() {
    TRAY.with(|t| {
        let t = t.borrow();
        let Some(tray) = t.as_ref() else { return };
        let enabled = tray.core.clipboard.is_enabled();
        tray.capture.set_text(capture_label(enabled));
        let icon = if enabled { &tray.normal } else { &tray.muted };
        if let Err(e) = tray.icon.set_icon(Some(icon.clone())) {
            tracing::warn!("tray icon not updated: {e}");
        }
    });
}

fn on_menu(id: &MenuId) {
    TRAY.with(|t| {
        let t = t.borrow();
        let Some(tray) = t.as_ref() else { return };
        if *id == tray.open {
            crate::popup::show();
        } else if id == tray.settings.id() {
            crate::settings::open(tray.core.clone());
        } else if id == tray.capture.id() {
            // Saved and applied by the core; the icon follows on the
            // settings-changed that comes back (see `sync`).
            let enabled = !tray.core.clipboard.is_enabled();
            if let Err(e) = tray.core.set_capture_enabled(enabled) {
                tracing::warn!("capture toggle failed: {e}");
            }
        } else if *id == tray.quit {
            let _ = slint::quit_event_loop();
        }
    });
}
