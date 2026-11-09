//! The theme's `--window-wash`: the soft radial glows the WebView painted on
//! `body`, over the acrylic backdrop and under `--bg`.
//!
//! Every glow is sized and placed in percent of the window, so the field is
//! the same at any window size: it is rendered once per theme into a small
//! image, and the windows stretch it over themselves.

use slint::{Image, Rgba8Pixel, SharedPixelBuffer};

use crate::theme::parse_color;

/// The rendered field's size. The glows have no edges, so stretching this
/// over a window loses nothing that shows.
const W: u32 = 192;
const H: u32 = 128;

/// `radial-gradient(<rx>% <ry>% at <cx>% <cy>%, <color>, transparent <stop>%)`,
/// in fractions of the window.
#[derive(Debug, Clone, PartialEq)]
struct Glow {
    rx: f32,
    ry: f32,
    cx: f32,
    cy: f32,
    rgba: [f32; 4],
    stop: f32,
}

/// The glows of a theme file's `--window-wash`, top layer first, as CSS
/// lists them. Anything outside the one form the themes use is skipped.
fn glows(css: &str) -> Vec<Glow> {
    let Some(start) = css.find("--window-wash:") else {
        return Vec::new();
    };
    let rest = &css[start + "--window-wash:".len()..];
    let value = &rest[..rest.find(';').unwrap_or(rest.len())];
    value
        .split("radial-gradient(")
        .skip(1)
        .filter_map(|g| parse_glow(g.trim()))
        .collect()
}

fn parse_glow(g: &str) -> Option<Glow> {
    let (shape, rest) = g.split_once(',')?;
    let (size, at) = shape.split_once(" at ")?;
    let pct = |s: &str| {
        s.trim()
            .strip_suffix('%')?
            .parse::<f32>()
            .ok()
            .map(|v| v / 100.0)
    };
    let mut size = size.split_whitespace();
    let (rx, ry) = (pct(size.next()?)?, pct(size.next()?)?);
    let mut at = at.split_whitespace();
    let (cx, cy) = (pct(at.next()?)?, pct(at.next()?)?);
    // The color is an rgba(...) holding commas of its own.
    let close = rest.find(')')?;
    let color = parse_color(rest[..=close].trim())?;
    let tail = &rest[close + 1..];
    let stop = tail
        .split_whitespace()
        .find_map(|t| pct(t.trim_end_matches([',', ')'])))
        .unwrap_or(1.0);
    Some(Glow {
        rx,
        ry,
        cx,
        cy,
        rgba: [
            f32::from(color.red()) / 255.0,
            f32::from(color.green()) / 255.0,
            f32::from(color.blue()) / 255.0,
            f32::from(color.alpha()) / 255.0,
        ],
        stop,
    })
}

/// The wash of a theme file, ready to stretch over a window.
pub fn render(css: &str) -> Image {
    let glows = glows(css);
    let mut buf = SharedPixelBuffer::<Rgba8Pixel>::new(W, H);
    let pixels = buf.make_mut_slice();
    for y in 0..H {
        for x in 0..W {
            let u = (x as f32 + 0.5) / W as f32;
            let v = (y as f32 + 0.5) / H as f32;
            // Premultiplied, composited bottom layer first.
            let mut acc = [0.0f32; 4];
            for g in glows.iter().rev() {
                let d = (((u - g.cx) / g.rx).powi(2) + ((v - g.cy) / g.ry).powi(2)).sqrt();
                // The color at 0, fading linearly to transparent at `stop`.
                let a = g.rgba[3] * (1.0 - (d / g.stop).min(1.0));
                for (out, c) in acc.iter_mut().zip(g.rgba).take(3) {
                    *out = c * a + *out * (1.0 - a);
                }
                acc[3] = a + acc[3] * (1.0 - a);
            }
            let alpha = acc[3];
            let un = |p: f32| {
                if alpha > 0.0 {
                    ((p / alpha) * 255.0).round().clamp(0.0, 255.0) as u8
                } else {
                    0
                }
            };
            pixels[(y * W + x) as usize] = Rgba8Pixel {
                r: un(acc[0]),
                g: un(acc[1]),
                b: un(acc[2]),
                a: (alpha * 255.0).round() as u8,
            };
        }
    }
    Image::from_rgba8(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_theme_wash_parses() {
        for name in crate::theme::names() {
            let n = glows(crate::theme::css_of(name)).len();
            assert!(n >= 1, "{name}: no glow parsed");
        }
        let g = glows(crate::theme::css_of("darkblue"));
        assert_eq!(g.len(), 3);
        assert_eq!(
            g[0],
            Glow {
                rx: 1.3,
                ry: 0.9,
                cx: 0.08,
                cy: -0.12,
                rgba: [122.0 / 255.0, 162.0 / 255.0, 1.0, g[0].rgba[3]],
                stop: 0.55,
            }
        );
    }
}
