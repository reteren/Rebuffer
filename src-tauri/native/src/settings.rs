//! The settings window, ported from `src/routes/Settings.svelte`.
//!
//! Built when opened and dropped when closed: it is a window a person opens a
//! few times a year, and a tray app should not carry it the rest of the time.
//! Every control becomes a settings patch through `Core::update_settings`, so
//! the window, the popup, the tray and a hand edit of settings.json all go
//! through the one path that also applies the change to the running app.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use rebuffer_core::api::Core;
use rebuffer_core::model::{ImportMode, StoreProgress};
use rebuffer_core::settings::Settings;
use serde_json::{json, Value};
use slint::winit_030::WinitWindowAccessor;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::i18n::{tr, tr_with};
use crate::popup::fmt_bytes;
use crate::ui::{AccentPreset, LangOption, SettingsData, SettingsWindow, UsageSeg};

const MB: u64 = 1024 * 1024;

/// Accent presets offered next to the theme's own: the theme palette's
/// extension colors plus one warm one. The WebView window has a free color
/// picker; Slint has none, so the native one offers these.
const ACCENTS: [&str; 8] = [
    "#7aa2ff", "#4ec9b0", "#bb9af7", "#e0af68", "#f7768e", "#2ac3de", "#9ece6a", "#ff9e64",
];

/// Win+key chords Windows keeps for itself, with the locale key that names
/// what each one does. From Settings.svelte.
const WIN_RESERVED: [(char, &str); 20] = [
    ('V', "hotkey.winV"),
    ('E', "hotkey.winE"),
    ('R', "hotkey.winR"),
    ('L', "hotkey.winL"),
    ('D', "hotkey.winD"),
    ('I', "hotkey.winI"),
    ('S', "hotkey.winS"),
    ('K', "hotkey.winK"),
    ('G', "hotkey.winG"),
    ('T', "hotkey.winT"),
    ('X', "hotkey.winX"),
    ('U', "hotkey.winU"),
    ('W', "hotkey.winW"),
    ('A', "hotkey.winA"),
    ('H', "hotkey.winH"),
    ('F', "hotkey.winF"),
    ('M', "hotkey.winM"),
    ('Z', "hotkey.winZ"),
    ('P', "hotkey.winP"),
    ('Y', "hotkey.winY"),
];

struct Window {
    ui: SettingsWindow,
    core: Rc<Core>,
}

thread_local! {
    static WINDOW: RefCell<Option<Rc<Window>>> = const { RefCell::new(None) };
}

fn with<R>(f: impl FnOnce(&Rc<Window>) -> R) -> Option<R> {
    let w = WINDOW.with(|w| w.borrow().clone());
    w.as_ref().map(f)
}

/// Opens the settings window, building it if it is not open yet.
pub fn open(core: Rc<Core>) {
    if with(|w| w.focus()).is_some() {
        return;
    }
    let ui = match SettingsWindow::new() {
        Ok(ui) => ui,
        Err(e) => {
            tracing::error!("settings window could not be built: {e}");
            return;
        }
    };
    wire(&ui);
    let window = Rc::new(Window { ui, core });
    WINDOW.with(|w| *w.borrow_mut() = Some(window.clone()));
    window.refresh();
    window.refresh_stats();
    window.refresh_win_v();
    window.ui.set_version(
        tr_with(
            "settings.version",
            &[("version", env!("CARGO_PKG_VERSION"))],
        )
        .into(),
    );
    if let Err(e) = window.ui.show() {
        tracing::error!("settings window did not show: {e}");
        return;
    }
    // The attributes hook keeps every window off the taskbar; this one is a
    // real window and belongs there.
    window.ui.window().with_winit_window(|w| {
        use slint::winit_030::winit::platform::windows::WindowExtWindows;
        w.set_skip_taskbar(false);
        w.focus_window();
    });
}

