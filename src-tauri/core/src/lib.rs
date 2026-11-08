//! Rebuffer without a front end.
//!
//! Everything that captures, stores, configures and hands items to the shell
//! lives here and builds without Tauri or a WebView. A front end attaches by
//! implementing [`host::Host`] and calling into these modules; today that is
//! the Tauri app in `src-tauri/src`, the native interface replacing it will be
//! the next one. See `docs/SPEC.md` §1 for the process model.
//!
//! OWNER: the coordinator. Workers add to their own modules, not here.

pub mod api;
pub mod capture;
pub mod clipboard;
pub mod error;
pub mod host;
pub mod hotkey;
pub mod logging;
pub mod model;
pub mod paste;
pub mod placement;
pub mod preview;
pub mod settings;
pub mod shell;
pub mod shellthumb;
pub mod store;

use std::sync::atomic::{AtomicI64, Ordering};

/// The item currently sitting on the clipboard, or 0 for "none we know of".
///
/// Every clipboard change is either a capture we just stored or a write we made
/// ourselves, so the id is known at both moments; there is nowhere else it can
/// come from. It is deliberately not persisted — after a restart the clipboard
/// may hold anything, and claiming otherwise would be a lie on screen.
static CURRENT_CLIPBOARD_ID: AtomicI64 = AtomicI64::new(0);

/// Records which item is on the clipboard now, and tells the front end so it
/// can mark the card.
pub fn set_current_clipboard_id(host: &dyn host::Host, id: i64) {
    if CURRENT_CLIPBOARD_ID.swap(id, Ordering::SeqCst) != id {
        host::emit(host, model::events::CLIPBOARD_CURRENT, &id);
    }
}

pub fn current_clipboard_id() -> Option<i64> {
    match CURRENT_CLIPBOARD_ID.load(Ordering::SeqCst) {
        0 => None,
        id => Some(id),
    }
}
