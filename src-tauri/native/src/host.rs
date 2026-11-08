//! The core's events, delivered into the Slint event loop.

use rebuffer_core::host::Host;
use rebuffer_core::model::events;

/// Every event is posted to the main thread, where the popup decides what it
/// means for what is on screen. Posting is cheap and never blocks the
/// core's own threads.
pub struct NativeHost;

impl Host for NativeHost {
    fn emit_json(&self, event: &str, payload: serde_json::Value) {
        let event = event.to_string();
        let posted = slint::invoke_from_event_loop(move || match event.as_str() {
            events::ITEM_ADDED
            | events::ITEM_UPDATED
            | events::ITEMS_DELETED
            | events::CLIPBOARD_CURRENT => crate::popup::history_changed(),
            events::SETTINGS_CHANGED => {
                crate::popup::settings_changed();
                crate::tray::sync();
            }
            events::STORAGE_WARNING => match serde_json::from_value(payload) {
                Ok(w) => crate::popup::storage_warning(w),
                Err(e) => tracing::warn!("storage warning with an unreadable payload: {e}"),
            },
            other => tracing::debug!("event {other} has no native handler yet: {payload}"),
        });
        if let Err(e) = posted {
            tracing::debug!("event not delivered, no event loop: {e}");
        }
    }
}
