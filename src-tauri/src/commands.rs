//! The complete IPC surface. Mirrored by `src/lib/ipc.ts`.
//!
//! OWNER: the coordinator. Commands stay thin — they resolve state and call
//! `rebuffer_core::api::Core`, which owns the logic and the events. Only what is
//! about WebView windows (hide, show, the tray menu) is done here. Put logic in
//! the core, not here: the native front end calls the same `Core`.

use tauri::{AppHandle, Manager, State};

use crate::error::AppResult;
use crate::model::{
    CleanupResult, Facet, Filter, ImportMode, ItemDto, Sort, StorageStats, TabCounts,
};
use crate::settings::Settings;
use crate::AppState;

// ---------------------------------------------------------------------------
// query
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn list_items(
    state: State<'_, AppState>,
    filter: Filter,
    sort: Sort,
    offset: u32,
    limit: u32,
) -> AppResult<Vec<ItemDto>> {
    state.store.list(&filter, sort, offset, limit)
}

#[tauri::command]
pub fn search_items(
    state: State<'_, AppState>,
    query: String,
    filter: Filter,
    limit: u32,
) -> AppResult<Vec<ItemDto>> {
    state.store.search(&query, &filter, limit)
}

/// An `asset:` protocol URL the WebView can load directly, avoiding a base64
/// round trip for every thumbnail.
#[tauri::command]
pub fn get_item_blob_url(state: State<'_, AppState>, id: i64) -> AppResult<String> {
    let path = state.store.blob_path(id)?;
    // Windows serves Tauri's custom protocols over http://<scheme>.localhost,
    // not scheme://localhost — the CSP in tauri.conf.json lists both forms for
    // exactly this reason, and the asset:// spelling silently fails to load.
    Ok(format!(
        "http://asset.localhost/{}",
        urlencoding::encode(&path.to_string_lossy())
    ))
}

#[tauri::command]
pub fn get_extension_facets(state: State<'_, AppState>, filter: Filter) -> AppResult<Vec<Facet>> {
    state.store.ext_facets(&filter)
}

#[tauri::command]
pub fn get_tab_counts(state: State<'_, AppState>) -> AppResult<TabCounts> {
    state.store.tab_counts()
}

#[tauri::command]
pub fn get_storage_stats(state: State<'_, AppState>) -> AppResult<StorageStats> {
    state.store.stats()
}

// ---------------------------------------------------------------------------
// mutate
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn copy_to_clipboard(
    state: State<'_, AppState>,
    ids: Vec<i64>,
    plain_text: bool,
) -> AppResult<()> {
    state.copy_items(&ids, plain_text)
}

/// The item currently on the clipboard, if we know. `None` after a restart, or
/// when something we never captured was copied.
#[tauri::command]
pub fn get_current_clipboard_id() -> Option<i64> {
    crate::current_clipboard_id()
}

#[tauri::command]
pub fn paste_to_previous_window(
    app: AppHandle,
    state: State<'_, AppState>,
    ids: Vec<i64>,
    plain_text: bool,
) -> AppResult<()> {
    state.copy_items(&ids, plain_text)?;
    crate::window::hide_popup(&app)?;
    if state.settings.get().behavior.auto_paste {
        crate::window::paste::send_paste()?;
    }
    Ok(())
}

#[tauri::command]
pub fn set_pinned(state: State<'_, AppState>, ids: Vec<i64>, pinned: bool) -> AppResult<()> {
    state.set_pinned(&ids, pinned)
}

#[tauri::command]
pub fn rename_item(state: State<'_, AppState>, id: i64, title: String) -> AppResult<()> {
    state.rename(id, &title)
}

#[tauri::command]
pub fn delete_items(state: State<'_, AppState>, ids: Vec<i64>) -> AppResult<()> {
    state.delete(&ids)
}

#[tauri::command]
pub fn add_files(state: State<'_, AppState>, paths: Vec<String>) -> AppResult<Vec<ItemDto>> {
    state.add_files(&paths)
}

#[tauri::command]
pub fn save_item_as(state: State<'_, AppState>, id: i64, target: String) -> AppResult<()> {
    state.save_item_as(id, std::path::Path::new(&target))
}

#[tauri::command]
pub fn open_item(state: State<'_, AppState>, id: i64) -> AppResult<()> {
    crate::shell::open_item(&state.store, id)
}