/// Renders the settings window into a PNG without activating it, then
/// quits. See `crate::Snapshot`.
pub fn snapshot(core: Rc<Core>, spec: crate::Snapshot) {
    let ui = match SettingsWindow::new() {
        Ok(ui) => ui,
        Err(e) => {
            tracing::error!("settings window could not be built: {e}");
            let _ = slint::quit_event_loop();
            return;
        }
    };
    let window = Rc::new(Window { ui, core });
    WINDOW.with(|w| *w.borrow_mut() = Some(window.clone()));
    window.refresh();
    window.refresh_stats();
    window.refresh_win_v();
    window.ui.set_version(
        tr_with(
            "settings.version",
            &[("version", env!("CARGO_PKG_VERSION"))],
        )
        .into(),
    );
    if let Some(theme) = &spec.theme {
        crate::theme::apply(&window.ui, theme, "");
    }
    if let Some(lang) = &spec.lang {
        crate::i18n::apply(&window.ui, lang);
    }
    window.ui.set_section(spec.section);
    let _ = window.ui.show();
    slint::Timer::single_shot(Duration::from_millis(300), || {
        with(|w| {
            w.ui.window()
                .set_size(slint::LogicalSize::new(960.0, 660.0))
        });
    });
    slint::Timer::single_shot(Duration::from_millis(1200), move || {
        with(|w| match w.ui.window().take_snapshot() {
            Ok(buf) => {
                let img =
                    image::RgbaImage::from_raw(buf.width(), buf.height(), buf.as_bytes().to_vec());
                match img.map(|i| i.save(&spec.path)) {
                    Some(Ok(())) => tracing::info!("snapshot saved to {}", spec.path.display()),
                    Some(Err(e)) => tracing::error!("snapshot not saved: {e}"),
                    None => tracing::error!("snapshot buffer has the wrong size"),
                }
            }
            Err(e) => tracing::error!("snapshot failed: {e}"),
        });
        let _ = slint::quit_event_loop();
    });
}

/// Settings changed somewhere (this window, the popup, the tray, the file).
pub fn settings_changed() {
    with(|w| w.refresh());
}

/// The store changed: keep the usage meter honest while the window is open.
pub fn history_changed() {
    with(|w| w.refresh_stats());
}

pub fn store_progress(p: StoreProgress) {
    with(|w| {
        if p.phase == "completed" {
            w.ui.set_progress(SharedString::new());
            return;
        }
        let text = if p.total > 0 {
            format!("{} — {} / {}", p.phase, p.done, p.total)
        } else {
            p.phase.clone()
        };
        let fraction = if p.total > 0 {
            (p.done as f32 / p.total as f32).min(1.0)
        } else {
            0.0
        };
        w.ui.set_progress(text.into());
        w.ui.set_progress_fraction(fraction);
    });
}

fn wire(ui: &SettingsWindow) {
    ui.window().on_close_requested(|| {
        // Dropped after this handler returns, not inside it.
        slint::Timer::single_shot(Duration::from_millis(0), || {
            WINDOW.with(|w| w.borrow_mut().take());
        });
        slint::CloseRequestResponse::HideWindow
    });
    ui.on_set_bool(|path, value| {
        with(|w| {
            if path == "behavior.captureEnabled" {
                let r = w.core.set_capture_enabled(value);
                w.report(r.map(|_| ()));
            } else {
                w.patch(&path, json!(value));
            }
        });
    });
    ui.on_set_number(|path, text| {
        with(|w| w.set_number(&path, &text));
    });
    ui.on_set_text(|path, value| {
        with(|w| w.patch(&path, json!(value.as_str())));
    });
    ui.on_record_hotkey(|text, ctrl, alt, shift, meta| {
        with(|w| w.record_hotkey(&text, ctrl, alt, shift, meta));
    });
    ui.on_set_win_v(|on| {
        with(|w| {
            if let Err(e) = rebuffer_core::settings::set_clipboard_history_enabled(on) {
                w.error(&format!("Could not change Windows clipboard history: {e}"));
            }
            w.refresh_win_v();
        });
    });
    ui.on_pick_store(|| {
        with(|w| w.pick_store());
    });
    ui.on_clean_now(|days| {
        with(|w| {
            let days = days.trim().parse::<u32>().unwrap_or(30);
            match w.core.store.run_cleanup(Some(days)) {
                Ok(r) => w.cleanup_done(r.removed_items, r.freed_bytes),
                Err(e) => w.error(&format!("Clean up failed: {e}")),
            }
            w.refresh_stats();
        });
    });
    ui.on_add_blocked(|name| {
        with(|w| {
            let name = name.trim().to_lowercase();
            let mut list = w.core.settings.get().privacy.blocked_processes;
            if name.is_empty() || list.contains(&name) {
                return;
            }
            list.push(name);
            w.patch("privacy.blockedProcesses", json!(list));
        });
    });
    ui.on_remove_blocked(|name| {
        with(|w| {
            let list: Vec<String> = w
                .core
                .settings
                .get()
                .privacy
                .blocked_processes
                .into_iter()
                .filter(|p| p.as_str() != name.as_str())
                .collect();
            w.patch("privacy.blockedProcesses", json!(list));
        });
    });
    ui.on_export_data(|| {
        with(|w| w.export());
    });
    ui.on_import_data(|mode| {
        with(|w| {
            w.import(if mode == 1 {
                ImportMode::Replace
            } else {
                ImportMode::Merge
            })
        });
    });
    ui.on_clear_history(|include_pinned| {
        with(|w| {
            match w.core.clear_history(include_pinned) {
                Ok(r) => w.cleanup_done(r.removed_items, r.freed_bytes),
                Err(e) => w.error(&e.to_string()),
            }
            w.refresh_stats();
        });
    });
    ui.on_dismiss_error(|| {
        with(|w| w.ui.set_error(SharedString::new()));
    });
}

