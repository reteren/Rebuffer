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

mod host;
mod i18n;
mod instance;
mod notify;
mod popup;
mod settings;
mod theme;
mod thumbs;
mod tray;
mod ui;

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

    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("software".into())
        // A tray app: its windows never get a taskbar button, as in the
        // WebView build. The settings window will opt back in when it exists.
        .with_winit_window_attributes_hook(move |attributes| {
            use slint::winit_030::winit::platform::windows::WindowAttributesExtWindows;
            attributes.with_skip_taskbar(true).with_active(!inactive)
        })
        .select()
        .expect("the winit backend with the software renderer is compiled in");

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
