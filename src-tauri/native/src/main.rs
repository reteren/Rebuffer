//! Rebuffer without a WebView: the core from `rebuffer-core`, the interface in
//! Slint, the tray through `tray-icon`.
//!
//! Everything UI runs on the main thread inside Slint's event loop. Threads
//! the core owns (clipboard listener, hotkey dispatcher, thumbnailer, janitor)
//! reach the interface only through `slint::invoke_from_event_loop`, via
//! `host::NativeHost` for core events and the hotkey callback below.
//!
//! OWNER: the coordinator.

// Release builds must not open a console window behind the tray app.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod edges;
mod host;
mod i18n;
mod instance;
mod notify;
mod popup;
mod reveal;
mod settings;
mod theme;
mod thumbs;
mod tray;
mod ui;
mod wash;

use std::rc::Rc;
use std::sync::Arc;

use rebuffer_core::api::Core;

/// `--snapshot <png> [--theme <name>] [--lang <code>] [--window settings]
/// [--section <n>]`: render the popup to a
/// PNG and exit, without activating any window. For checking the interface
/// without taking focus from whatever the user is doing; the overrides are
/// not saved.
pub struct Snapshot {
    pub path: std::path::PathBuf,
    pub theme: Option<String>,
    pub lang: Option<String>,
    /// `settings` for the settings window; the popup otherwise.
    pub window: Option<String>,
    /// Settings section to open, 0-5; for the popup, the tab.
    pub section: i32,
    /// `sort`, `filter` or `card`: a popup menu to open first.
    pub menu: Option<String>,
}

fn snapshot_args() -> Option<Snapshot> {
    let args: Vec<String> = std::env::args().collect();
    let value = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    Some(Snapshot {
        path: value("--snapshot")?.into(),
        theme: value("--theme"),
        lang: value("--lang"),
        window: value("--window"),
        section: value("--section").and_then(|s| s.parse().ok()).unwrap_or(0),
        menu: value("--menu"),
    })
}

/// Whether this session can make an OpenGL context at all: the femtovg
/// renderer only finds out when its first window is created, too late to
/// fall back, so ask the driver up front with a throwaway window.
fn gl_available() -> bool {
    use windows::core::w;
    use windows::Win32::Graphics::Gdi::{GetDC, ReleaseDC};
    use windows::Win32::Graphics::OpenGL::{
        wglCreateContext, wglDeleteContext, ChoosePixelFormat, SetPixelFormat, PFD_DOUBLEBUFFER,
        PFD_DRAW_TO_WINDOW, PFD_SUPPORT_OPENGL, PFD_TYPE_RGBA, PIXELFORMATDESCRIPTOR,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WS_POPUP,
    };
    // FFI: a hidden STATIC window of our own, its DC, a pixel format and a
    // context; every one is released before returning.
    unsafe {
        let Ok(hwnd) = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            w!(""),
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            None,
            None,
        ) else {
            return false;
        };
        let dc = GetDC(Some(hwnd));
        let pfd = PIXELFORMATDESCRIPTOR {
            nSize: std::mem::size_of::<PIXELFORMATDESCRIPTOR>() as u16,
            nVersion: 1,
            dwFlags: PFD_DRAW_TO_WINDOW | PFD_SUPPORT_OPENGL | PFD_DOUBLEBUFFER,
            iPixelType: PFD_TYPE_RGBA,
            cColorBits: 32,
            cAlphaBits: 8,
            ..Default::default()
        };
        let format = ChoosePixelFormat(dc, &pfd);
        let mut ok = format != 0 && SetPixelFormat(dc, format, &pfd).is_ok();
        if ok {
            match wglCreateContext(dc) {
                Ok(ctx) => {
                    let _ = wglDeleteContext(ctx);
                }
                Err(_) => ok = false,
            }
        }
        ReleaseDC(Some(hwnd), dc);
        let _ = DestroyWindow(hwnd);
        ok
    }
}

