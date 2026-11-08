//! Data and decoding shared by the UI spikes, so every spike shows the same
//! cards and pays the same decode cost: what differs between them is only the
//! toolkit.

use std::path::Path;
use std::time::Duration;

use serde::Deserialize;

/// One card, as exported read-only from the real store by
/// `export_items.py`.
#[derive(Debug, Clone, Deserialize)]
pub struct Item {
    pub id: i64,
    pub kind: String,
    pub title: String,
    pub text: String,
    pub thumb: Option<String>,
    pub animated: Option<String>,
    pub ext: String,
    pub ts: i64,
}

/// Strings in every script the interface ships, mixed in among the real
/// cards: the spike has to prove it shapes Arabic and lays out CJK, not that
/// it can draw ASCII.
const SAMPLES: &[&str] = &[
    "مرحبا بالعالم — نسخ هذا النص إلى الحافظة",
    "سجل الحافظة: 42 عنصرًا، آخر نسخ منذ 3 دقائق",
    "クリップボードの履歴を検索します",
    "東京都渋谷区の住所をコピーしました",
    "剪贴板历史记录已保存",
    "Привет, это строка из буфера обмена",
    "Grüße aus der Zwischenablage — äöüß",
    "Mixed: Rebuffer نسخة 1.2 بيتا (beta) テスト",
];

/// The real items, cycled out to `count` cards, with a sample string every
/// seventh card. Ids stay unique.
pub fn load(path: &Path, count: usize) -> Vec<Item> {
    let raw = std::fs::read(path).expect("items.json is readable");
    let real: Vec<Item> = serde_json::from_slice(&raw).expect("items.json is valid");
    assert!(!real.is_empty(), "items.json has no items");
    (0..count)
        .map(|i| {
            if i % 7 == 3 {
                Item {
                    id: -(i as i64) - 1,
                    kind: "text".into(),
                    title: String::new(),
                    text: SAMPLES[(i / 7) % SAMPLES.len()].into(),
                    thumb: None,
                    animated: None,
                    ext: String::new(),
                    ts: 0,
                }
            } else {
                let mut it = real[i % real.len()].clone();
                it.id = i as i64;
                it
            }
        })
        .collect()
}

/// The text a card shows when it has no picture.
pub fn label(item: &Item) -> String {
    if !item.title.is_empty() {
        item.title.clone()
    } else if !item.text.is_empty() {
        item.text.clone()
    } else {
        format!("{} {}", item.kind, item.ext)
    }
}

pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// A thumbnail decoded and shrunk to fit `max` on its long side.
pub fn decode_thumb(path: &str, max: u32) -> Option<Rgba> {
    let img = image::open(path).ok()?;
    let img = if img.width() > max || img.height() > max {
        img.thumbnail(max, max)
    } else {
        img
    };
    let rgba = img.to_rgba8();
    Some(Rgba {
        width: rgba.width(),
        height: rgba.height(),
        pixels: rgba.into_raw(),
    })
}

pub struct Frame {
    pub image: Rgba,
    pub delay: Duration,
}

/// Every frame of a GIF, shrunk like a thumbnail. Only ever called for cards
/// on screen.
pub fn decode_gif(path: &str, max: u32) -> Option<Vec<Frame>> {
    use image::AnimationDecoder;
    let file = std::io::BufReader::new(std::fs::File::open(path).ok()?);
    let decoder = image::codecs::gif::GifDecoder::new(file).ok()?;
    let frames = decoder.into_frames().collect_frames().ok()?;
    Some(
        frames
            .into_iter()
            .map(|f| {
                let (n, d) = f.delay().numer_denom_ms();
                let delay = Duration::from_millis((n / d.max(1)).max(20) as u64);
                let img = image::DynamicImage::ImageRgba8(f.into_buffer()).thumbnail(max, max);
                let rgba = img.to_rgba8();
                Frame {
                    image: Rgba {
                        width: rgba.width(),
                        height: rgba.height(),
                        pixels: rgba.into_raw(),
                    },
                    delay,
                }
            })
            .collect(),
    )
}