impl Window {
    fn focus(&self) {
        let _ = self.ui.show();
        self.ui.window().with_winit_window(|w| w.focus_window());
    }

    fn error(&self, text: &str) {
        tracing::warn!("{text}");
        self.ui.set_error(text.into());
    }

    fn report(&self, r: rebuffer_core::error::AppResult<()>) {
        if let Err(e) = r {
            self.error(&e.to_string());
            // The control already moved; put it back where the settings are.
            self.refresh();
        }
    }

    /// Sets one field, given as a dotted path in the settings.json shape.
    fn patch(&self, path: &str, value: Value) {
        let mut patch = value;
        for key in path.rsplit('.') {
            patch = json!({ key: patch });
        }
        let r = self.core.update_settings(patch).map(|_| ());
        self.report(r);
    }

    /// A number box was committed. Out-of-range values are clamped like the
    /// WebView NumberField does; junk puts the field back.
    fn set_number(&self, path: &str, text: &str) {
        let text = text.trim();
        let s = self.core.settings.get();
        if path == "storage.maxStoreMB" && text.is_empty() {
            self.patch("storage.maxStoreBytes", Value::Null);
            return;
        }
        let Ok(n) = text.parse::<u64>() else {
            self.refresh();
            return;
        };
        match path {
            "storage.retentionDays" | "storage.tempFilesDays" => {
                self.patch(path, json!(n.clamp(1, u64::from(u32::MAX))))
            }
            "storage.maxItemMB" => self.patch("storage.maxItemBytes", json!(n.max(1) * MB)),
            "storage.maxStoreMB" => self.patch("storage.maxStoreBytes", json!(n.max(1) * MB)),
            "window.percentOfMonitor" => self.patch(path, json!(n.clamp(10, 100))),
            "window.zoomStep" => self.patch(path, json!(n.clamp(1, 5))),
            "window.fixed.width" => self.patch(
                "window.fixed",
                json!({ "width": n.clamp(320, 3840), "height": s.window.fixed.height }),
            ),
            "window.fixed.height" => self.patch(
                "window.fixed",
                json!({ "width": s.window.fixed.width, "height": n.clamp(320, 2160) }),
            ),
            other => tracing::warn!("number for unknown setting {other}"),
        }
    }

    // -- hotkey -------------------------------------------------------------------

    /// One key press while recording. Mirrors formatChord / checkReserved /
    /// advisoryWarnings in Settings.svelte.
    fn record_hotkey(&self, text: &str, ctrl: bool, alt: bool, shift: bool, meta: bool) {
        let Some(key) = chord_key(text) else {
            // A lone modifier: keep waiting for the key it modifies.
            if is_modifier(text) {
                return;
            }
            self.ui.set_hotkey_error(tr("hotkey.errInvalidKey").into());
            return;
        };
        if meta {
            if let Some((_, name_key)) = key
                .chars()
                .next()
                .filter(|_| key.len() == 1)
                .and_then(|c| WIN_RESERVED.iter().find(|(r, _)| *r == c))
            {
                let name = tr(name_key);
                self.ui
                    .set_hotkey_error(tr_with("hotkey.reservedChord", &[("name", &name)]).into());
                return;
            }
        }
        let mut parts = Vec::new();
        if ctrl {
            parts.push("Ctrl");
        }
        if alt {
            parts.push("Alt");
        }
        if shift {
            parts.push("Shift");
        }
        if meta {
            parts.push("Win");
        }
        parts.push(&key);
        let chord = parts.join("+");

        let mut warnings = Vec::new();
        if !ctrl && !alt && !shift && !meta {
            warnings.push(tr("hotkey.warnBareKey"));
        }
        if ctrl && !meta && matches!(key.as_str(), "C" | "X" | "V") {
            warnings.push(tr("hotkey.warnSystemCopy"));
        }
        if meta {
            warnings.push(tr("hotkey.warnWinChord"));
        }
        if chord == "Alt+V" {
            warnings.push(tr("hotkey.infoDefault"));
        }

        self.ui.set_recording(false);
        match self
            .core
            .update_settings(json!({ "hotkey": { "binding": chord } }))
        {
            Ok(_) => {
                self.ui.set_hotkey_error(SharedString::new());
                self.ui.set_hotkey_warning(warnings.join(" ").into());
            }
            Err(e) => {
                self.ui.set_hotkey_error(e.to_string().into());
                self.refresh();
            }
        }
    }

