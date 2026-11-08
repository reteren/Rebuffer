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
mod popup;
mod settings;
mod theme;
mod thumbs;
mod tray;
mod ui;

use std::rc::Rc;
use std::sync::Arc;

use rebuffer_core::api::Core;

/// `--snapshot <png> [--theme <name>] [--lang <code>]`: render the popup to a
/// PNG and exit, without activating any window. For checking the interface
/// without taking focus from whatever the user is doing; the overrides are
/// not saved.
pub struct Snapshot {
    pub path: std::path::PathBuf,
    pub theme: Option<String>,
    pub lang: Option<String>,
    /// `settings` for the settings window; the popup otherwise.
    pub window: Option<String>,
    /// Settings section to open, 0-5.
    pub section: i32,
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
    })
}

fn main() {
    let snapshot = snapshot_args();
    let inactive = snapshot.is_some();

    // Two processes on one store would both capture every copy and both
    // claim the hotkey. A second launch hands over to the first and leaves.
    if instance::webview_build_running() {
        instance::refuse("The WebView build of Rebuffer is running. Quit it from the tray first.");
        return;
    }
    let Some(_instance) = instance::acquire() else {
        instance::signal_first();
        return;
    };

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
