//! Themes, from the CSS files in `themes/` — the WebView front end's theme
//! files, carried over unchanged.
//!
//! Each file declares `--token: value;` pairs under `[data-theme="<name>"]`.
//! Only plain colors are taken (`#rgb`, `#rrggbb`, `rgba(r, g, b, a)`);
//! gradients and shadows have no direct Slint counterpart and are skipped.
//! A token a theme does not define keeps the darkblue value.

use slint::{Color, ComponentHandle};

use crate::ui::{Theme, ThemeSwatch};

// In the order of the WebView's THEMES list (types.ts), which the settings
// strip shows.
const FILES: &[(&str, &str)] = &[
    ("darkblue", include_str!("../themes/darkblue.css")),
    ("black", include_str!("../themes/black.css")),
    ("light", include_str!("../themes/light.css")),
    ("grey", include_str!("../themes/grey.css")),
    ("skyblue", include_str!("../themes/skyblue.css")),
    ("dark-green", include_str!("../themes/dark-green.css")),
    ("dark-purple", include_str!("../themes/dark-purple.css")),
    ("ember", include_str!("../themes/ember.css")),
    ("ocean", include_str!("../themes/ocean.css")),
    ("wine", include_str!("../themes/wine.css")),
    ("paper", include_str!("../themes/paper.css")),
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
    let window = over(theme.get_bg_1(), theme.get_bg_0());
    theme.set_window_base(window);
    let glass = theme.get_surface_3();
    theme.set_menu_bg(over(
        glass.with_alpha(f32::from(glass.alpha()) / 255.0 * 0.88),
        window,
    ));
    set_controls(&theme, css_of(name).contains("color-scheme: light"));
    theme.set_card_corner(over(theme.get_surface_1(), window));
}

/// The colors Chromium paints a checkbox or radio in, with `accent-color` set
/// to the accent: the WebView front end used the stock controls. The mark on
/// a checked box, and the ring inside a checked radio, is whichever of light
/// or dark reads on the accent.
fn set_controls(theme: &Theme<'_>, light: bool) {
    let (fill, border) = if light {
        (
            Color::from_rgb_u8(0xff, 0xff, 0xff),
            Color::from_rgb_u8(0x76, 0x76, 0x76),
        )
    } else {
        (
            Color::from_rgb_u8(0x3b, 0x3b, 0x3b),
            Color::from_rgb_u8(0x85, 0x85, 0x85),
        )
    };
    theme.set_control_fill(fill);
    theme.set_control_border(border);
    let a = theme.get_accent();
    let luma =
        0.2126 * f32::from(a.red()) + 0.7152 * f32::from(a.green()) + 0.0722 * f32::from(a.blue());
    theme.set_control_mark(if luma > 140.0 {
        Color::from_rgb_u8(0x3b, 0x3b, 0x3b)
    } else {
        Color::from_rgb_u8(0xff, 0xff, 0xff)
    });
}

/// `top` composited over an opaque `bottom` (source-over).
fn over(top: Color, bottom: Color) -> Color {
    let a = f32::from(top.alpha()) / 255.0;
    let mix = |t: u8, b: u8| (f32::from(t) * a + f32::from(b) * (1.0 - a)).round() as u8;
    Color::from_rgb_u8(
        mix(top.red(), bottom.red()),
        mix(top.green(), bottom.green()),
        mix(top.blue(), bottom.blue()),
    )
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
                frame: get("bg-1"),
                frame_border: get("window-border"),
                surface: get("surface-1"),
                surface_2: get("surface-2"),
                border: get("border-1"),
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