    // -- storage and data -----------------------------------------------------------

    fn pick_store(&self) {
        let Some(dir) = rfd::FileDialog::new()
            .set_title(tr("dialog.chooseStore"))
            .pick_folder()
        else {
            return;
        };
        if let Err(e) = self.core.relocate_store(&dir) {
            self.error(&format!("Could not move the store: {e}"));
        }
        self.ui.set_progress(SharedString::new());
        self.refresh();
        self.refresh_stats();
    }

    fn export(&self) {
        let Some(target) = rfd::FileDialog::new()
            .set_title(tr("dialog.exportTitle"))
            .set_file_name("rebuffer-export.rbx")
            .add_filter(tr("dialog.archiveFilter"), &["rbx"])
            .save_file()
        else {
            return;
        };
        if let Err(e) = self.core.export(&target) {
            self.error(&format!("Export failed: {e}"));
        }
        self.ui.set_progress(SharedString::new());
    }

    fn import(&self, mode: ImportMode) {
        let Some(archive) = rfd::FileDialog::new()
            .set_title(tr("dialog.importTitle"))
            .add_filter(tr("dialog.archiveFilter"), &["rbx"])
            .pick_file()
        else {
            return;
        };
        if let Err(e) = self.core.import(&archive, mode) {
            self.error(&format!("Import failed: {e}"));
        }
        self.ui.set_progress(SharedString::new());
        self.refresh();
        self.refresh_stats();
    }

    fn cleanup_done(&self, count: i64, bytes: i64) {
        self.ui.set_cleanup_result(
            tr_with(
                "settings.cleanupDone",
                &[("count", &count.to_string()), ("size", &fmt_bytes(bytes))],
            )
            .into(),
        );
    }

    // -- refresh ---------------------------------------------------------------------

    /// Re-reads the settings into the window: values, theme, language.
    fn refresh(&self) {
        let s = self.core.settings.get();
        crate::theme::apply(&self.ui, &s.appearance.theme, &s.appearance.accent);
        self.ui
            .global::<crate::ui::Theme>()
            .set_reduce_motion(s.appearance.reduce_motion);
        crate::i18n::apply(&self.ui, &s.appearance.language);
        self.ui.set_data(data(&s));
        // Six to a row fits the narrowest window.
        let mut swatches = crate::theme::swatches(|name| tr(&format!("theme.{name}")));
        let more = swatches.split_off(swatches.len().min(6));
        self.ui.set_themes(ModelRc::new(VecModel::from(swatches)));
        self.ui.set_themes_more(ModelRc::new(VecModel::from(more)));
        let langs: Vec<LangOption> = crate::i18n::LANGUAGES
            .iter()
            .map(|(code, name)| LangOption {
                code: (*code).into(),
                name: (*name).into(),
            })
            .collect();
        self.ui.set_languages(ModelRc::new(VecModel::from(langs)));
        let accents: Vec<AccentPreset> = ACCENTS
            .iter()
            .filter_map(|hex| {
                crate::theme::parse_color(hex).map(|color| AccentPreset {
                    hex: (*hex).into(),
                    color,
                })
            })
            .collect();
        self.ui.set_accents(ModelRc::new(VecModel::from(accents)));
    }

    fn refresh_stats(&self) {
        let Ok(stats) = self.core.store.stats() else {
            return;
        };
        self.ui.set_usage_summary(
            tr_with(
                "settings.usageSummary",
                &[
                    ("size", &fmt_bytes(stats.total_bytes)),
                    ("count", &stats.total_items.to_string()),
                ],
            )
            .into(),
        );
        let total = stats.total_bytes.max(1) as f32;
        let segs: Vec<UsageSeg> = stats
            .by_kind
            .iter()
            .map(|k| UsageSeg {
                fraction: k.bytes as f32 / total,
                color: kind_color(&k.kind),
                label: tr_with(
                    "settings.usageKind",
                    &[
                        ("kind", &tr(&format!("kind.{}", k.kind))),
                        ("count", &k.count.to_string()),
                        ("size", &fmt_bytes(k.bytes)),
                    ],
                )
                .into(),
            })
            .collect();
        self.ui.set_usage(ModelRc::new(VecModel::from(segs)));
    }

