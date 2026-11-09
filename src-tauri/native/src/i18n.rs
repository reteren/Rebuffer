//! Translations, from the locale JSON in `locales/` (kept in the shape the
//! WebView front end used, so the files carried over unchanged).
//!
//! Lookup as that front end did it: nested keys flattened with
//! dots, `{name}` placeholders, English for anything a locale lacks, and
//! `"system"` resolved against the languages Windows shows its own UI in.

use std::cell::RefCell;
use std::collections::HashMap;

use slint::ComponentHandle;

use crate::ui::I18n;

const LOCALES: &[(&str, &str)] = &[
    ("en", include_str!("../locales/en.json")),
    ("ru", include_str!("../locales/ru.json")),
    ("de", include_str!("../locales/de.json")),
    ("es", include_str!("../locales/es.json")),
    ("pt", include_str!("../locales/pt.json")),
    ("it", include_str!("../locales/it.json")),
    ("zh", include_str!("../locales/zh.json")),
    ("ja", include_str!("../locales/ja.json")),
    ("fr", include_str!("../locales/fr.json")),
    ("ar", include_str!("../locales/ar.json")),
];

type Dict = HashMap<String, String>;

struct State {
    code: &'static str,
    active: Dict,
    english: Dict,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

fn flatten(prefix: &str, value: &serde_json::Value, out: &mut Dict) {
    match value {
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                let key = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                flatten(&key, v, out);
            }
        }
        serde_json::Value::String(s) => {
            out.insert(prefix.to_string(), s.clone());
        }
        _ => {}
    }
}

fn load(code: &str) -> Dict {
    let raw = LOCALES
        .iter()
        .find(|(c, _)| *c == code)
        .map(|(_, raw)| *raw)
        .unwrap_or(LOCALES[0].1);
    let mut out = Dict::new();
    match serde_json::from_str::<serde_json::Value>(raw) {
        Ok(v) => flatten("", &v, &mut out),
        Err(e) => tracing::error!("locale {code} is not valid JSON: {e}"),
    }
    out
}

/// `setting` is `appearance.language`: a code, or `"system"`.
fn resolve(setting: &str) -> &'static str {
    let wanted: Vec<String> = if setting == "system" {
        rebuffer_core::settings::system_ui_languages()
    } else {
        vec![setting.to_string()]
    };
    for tag in wanted {
        let primary = tag.split(['-', '_']).next().unwrap_or("").to_lowercase();
        if let Some((code, _)) = LOCALES.iter().find(|(c, _)| *c == primary) {
            return code;
        }
    }
    "en"
}

/// Native names, as the WebView picker shows them, in its order.
pub const LANGUAGES: [(&str, &str); 10] = [
    ("en", "English"),
    ("ar", "العربية"),
    ("zh", "中文"),
    ("fr", "Français"),
    ("de", "Deutsch"),
    ("it", "Italiano"),
    ("ja", "日本語"),
    ("pt", "Português"),
    ("ru", "Русский"),
    ("es", "Español"),
];

/// Loads the strings for `setting` (a code or `"system"`), if they are not the
/// ones loaded already, and returns the code it resolved to.
pub fn set_language(setting: &str) -> &'static str {
    let code = resolve(setting);
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        if s.as_ref().is_some_and(|st| st.code == code) {
            return;
        }
        *s = Some(State {
            code,
            active: load(code),
            english: load("en"),
        });
    });
    code
}

/// Switches the interface language and tells a window to re-read every
/// string. Each window has its own `I18n` global, so each one is applied.
pub fn apply<C>(ui: &C, setting: &str)
where
    C: ComponentHandle,
    for<'a> I18n<'a>: slint::Global<'a, C>,
{
    let code = set_language(setting);
    let global = ui.global::<I18n>();
    global.on_tr(|key, _revision| tr(key.as_str()).into());
    // Always bumped: a window built after the language was loaded still has
    // to evaluate its strings once against it.
    global.set_rtl(code == "ar");
    global.set_revision(global.get_revision() + 1);
}

/// The string for `key`, English when the active locale lacks it, the key
/// itself when English does too (visible, rather than silently blank).
pub fn tr(key: &str) -> String {
    STATE.with(|s| {
        let s = s.borrow();
        let Some(st) = s.as_ref() else {
            return key.to_string();
        };
        st.active
            .get(key)
            .or_else(|| st.english.get(key))
            .cloned()
            .unwrap_or_else(|| key.to_string())
    })
}

/// `tr` with `{name}` placeholders filled.
pub fn tr_with(key: &str, args: &[(&str, &str)]) -> String {
    let mut s = tr(key);
    for (name, value) in args {
        s = s.replace(&format!("{{{name}}}"), value);
    }
    s
}

/// The active locale code, for date formatting.
pub fn code() -> &'static str {
    STATE.with(|s| s.borrow().as_ref().map_or("en", |st| st.code))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_locale_parses_and_has_the_popup_keys() {
        let en = load("en");
        assert!(en.len() > 200);
        for (code, _) in LOCALES {
            let d = load(code);
            for key in ["tabs.all", "menu.copy", "grid.today", "status.items"] {
                assert!(d.contains_key(key), "{code} lacks {key}");
            }
        }
    }

    #[test]
    fn languages_resolve_by_their_primary_subtag() {
        assert_eq!(resolve("ru"), "ru");
        assert_eq!(resolve("pt-BR"), "pt");
        assert_eq!(resolve("xx"), "en");
    }
}