fn main() {
    if std::env::args().any(|a| a == "--notify-test") {
        notify::self_test();
        return;
    }
    let snapshot = snapshot_args();
    let inactive = snapshot.is_some();

    // Two processes on one store would both capture every copy and both
    // claim the hotkey. A second launch hands over to the first and leaves.
    // The mutex first: a second native instance hands over and leaves. Only
    // then look for a WebView build. Installed, the native build is itself
    // named rebuffer.exe, so any other rebuffer.exe can only be the WebView
    // one once we know we are the first native instance.
    let Some(_instance) = instance::acquire() else {
        instance::signal_first();
        return;
    };
    if instance::webview_build_running() {
        instance::refuse("The WebView build of Rebuffer is running. Quit it from the tray first.");
        return;
    }

    // Glass first: the GPU renderer draws translucent windows that DWM backs
    // with acrylic, as it did the WebView ones. Where OpenGL is missing (a
    // remote session, a broken driver), or REBUFFER_RENDERER=software asks
    // for it, the software renderer draws them opaque instead.
    let want_glass = std::env::var("REBUFFER_RENDERER").as_deref() != Ok("software");
    let selector = |renderer: &str, transparent: bool| {
        slint::BackendSelector::new()
            .backend_name("winit".into())
            .renderer_name(renderer.into())
            // A tray app: its windows never get a taskbar button, as in the
            // WebView build.
            .with_winit_window_attributes_hook(move |attributes| {
                use slint::winit_030::winit::platform::windows::WindowAttributesExtWindows;
                attributes
                    .with_skip_taskbar(true)
                    .with_active(!inactive)
                    .with_transparent(transparent)
            })
            .select()
    };
    let glass = want_glass && gl_available() && selector("femtovg", true).is_ok();
    if !glass {
        selector("software", false)
            .expect("the winit backend with the software renderer is compiled in");
    }
    theme::set_glass(glass);

    notify::init();
    let host = Arc::new(host::NativeHost);
    let core = match Core::start(
        host,
        Box::new(|| {
            // The hotkey dispatcher thread; the popup lives on the main one.
            tracing::info!("hotkey fired");
            let _ = slint::invoke_from_event_loop(popup::toggle);
        }),
    ) {
        Ok(core) => Rc::new(core),
        Err(e) => {
            instance::refuse(&format!("Rebuffer could not start: {e}"));
            return;
        }
    };

    // Logged here: the log file opens with the core.
    tracing::info!(
        "renderer: {}",
        if glass { "femtovg (glass)" } else { "software" }
    );
    if let Err(e) = popup::init(core.clone()) {
        instance::refuse(&format!("Rebuffer could not create its window: {e}"));
        return;
    }
    install_autostart_hook(&core);

    if let Some(snapshot) = snapshot {
        if snapshot.window.as_deref() == Some("settings") {
            settings::snapshot(core.clone(), snapshot);
        } else {
            popup::snapshot(snapshot);
        }
        let _ = slint::run_event_loop_until_quit();
        return;
    }

    let _tray = match tray::install(core.clone()) {
        Ok(t) => t,
        Err(e) => {
            instance::refuse(&format!("Rebuffer could not add its tray icon: {e}"));
            return;
        }
    };
    let _signal = instance::listen(popup::show);

    tracing::info!("native front end ready");
    if let Err(e) = slint::run_event_loop_until_quit() {
        tracing::error!("event loop ended with an error: {e}");
    }
    tracing::info!("native front end exiting");
}

/// The Run-key value the WebView build registered itself under, so the native
/// build takes over the same entry instead of adding a second one.
const RUN_VALUE: &str = "Rebuffer";

/// Keeps HKCU\...\Run in step with `behavior.launchOnStartup`, and heals it
/// once at startup. Only an installed copy does this: a development build
/// registering its own path would make Windows start a build directory at
/// every logon.
fn install_autostart_hook(core: &Core) {
    // The installer puts uninstall.exe next to the binary; a build directory
    // never has one. (The file name says nothing: the development build is
    // rebuffer.exe too.)
    let installed = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|dir| dir.join("uninstall.exe").is_file()))
        .unwrap_or(false);
    if !installed {
        tracing::info!("not an installed copy; leaving autostart alone");
        return;
    }
    rebuffer_core::settings::set_autostart_hook(|b| {
        let r = if b.launch_on_startup {
            rebuffer_core::settings::rewrite_run_value(RUN_VALUE, false)
        } else {
            rebuffer_core::settings::delete_run_value(RUN_VALUE)
        };
        if let Err(e) = r {
            tracing::warn!("autostart not updated: {e}");
        }
    });
    rebuffer_core::settings::apply_autostart(&core.settings.get().behavior);
}