    /// Windows clipboard history is OS state the user can change elsewhere,
    /// so it is read, not remembered.
    fn refresh_win_v(&self) {
        match rebuffer_core::settings::clipboard_history_enabled() {
            Ok(on) => self.ui.set_win_v(i32::from(on)),
            Err(e) => self.error(&format!("Could not read Windows clipboard history: {e}")),
        }
    }
}

fn data(s: &Settings) -> SettingsData {
    let a = &s.appearance;
    let effective_accent = crate::theme::parse_color(a.accent.trim())
        .unwrap_or_else(|| crate::theme::accent_of(&a.theme));
    SettingsData {
        hotkey: s.hotkey.binding.clone().into(),
        aggressive: s.hotkey.aggressive_mode,
        launch_on_startup: s.behavior.launch_on_startup,
        silent_start: s.behavior.silent_start,
        capture_enabled: s.behavior.capture_enabled,
        auto_paste: s.behavior.auto_paste,
        paste_plain: s.behavior.paste_as_plain_text,
        close_on_copy: s.behavior.close_on_copy,
        store_path: s.storage.path.clone().into(),
        retention_days: s.storage.retention_days.to_string().into(),
        temp_days: s.storage.temp_files_days.to_string().into(),
        max_item_mb: (s.storage.max_item_bytes / MB).max(1).to_string().into(),
        store_cap_mb: s
            .storage
            .max_store_bytes
            .map(|b| (b / MB).max(1).to_string())
            .unwrap_or_default()
            .into(),
        notify_full: s.storage.notify_when_full,
        theme: a.theme.clone().into(),
        language: a.language.clone().into(),
        size_percent: s.window.size_mode != "fixed",
        percent: s.window.percent_of_monitor.to_string().into(),
        fixed_w: s.window.fixed.width.to_string().into(),
        fixed_h: s.window.fixed.height.to_string().into(),
        drag_bar: s.window.drag_bar,
        zoom: s.window.zoom_step.to_string().into(),
        show_age: a.show_age,
        time_format: match a.time_format.as_str() {
            "clock24" => 1,
            "clock12" => 2,
            _ => 0,
        },
        label_size: match a.format_label_size.as_str() {
            "off" => 0,
            "small" => 1,
            "large" => 3,
            _ => 2,
        },
        animate_gifs: a.animate_gifs,
        reduce_motion: a.reduce_motion,
        accent: a.accent.clone().into(),
        effective_accent,
        respect_flags: s.privacy.respect_clipboard_flags,
        link_previews: s.privacy.link_previews,
        blocked: ModelRc::new(VecModel::from(
            s.privacy
                .blocked_processes
                .iter()
                .map(|p| SharedString::from(p.as_str()))
                .collect::<Vec<_>>(),
        )),
    }
}

/// The legend colors of the WebView storage meter.
fn kind_color(kind: &str) -> slint::Color {
    let hex = match kind {
        "image" => "#7aa2ff",
        "video" => "#bb9af7",
        "text" => "#9ece6a",
        "file" => "#e0af68",
        _ => "#565f89",
    };
    crate::theme::parse_color(hex).unwrap_or_default()
}

/// Slint's text for a key, as a chord key: A-Z, 0-9 or F1-F24. Slint passes
/// the key itself with Ctrl held, never the control character - which would
/// be ambiguous anyway: U+0010..U+0018 are its codes for the modifier keys.
fn chord_key(text: &str) -> Option<String> {
    let mut chars = text.chars();
    let c = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    if c.is_ascii_alphabetic() || c.is_ascii_digit() {
        return Some(c.to_ascii_uppercase().to_string());
    }
    // Slint's F1..F24 are U+F704..U+F71B.
    let code = c as u32;
    if (0xF704..=0xF71B).contains(&code) {
        return Some(format!("F{}", code - 0xF704 + 1));
    }
    None
}

fn is_modifier(text: &str) -> bool {
    matches!(text.chars().next(), Some('\u{10}'..='\u{18}'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_map_to_chord_names() {
        assert_eq!(chord_key("v").as_deref(), Some("V"));
        assert_eq!(chord_key("V").as_deref(), Some("V"));
        assert_eq!(chord_key("7").as_deref(), Some("7"));
        assert_eq!(chord_key("\u{16}"), None);
        assert_eq!(chord_key("\u{F704}").as_deref(), Some("F1"));
        assert_eq!(chord_key("\u{F71B}").as_deref(), Some("F24"));
        assert_eq!(chord_key("\u{1b}"), None);
        assert_eq!(chord_key("ab"), None);
        assert!(is_modifier("\u{11}"));
        assert!(!is_modifier("a"));
    }
}
