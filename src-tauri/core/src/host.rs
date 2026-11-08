//! What the core needs from whatever draws the interface.
//!
//! The store, the janitor and the settings watcher report what happened —
//! a thumbnail landed, rows were pruned, the file was edited by hand — but they
//! must not know who is listening. Today the listener is a set of WebViews
//! behind Tauri; the native interface replacing them will be a channel into its
//! own event loop. Both implement this trait, and nothing under `store/`,
//! `clipboard/` or `settings.rs` names either of them.
//!
//! OWNER: the coordinator. Keep the trait small: every method added here is a
//! method every front end has to answer.

use std::path::Path;

use serde::Serialize;

/// The front end, as seen from the core.
pub trait Host: Send + Sync + 'static {
    /// Delivers one event. `event` is one of the names in `model::events`;
    /// `payload` is exactly what the WebView front end receives as JSON.
    fn emit_json(&self, event: &str, payload: serde_json::Value);

    /// Lets the front end read one file outside the store, which the WebView's
    /// asset protocol needs per file. A front end that reads the disk itself
    /// has nothing to grant, hence the default.
    fn allow_file(&self, _path: &Path) {}

    /// Same, for a whole directory: the store root, before anything inside it
    /// is shown.
    fn allow_dir(&self, _path: &Path) {}
}

/// Serializes `payload` and hands it to `host`. A payload that will not
/// serialize is a programming error, but not one worth taking the process
/// down for: the event is dropped and the fact logged.
pub fn emit<T: Serialize + ?Sized>(host: &dyn Host, event: &str, payload: &T) {
    match serde_json::to_value(payload) {
        Ok(value) => host.emit_json(event, value),
        Err(e) => tracing::warn!("event {event} dropped, payload did not serialize: {e}"),
    }
}

/// A host that hears nothing. For tests and for work done before any front
/// end exists.
pub struct NullHost;

impl Host for NullHost {
    fn emit_json(&self, _event: &str, _payload: serde_json::Value) {}
}
