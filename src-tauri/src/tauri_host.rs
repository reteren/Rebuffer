//! The WebView front end as a `Host`: events go out through Tauri's emitter,
//! file grants through the asset-protocol scope.
//!
//! OWNER: the coordinator. This is Tauri glue and is deleted together with the
//! WebView front end; nothing in the core may name it.

use std::path::Path;

use tauri::{AppHandle, Emitter, Manager};

use crate::host::Host;

pub struct TauriHost {
    app: AppHandle,
}

impl TauriHost {
    pub fn new(app: AppHandle) -> TauriHost {
        TauriHost { app }
    }
}

impl Host for TauriHost {
    fn emit_json(&self, event: &str, payload: serde_json::Value) {
        if let Err(e) = self.app.emit(event, payload) {
            tracing::debug!("event {event} not delivered: {e}");
        }
    }

    fn allow_file(&self, path: &Path) {
        if let Err(e) = self.app.asset_protocol_scope().allow_file(path) {
            tracing::debug!("could not allow {}: {e}", path.display());
        }
    }

    fn allow_dir(&self, path: &Path) {
        if let Err(e) = self.app.asset_protocol_scope().allow_directory(path, true) {
            tracing::warn!("could not grant asset access to {}: {e}", path.display());
        }
    }
}
