//! The running core: everything a front end calls, in one place.
//!
//! `Core::start` brings the process up in the order the pieces depend on each
//! other — logging, settings, store, clipboard listener, hotkey — and the
//! methods below are the operations that touch more than one of them or have
//! to announce what they did. A front end turns a click into one of these
//! calls; it does not reach into the store and emit events itself, so the
//! WebView front end and the native one cannot drift apart in what a
//! "delete" or a "settings change" means.
//!
//! Plain reads (`list`, `search`, `stats`, …) go straight to the public fields:
//! wrapping them here would add a layer and no meaning.
//!
//! OWNER: the coordinator.

use std::path::Path;
use std::sync::Arc;

use crate::clipboard::ClipboardWatcher;
use crate::error::AppResult;
use crate::host::{self, Host};
use crate::hotkey::{Chord, HotkeyManager};
use crate::model::{events, CleanupResult, ImportMode, ItemDto, RetentionPolicy};
use crate::settings::{self, Settings, SettingsStore};
use crate::store::{janitor, Store};

/// The hotkey a binding falls back to when the stored one cannot be parsed.
const DEFAULT_BINDING: &str = "Alt+V";

pub struct Core {
    /// Keeps the non-blocking log writer alive. Dropping it shuts the writer
    /// down, and every later line is discarded — which reads exactly like the
    /// app going silent after startup.
    _log_guard: Option<tracing_appender::non_blocking::WorkerGuard>,
    pub store: Arc<Store>,
    pub settings: Arc<SettingsStore>,
    pub clipboard: Arc<ClipboardWatcher>,
    pub hotkeys: Arc<HotkeyManager>,
    host: Arc<dyn Host>,
}

impl Core {
    /// Brings the core up and attaches `host` to it. `on_hotkey` runs on the
    /// hotkey dispatcher thread each time the chord fires.
    pub fn start(
        host: Arc<dyn Host>,
        on_hotkey: Box<dyn Fn() + Send + Sync + 'static>,
    ) -> AppResult<Core> {
        // Logging comes first. settings.json always lives under the default
        // root, so its directory is known without reading any settings, and
        // starting the logger afterwards threw away exactly the messages
        // worth having: the warning that settings.json was corrupt and was
        // replaced by defaults went nowhere, so a user whose settings were
        // silently reset had no way to find out.
        let default_root = settings::default_store_root();
        let log_guard = crate::logging::init(&default_root.join("logs"));

        let settings = Arc::new(SettingsStore::load(&default_root.join("settings.json"))?);
        let resolved = settings.get();

        let store_root = resolved.store_root();
        if store_root != default_root {
            tracing::info!(
                "store relocated to {}; logs stay under the default root",
                store_root.display()
            );
        }
        tracing::info!("rebuffer starting, store at {}", store_root.display());

        // A configured store on a removable or network volume will one day
        // be gone at startup. Failing here would be unrecoverable for the
        // user: the tray icon never appears, so there is no way to open
        // settings and correct the path. Fall back to the default root and
        // say so loudly instead — a running app with an empty history can
        // be fixed, one that refuses to launch cannot.
        let (store, store_root) = match Store::open(&store_root) {
            Ok(s) => (Arc::new(s), store_root),
            Err(e) => {
                tracing::error!(
                    "store at {} could not be opened ({e}); falling back to {}",
                    store_root.display(),
                    default_root.display()
                );
                let s = Arc::new(Store::open(&default_root)?);
                host::emit(
                    &*host,
                    events::STORE_UNAVAILABLE,
                    &format!("{}", store_root.display()),
                );
                (s, default_root)
            }
        };

        // Granted only now: the front end may read the store it is actually
        // using, and doing this before the open would have granted the path
        // that failed rather than the one in use.
        host.allow_dir(&store_root);

        // Without this the store holds no host and every event it emits from
        // a background thread is silently dropped: the thumbnailer's and the
        // link lookup's "this row changed", and the janitor's prune and
        // storage-warning notices. Nothing errors — the UI just never hears,
        // and a card sits on its placeholder until something unrelated
        // redraws it.
        store.set_host(host.clone());

        // The store deliberately does not read settings.json, so without
        // this it runs on its defaults — 30 days and no size cap — and a
        // user who set either would never see it take effect.
        store.set_retention_policy(retention_policy(&resolved));
        store.set_link_previews(resolved.privacy.link_previews);
        crate::shell::set_temp_files_days(resolved.storage.temp_files_days);

        let capture_host = host.clone();
        let clipboard = Arc::new(ClipboardWatcher::start(
            store.clone(),
            Box::new(move |item| {
                // A fresh capture IS what is on the clipboard right now.
                crate::set_current_clipboard_id(&*capture_host, item.id);
                host::emit(&*capture_host, events::ITEM_ADDED, &item);
            }),
        )?);
        // The listener starts enabled; a user who paused capture and
        // restarted would otherwise see the muted tray icon over a listener
        // that is quietly recording again.
        clipboard.set_enabled(resolved.behavior.capture_enabled);
        if !resolved.behavior.capture_enabled {
            tracing::info!("capture is paused by settings");
        }

        let hotkeys = Arc::new(HotkeyManager::new(on_hotkey)?);

        // HotkeyManager::new only builds the machinery; nothing is
        // registered until rebind runs, so without this the app starts with
        // no hotkey at all. A binding the user has made unusable must not
        // stop the app from starting — fall back to the default and log it,
        // because a tray app that refuses to launch is unrecoverable
        // without editing settings.json by hand.
        let chord = Chord::parse(&resolved.hotkey.binding).unwrap_or_else(|e| {
            tracing::warn!(
                "hotkey {:?} is not parseable ({e}), falling back to {DEFAULT_BINDING}",
                resolved.hotkey.binding
            );
            Chord::parse(DEFAULT_BINDING).expect("the default binding must always parse")
        });
        match hotkeys.rebind(&chord, resolved.hotkey.aggressive_mode) {
            Ok(()) => tracing::info!(
                "hotkey {} registered (aggressive: {})",
                chord.to_display(),
                resolved.hotkey.aggressive_mode
            ),
            Err(e) => tracing::error!("could not register hotkey {}: {e}", chord.to_display()),
        }

        // A hand edit of settings.json gets the same treatment as a change
        // made in the settings window: live state follows, windows re-sync.
        // Weak references, so the listener the settings store holds does not
        // keep the settings store itself (or anything else) alive.
        let weak_store = Arc::downgrade(&store);
        let weak_hotkeys = Arc::downgrade(&hotkeys);
        let weak_settings = Arc::downgrade(&settings);
        let weak_clipboard = Arc::downgrade(&clipboard);
        let reload_host = host.clone();
        settings.set_on_reload(Arc::new(move |before: &Settings, next: &Settings| {
            let (Some(store), Some(hotkeys), Some(settings), Some(clipboard)) = (
                weak_store.upgrade(),
                weak_hotkeys.upgrade(),
                weak_settings.upgrade(),
                weak_clipboard.upgrade(),
            ) else {
                return;
            };
            tracing::info!("settings.json changed on disk; applying it");
            let live = Live {
                store: &store,
                hotkeys: &hotkeys,
                settings: &settings,
                clipboard: &clipboard,
                host: &*reload_host,
            };
            if let Err(e) = live.apply(before, next) {
                tracing::warn!("hand-edited settings not fully applied: {e}");
            }
        }));

        Ok(Core {
            _log_guard: log_guard,
            store,
            settings,
            clipboard,
            hotkeys,
            host,
        })
    }

