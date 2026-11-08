//! Rebuffer — persistent clipboard history for Windows.
//!
//! Three long-lived pieces share one process: the message-loop thread that owns
//! the hidden clipboard-listener window, the store, and the pre-created popup
//! WebView2 window. The settings and tray-menu windows are built on demand and
//! destroyed again when idle. See `docs/SPEC.md` §1.
//!
//! OWNER: the coordinator. Workers add to their own modules, not here.

pub mod commands;
pub mod tauri_host;
pub mod tray;
pub mod window;

// The core under its old paths, so `crate::store`, `crate::settings` and the
// rest read the same in the Tauri glue as they did before the split.
pub use rebuffer_core::{
    capture, clipboard, current_clipboard_id, error, host, hotkey, logging, model, preview,
    set_current_clipboard_id, settings, shell, shellthumb, store,
};

use std::sync::Arc;

use tauri::Manager;

/// Managed state, resolved by every command through `State<'_, AppState>`.
/// It is the core itself: the Tauri glue holds nothing of its own.
pub type AppState = rebuffer_core::api::Core;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // A second launch must surface the existing instance, never start a
        // second clipboard listener.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Err(e) = window::show_popup(app) {
                tracing::error!("second instance could not show the popup: {e}");
            }
        }))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        // No baked arguments: the plugin writes them into the Run key once at
        // init, so a baked --silent could never be withdrawn when the user
        // turns silentStart off. The flag carried no information anyway — the
        // app reads settings.json at startup regardless.
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let handle = app.handle().clone();
            let host: Arc<dyn host::Host> = Arc::new(tauri_host::TauriHost::new(handle.clone()));

            let hotkey_handle = handle.clone();
            let core = rebuffer_core::api::Core::start(
                host,
                Box::new(move || {
                    // Never discard this: the hotkey firing but the window not
                    // appearing is the single hardest failure to diagnose from
                    // the outside, because both look like "the hotkey does not
                    // work".
                    tracing::info!("hotkey fired");
                    if let Err(e) = window::show_popup(&hotkey_handle) {
                        tracing::error!("hotkey fired but the popup did not show: {e}");
                    }
                }),
            )?;

            // The popup is the only window created up front, and it is the
            // only one worth the resident memory: it is what the hotkey opens
            // and it has to be instant. Settings and the tray menu are built
            // on demand and get their backdrop then (see window::mod).
            if let Some(w) = handle.get_webview_window(window::POPUP_LABEL) {
                let _ = window::apply_backdrop(&w);
            }

            // A tray app is idle nearly all of the time; give the resident
            // pages back to Windows while nothing of ours is on screen.
            window::memory::start_idle_pass(&handle);

            tray::install(&handle)?;

            app.manage(core);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_items,
            commands::search_items,
            commands::get_item_blob_url,
            commands::get_extension_facets,
            commands::get_tab_counts,
            commands::get_storage_stats,
            commands::copy_to_clipboard,
            commands::paste_to_previous_window,
            commands::set_pinned,
            commands::rename_item,
            commands::delete_items,
            commands::add_files,
            commands::save_item_as,
            commands::open_item,
            commands::open_item_with,
            commands::show_in_folder,
            commands::begin_drag,
            commands::get_settings,
            commands::get_system_languages,
            commands::update_settings,
            commands::relocate_store,
            commands::export_data,
            commands::import_data,
            commands::set_capture_enabled,
            commands::run_cleanup_now,
            commands::clear_history,
            commands::get_current_clipboard_id,
            commands::get_clipboard_history_enabled,
            commands::set_clipboard_history_enabled,
            commands::hide_popup,
            commands::hide_tray_menu,
            commands::quit_app,
            commands::get_capture_enabled,
            commands::show_settings_window,
            commands::popup_ready,
        ])
        .run(tauri::generate_context!())
        .expect("error while running rebuffer");
}
