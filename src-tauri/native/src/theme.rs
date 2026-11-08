//! Themes, read from the same CSS files the WebView front end uses.
//!
//! Each file declares `--token: value;` pairs under `[data-theme="<name>"]`.
//! Only plain colors are taken (`#rgb`, `#rrggbb`, `rgba(r, g, b, a)`);
//! gradients and shadows have no direct Slint counterpart and are skipped.
//! A token a theme does not define keeps the darkblue value.

use slint::{Color, ComponentHandle};

use crate::ui::{Theme, ThemeSwatch};

const FILES: &[(&str, &str)] = &[
    (
        "darkblue",
        include_str!("../../../src/lib/styles/themes/darkblue.css"),
    ),
    (
        "black",
        include_str!("../../../src/lib/styles/themes/black.css"),
    ),
    (
        "dark-green",
        include_str!("../../../src/lib/styles/themes/dark-green.css"),
    ),
    (
        "dark-purple",
        include_str!("../../../src/lib/styles/themes/dark-purple.css"),
    ),
    (
        "ember",
        include_str!("../../../src/lib/styles/themes/ember.css"),
    ),
    (
        "grey",
        include_str!("../../../src/lib/styles/themes/grey.css"),
    ),
    (
        "light",
        include_str!("../../../src/lib/styles/themes/light.css"),
    ),
    (
        "ocean",
        include_str!("../../../src/lib/styles/themes/ocean.css"),
    ),
    (
        "paper",
        include_str!("../../../src/lib/styles/themes/paper.css"),
    ),
    (
        "skyblue",
        include_str!("../../../src/lib/styles/themes/skyblue.css"),
    ),
    (
        "wine",
        include_str!("../../../src/lib/styles/themes/wine.css"),
    ),
];

/// Every `--token: <color>` in `css`, in file order.
fn tokens(css: &str) -> Vec<(&str, Color)> {
    css.lines()
        .filter_map(|line| {
            let line = line.trim();
            let rest = line.strip_prefix("--")?;
            let (name, value) = rest.split_once(':')?;
            let value = value.split(';').next()?.split("/*").next()?.trim();
            Some((name.trim(), parse_color(value)?))
        })
        .collect()
}

pub(crate) fn parse_color(v: &str) -> Option<Color> {
    if let Some(hex) = v.strip_prefix('#') {
        let n = |s: &str| u8::from_str_radix(s, 16).ok();
        return match hex.len() {
            3 => {
                let d: Vec<u8> = hex
                    .chars()
                    .map(|c| u8::from_str_radix(&c.to_string(), 16).ok().map(|x| x * 17))
                    .collect::<Option<_>>()?;
                Some(Color::from_rgb_u8(d[0], d[1], d[2]))
            }
            6 => Some(Color::from_rgb_u8(
                n(&hex[0..2])?,
                n(&hex[2..4])?,
                n(&hex[4..6])?,
            )),
            8 => Some(Color::from_argb_u8(
                n(&hex[6..8])?,
                n(&hex[0..2])?,
                n(&hex[2..4])?,
                n(&hex[4..6])?,
            )),
            _ => None,
        };
    }
    let inner = v.strip_prefix("rgba(").or_else(|| v.strip_prefix("rgb("))?;
    let inner = inner.strip_suffix(')')?;
    let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
    let c = |i: usize| parts.get(i)?.parse::<f32>().ok();
    let (r, g, b) = (c(0)?, c(1)?, c(2)?);
    let a = if parts.len() > 3 { c(3)? } else { 1.0 };
    Some(Color::from_argb_u8(
        (a.clamp(0.0, 1.0) * 255.0).round() as u8,
        r as u8,
        g as u8,
        b as u8,
    ))
}

/// The names `appearance.theme` may hold.
#[cfg(test)]
pub fn names() -> impl Iterator<Item = &'static str> {
    FILES.iter().map(|(n, _)| *n)
}

fn css_of(name: &str) -> &'static str {
    FILES
        .iter()
        .find(|(n, _)| *n == name)
        .or_else(|| FILES.first())
        .map(|(_, css)| *css)
        .unwrap_or_default()
}