    /// The front end every event goes to.
    pub fn host(&self) -> &dyn Host {
        &*self.host
    }

    // -----------------------------------------------------------------------
    // items
    // -----------------------------------------------------------------------

    /// Puts `ids` on the clipboard.
    pub fn copy_items(&self, ids: &[i64], plain_text: bool) -> AppResult<()> {
        crate::clipboard::writer::write_items(&self.store, ids, plain_text)?;
        // We wrote it, so we know what is on the clipboard — the listener
        // skips our own writes and would never report this one.
        if let [id] = ids[..] {
            crate::set_current_clipboard_id(&*self.host, id);
        }
        Ok(())
    }

    pub fn set_pinned(&self, ids: &[i64], pinned: bool) -> AppResult<()> {
        self.store.set_pinned(ids, pinned)?;
        host::emit(&*self.host, events::ITEM_UPDATED, ids);
        Ok(())
    }

    pub fn rename(&self, id: i64, title: &str) -> AppResult<()> {
        self.store.rename(id, title)?;
        host::emit(&*self.host, events::ITEM_UPDATED, &[id]);
        Ok(())
    }

    pub fn delete(&self, ids: &[i64]) -> AppResult<()> {
        self.store.delete(ids)?;
        host::emit(&*self.host, events::ITEMS_DELETED, ids);
        Ok(())
    }

    /// The shelf's `+ Add`: stores references to files where they lie.
    pub fn add_files(&self, paths: &[String]) -> AppResult<Vec<ItemDto>> {
        let items = self.store.add_references(paths)?;
        for item in &items {
            host::emit(&*self.host, events::ITEM_ADDED, item);
        }
        Ok(items)
    }

    pub fn save_item_as(&self, id: i64, target: &Path) -> AppResult<()> {
        let src = self.store.blob_path(id)?;
        std::fs::copy(src, target)?;
        Ok(())
    }

    /// The Data tab's Reset. Distinct from `Store::run_cleanup`, which is the
    /// janitor and deliberately spares pinned items and shelf references.
    pub fn clear_history(&self, include_pinned: bool) -> AppResult<CleanupResult> {
        let result = self.store.clear_history(include_pinned)?;
        // An empty list means "everything went": the front end reloads.
        host::emit(&*self.host, events::ITEMS_DELETED, &Vec::<i64>::new());
        Ok(result)
    }

    // -----------------------------------------------------------------------
    // store as a whole
    // -----------------------------------------------------------------------

    pub fn relocate_store(&self, target: &Path) -> AppResult<()> {
        // Granted before the move, not after: the front end may request a
        // thumbnail the moment relocation finishes.
        self.host.allow_dir(target);
        janitor::relocate(&*self.host, &self.store, target)
    }