#[tauri::command]
pub fn open_item_with(state: State<'_, AppState>, id: i64) -> AppResult<()> {
    crate::shell::open_item_with(&state.store, id)
}

#[tauri::command]
pub fn show_in_folder(state: State<'_, AppState>, id: i64) -> AppResult<()> {
    crate::shell::show_item_in_folder(&state.store, id)
}

/// Starts an OLE drag carrying `CF_HDROP`. Non-file items are materialized to
/// temp files with a sensible name first.
#[tauri::command]
pub fn begin_drag(state: State<'_, AppState>, ids: Vec<i64>) -> AppResult<()> {
    crate::shell::begin_drag(&state.store, &ids)
}

// ---------------------------------------------------------------------------
// settings & lifecycle
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Settings {
    state.settings.get()
}

/// The BCP-47 tags of the languages Windows shows its own interface in, most
/// preferred first. The `"system"` language setting resolves against this
/// rather than `navigator.language`, which WebView2 always reports as `en-US`.
#[tauri::command]
pub fn get_system_languages() -> Vec<String> {
    crate::settings::system_ui_languages()
}

/// Reads the live state of Windows clipboard history (Win+V) for this user.
/// OS state, not a stored preference — see `settings::clipboard_history_enabled`.
#[tauri::command]
pub fn get_clipboard_history_enabled() -> AppResult<bool> {
    crate::settings::clipboard_history_enabled()
}

/// Enables or disables Windows clipboard history (Win+V) for this user by
/// writing an explicit DWORD, never by deleting the value.
#[tauri::command]
pub fn set_clipboard_history_enabled(enabled: bool) -> AppResult<()> {
    crate::settings::set_clipboard_history_enabled(enabled)
}

#[tauri::command]
pub fn update_settings(
    state: State<'_, AppState>,
    patch: serde_json::Value,
) -> AppResult<Settings> {
    state.update_settings(patch)
}

#[tauri::command]
pub fn relocate_store(state: State<'_, AppState>, path: String) -> AppResult<()> {
    state.relocate_store(std::path::Path::new(&path))
}

#[tauri::command]
pub fn export_data(state: State<'_, AppState>, path: String) -> AppResult<()> {
    state.export(std::path::Path::new(&path))
}

#[tauri::command]
pub fn import_data(state: State<'_, AppState>, path: String, mode: ImportMode) -> AppResult<()> {
    state.import(std::path::Path::new(&path), mode)
}

#[tauri::command]
pub fn set_capture_enabled(
    app: AppHandle,
    state: State<'_, AppState>,
    enabled: bool,
) -> AppResult<()> {
    state.clipboard.set_enabled(enabled);
    crate::tray::set_capture_enabled(&app, enabled)?;
    Ok(())
}

#[tauri::command]
pub fn run_cleanup_now(
    state: State<'_, AppState>,
    older_than_days: Option<u32>,
) -> AppResult<CleanupResult> {
    state.store.run_cleanup(older_than_days)
}

/// The Data tab's Reset. Distinct from `run_cleanup_now`, which is the janitor
/// and deliberately spares pinned items and shelf references.
#[tauri::command]
pub fn clear_history(state: State<'_, AppState>, include_pinned: bool) -> AppResult<CleanupResult> {
    state.clear_history(include_pinned)
}

/// Dismisses the tray menu. Its own window, so its own hide.
#[tauri::command]
pub fn hide_tray_menu(app: AppHandle) -> AppResult<()> {
    crate::window::hide_tray_menu(&app)
}

/// Quits the application from the tray menu. The store writes every capture
/// synchronously, so there is nothing to flush before going.
#[tauri::command]
pub fn quit_app(app: AppHandle) {
    app.exit(0);
}

/// Whether clipboard capture is currently on, for the tray menu's label.
#[tauri::command]
pub fn get_capture_enabled(state: State<'_, AppState>) -> bool {
    state.clipboard.is_enabled()
}

#[tauri::command]
pub fn hide_popup(app: AppHandle) -> AppResult<()> {
    crate::window::hide_popup(&app)
}

#[tauri::command]
pub fn show_settings_window(app: AppHandle) -> AppResult<()> {
    crate::window::show_settings(&app)
}

/// Lets the popup restore its saved size after the WebView has laid out, which
/// avoids a visible resize flash on show.
#[tauri::command]
pub fn popup_ready(app: AppHandle) {
    if let Some(w) = app.get_webview_window(crate::window::POPUP_LABEL) {
        let _ = w.set_focus();
    }
}