/// Applies `name` (falling back to darkblue for an unknown name) to a
/// window's `Theme` global, then `accent` over it when the user picked one.
/// Every window has its own instance of the global, so each one is applied.
pub fn apply<C>(ui: &C, name: &str, accent: &str)
where
    C: ComponentHandle,
    for<'a> Theme<'a>: slint::Global<'a, C>,
{
    // Darkblue first, so a token missing from `name` keeps a sane value
    // instead of whatever the previous theme left behind.
    let base = tokens(FILES[0].1);
    let theme = ui.global::<Theme>();
    for (token, color) in base.into_iter().chain(tokens(css_of(name))) {
        set(&theme, token, color);
    }
    if let Some(c) = parse_color(accent.trim()) {
        theme.set_accent(c);
    }
}

/// The accent a theme brings, for the settings window's color row.
pub fn accent_of(name: &str) -> Color {
    tokens(css_of(name))
        .into_iter()
        .find(|(t, _)| *t == "accent")
        .map(|(_, c)| c)
        .unwrap_or_else(|| Color::from_rgb_u8(0x7a, 0xa2, 0xff))
}

/// A miniature of every theme for the settings window.
pub fn swatches(label: impl Fn(&str) -> String) -> Vec<ThemeSwatch> {
    FILES
        .iter()
        .map(|(name, css)| {
            let t = tokens(css);
            let get = |k: &str| {
                t.iter()
                    .find(|(n, _)| *n == k)
                    .map(|(_, c)| *c)
                    .unwrap_or_default()
            };
            ThemeSwatch {
                name: (*name).into(),
                label: label(name).into(),
                bg: get("bg-0"),
                surface: get("surface-2"),
                accent: get("accent"),
                text: get("text-1"),
            }
        })
        .collect()
}

fn set(t: &Theme<'_>, token: &str, c: Color) {
    match token {
        "bg-0" => t.set_bg_0(c),
        "bg-1" => t.set_bg_1(c),
        "surface-1" => t.set_surface_1(c),
        "surface-2" => t.set_surface_2(c),
        "surface-3" => t.set_surface_3(c),
        "surface-4" => t.set_surface_4(c),
        "overlay-1" => t.set_overlay_1(c),
        "scrim-1" => t.set_scrim_1(c),
        "scrim-2" => t.set_scrim_2(c),
        "border-1" => t.set_border_1(c),
        "border-2" => t.set_border_2(c),
        "border-3" => t.set_border_3(c),
        "window-border" => t.set_window_border(c),
        "text-1" => t.set_text_1(c),
        "text-2" => t.set_text_2(c),
        "text-3" => t.set_text_3(c),
        "text-bright" => t.set_text_bright(c),
        "accent" => t.set_accent(c),
        "on-accent" => t.set_on_accent(c),
        "ext-1" => t.set_ext_1(c),
        "ext-2" => t.set_ext_2(c),
        "ext-3" => t.set_ext_3(c),
        "ext-4" => t.set_ext_4(c),
        "ext-5" => t.set_ext_5(c),
        "ext-6" => t.set_ext_6(c),
        "ext-7" => t.set_ext_7(c),
        "danger" => t.set_danger(c),
        "ok" => t.set_ok(c),
        "warn" => t.set_warn(c),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_parse_in_every_form_the_themes_use() {
        assert_eq!(parse_color("#fff"), Some(Color::from_rgb_u8(255, 255, 255)));
        assert_eq!(
            parse_color("#7aa2ff"),
            Some(Color::from_rgb_u8(0x7a, 0xa2, 0xff))
        );
        assert_eq!(
            parse_color("rgba(16, 18, 24, 0.72)"),
            Some(Color::from_argb_u8(184, 16, 18, 24))
        );
        assert_eq!(parse_color("0 1px 2px rgba(0, 0, 0, 0.35)"), None);
        assert_eq!(parse_color("radial-gradient(130% 90% at 8% -12%)"), None);
    }

    #[test]
    fn every_theme_defines_the_core_tokens() {
        for (name, css) in FILES {
            let found: Vec<&str> = tokens(css).into_iter().map(|(t, _)| t).collect();
            for needed in [
                "bg-0",
                "surface-1",
                "text-1",
                "text-2",
                "accent",
                "border-1",
            ] {
                assert!(found.contains(&needed), "theme {name} lacks --{needed}");
            }
        }
        assert_eq!(names().count(), 11);
    }
}