    pub fn export(&self, target: &Path) -> AppResult<()> {
        janitor::export(&*self.host, &self.store, target)
    }

    pub fn import(&self, archive: &Path, mode: ImportMode) -> AppResult<()> {
        janitor::import(&*self.host, &self.store, archive, mode)
    }

    // -----------------------------------------------------------------------
    // settings
    // -----------------------------------------------------------------------

    /// Merges `patch` into the settings, saves them, and pushes whatever
    /// changed at the OS state that holds it.
    pub fn update_settings(&self, patch: serde_json::Value) -> AppResult<Settings> {
        let before = self.settings.get();

        // Validate the chord BEFORE anything is persisted. Saving first and
        // validating after leaves settings.json and the settings UI showing
        // a binding the runtime is not actually using, which is worse than a
        // rejected change: the user sees their new hotkey and it does nothing.
        if let Some(binding) = patch
            .get("hotkey")
            .and_then(|h| h.get("binding"))
            .and_then(|b| b.as_str())
        {
            if binding != before.hotkey.binding {
                Chord::parse(binding)?;
            }
        }

        let next = self.settings.patch(patch)?;
        self.live().apply(&before, &next)
    }

    /// Pauses or resumes capture, and remembers it: the tray toggle and the
    /// settings switch are the same setting.
    pub fn set_capture_enabled(&self, enabled: bool) -> AppResult<Settings> {
        self.update_settings(serde_json::json!({ "behavior": { "captureEnabled": enabled } }))
    }

    fn live(&self) -> Live<'_> {
        Live {
            store: &self.store,
            hotkeys: &self.hotkeys,
            settings: &self.settings,
            clipboard: &self.clipboard,
            host: &*self.host,
        }
    }
}

/// What a settings change has to reach.
struct Live<'a> {
    store: &'a Store,
    hotkeys: &'a HotkeyManager,
    settings: &'a SettingsStore,
    clipboard: &'a ClipboardWatcher,
    host: &'a dyn Host,
}

fn retention_policy(s: &Settings) -> RetentionPolicy {
    RetentionPolicy {
        retention_days: s.storage.retention_days,
        max_store_bytes: s.storage.max_store_bytes.map(|b| b as i64),
    }
}

/// Settings that own live OS state have to be pushed at whatever holds it;
/// saving the file changes nothing on its own. Shared by the settings window
/// and by hand edits of the file, so the two cannot disagree on what a change
/// does. Announces the outcome as `settings-changed` and returns it.
impl Live<'_> {
    fn apply(&self, before: &Settings, next: &Settings) -> AppResult<Settings> {
        let (hotkeys, settings, host) = (self.hotkeys, self.settings, self.host);
        if next.hotkey.binding != before.hotkey.binding
            || next.hotkey.aggressive_mode != before.hotkey.aggressive_mode
        {
            let applied = Chord::parse(&next.hotkey.binding)
                .and_then(|chord| hotkeys.rebind(&chord, next.hotkey.aggressive_mode));
            if let Err(e) = applied {
                // The registration failed and the manager kept the old chord, so
                // roll the file back to match the runtime rather than letting the
                // two disagree. The rest of the change stays applied: one refused
                // hotkey should not discard the user's other changes.
                let rollback = serde_json::json!({
                    "hotkey": {
                        "binding": before.hotkey.binding,
                        "aggressiveMode": before.hotkey.aggressive_mode,
                    }
                });
                let reverted = settings.patch(rollback)?;
                self.apply_rest(before, &reverted);
                host::emit(host, events::SETTINGS_CHANGED, &reverted);
                return Err(e);
            }
        }

        self.apply_rest(before, next);
        host::emit(host, events::SETTINGS_CHANGED, next);
        Ok(next.clone())
    }

    /// Everything in `apply` past the hotkey, which cannot fail.
    fn apply_rest(&self, before: &Settings, next: &Settings) {
        let store = self.store;
        if next.behavior.capture_enabled != before.behavior.capture_enabled {
            self.clipboard.set_enabled(next.behavior.capture_enabled);
            tracing::info!(
                "capture {}",
                if next.behavior.capture_enabled {
                    "resumed"
                } else {
                    "paused"
                }
            );
        }
        if next.storage.temp_files_days != before.storage.temp_files_days {
            crate::shell::set_temp_files_days(next.storage.temp_files_days);
        }

        if next.storage.retention_days != before.storage.retention_days
            || next.storage.max_store_bytes != before.storage.max_store_bytes
        {
            store.set_retention_policy(retention_policy(next));
        }

        if next.privacy.link_previews != before.privacy.link_previews {
            store.set_link_previews(next.privacy.link_previews);
            // Switching it on is the user asking for previews, not just for
            // previews from here on: the links already in history are looked up
            // too. Failures are the lookup's business, so this never blocks the
            // save.
            if next.privacy.link_previews {
                match store.backfill_link_previews() {
                    Ok(n) if n > 0 => tracing::info!("looking up {n} link(s) already in history"),
                    Ok(_) => {}
                    Err(e) => tracing::warn!("could not queue existing links: {e}"),
                }
            }
        }
    }
}
