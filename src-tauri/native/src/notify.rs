//! Windows notifications, for `storage.notifyWhenFull`.
//!
//! A toast from an unpackaged app needs an AppUserModelID that a Start-menu
//! shortcut also carries. The installer's template writes `com.reteren.rebuffer`
//! onto the shortcut it creates, and `init` gives the process the same id, so
//! an installed copy can raise toasts. A development build has no such
//! shortcut; there the toast fails and the failure is only logged.

use std::cell::Cell;
use std::time::{Duration, Instant};

use rebuffer_core::model::StorageWarning;
use windows::core::{w, HSTRING};
use windows::Data::Xml::Dom::XmlDocument;
use windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};

use crate::i18n::{tr, tr_with};
use crate::popup::fmt_bytes;

/// The bundle id the installer puts on the Start-menu shortcut.
const AUMID: &str = "com.reteren.rebuffer";

/// The janitor runs every hour and reports "nearly full" each time the store
/// is over 90 % of its cap; a toast every hour for the same fact is nagging.
const NEARLY_FULL_EVERY: Duration = Duration::from_secs(24 * 60 * 60);

thread_local! {
    static LAST_NEARLY_FULL: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// Gives the process the id its shortcut carries. Call once, early.
pub fn init() {
    // FFI: a static wide string; failure only means no toasts.
    if let Err(e) = unsafe { SetCurrentProcessExplicitAppUserModelID(w!("com.reteren.rebuffer")) } {
        tracing::warn!("AppUserModelID not set, notifications will not show: {e}");
    }
}

/// The sentence for a storage warning, in the interface language. The popup's
/// banner says the same thing.
pub fn storage_text(w: &StorageWarning) -> String {
    if w.removed_items > 0 {
        tr_with(
            "notify.storePruned",
            &[
                ("count", &w.removed_items.to_string()),
                ("size", &fmt_bytes(w.freed_bytes)),
            ],
        )
    } else {
        let pct = if w.cap_bytes > 0 {
            w.used_bytes * 100 / w.cap_bytes
        } else {
            100
        };
        tr_with(
            "notify.storeNearlyFull",
            &[
                ("pct", &pct.to_string()),
                ("used", &fmt_bytes(w.used_bytes)),
                ("cap", &fmt_bytes(w.cap_bytes)),
            ],
        )
    }
}

/// Raises a toast for a storage warning when the user asked for them. A
/// report of items removed always notifies; "nearly full" at most once a day.
pub fn storage_warning(w: &StorageWarning, enabled: bool) {
    if !enabled {
        return;
    }
    if w.removed_items == 0 {
        let now = Instant::now();
        let recent =
            LAST_NEARLY_FULL.with(|l| l.get().is_some_and(|t| now - t < NEARLY_FULL_EVERY));
        if recent {
            return;
        }
        LAST_NEARLY_FULL.with(|l| l.set(Some(now)));
    }
    if let Err(e) = toast(&tr("app.name"), &storage_text(w)) {
        tracing::warn!("storage notification not shown: {e}");
    }
}

fn toast(title: &str, body: &str) -> windows::core::Result<()> {
    let xml = format!(
        "<toast><visual><binding template=\"ToastGeneric\"><text>{}</text><text>{}</text></binding></visual></toast>",
        escape(title),
        escape(body)
    );
    let doc = XmlDocument::new()?;
    doc.LoadXml(&HSTRING::from(xml))?;
    let toast = ToastNotification::CreateToastNotification(&doc)?;
    ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(AUMID))?.Show(&toast)
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// `--notify-test`: shows a sample "nearly full" toast and returns, without
/// touching the store, so notifications can be checked on a machine whose
/// store is nowhere near its cap.
pub fn self_test() {
    init();
    crate::i18n::set_language("system");
    let sample = StorageWarning {
        used_bytes: 940 * 1024 * 1024,
        cap_bytes: 1024 * 1024 * 1024,
        removed_items: 0,
        freed_bytes: 0,
    };
    match toast(&tr("app.name"), &storage_text(&sample)) {
        Ok(()) => tracing::info!("test notification shown"),
        Err(e) => {
            tracing::warn!("test notification failed: {e}");
            eprintln!("notification failed: {e}");
            // A release build has no console; the exit code is the answer.
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toast_text_is_escaped() {
        assert_eq!(escape("a < b & \"c\""), "a &lt; b &amp; &quot;c&quot;");
    }
}
