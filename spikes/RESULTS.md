# Native UI spikes — results (roadmap phase 1)

Measured 2026-11-08 on the owner's machine (Windows 11, release builds,
`opt-level = "s"`, LTO). Both spikes render the same popup: a 4-column grid of
10,000 cards built from 2,038 real store items (cycled) plus multilingual
sample strings every seventh card, a search field, thumbnails decoded off the
UI thread, and GIF animation. The data file is exported read-only from the
live store by a scratch script and is not part of the repository.

Run: `cargo build --release` here, then
`target/release/<spike>.exe <items.json> [--no-gif] [--query <q>] [--scroll-test]`.
Slint's renderer is chosen with `SLINT_BACKEND` (`winit-software`,
`winit-femtovg`).

For scale, the shipping WebView2 build: `rebuffer.exe` 21 MB private plus seven
`msedgewebview2.exe` processes at ≈333 MB private (one snapshot).

| | Slint 1.18.1, software | Slint 1.18.1, femtovg (GL) | GPUI (gpui-kit 0.9 / gpui-pre 0.3.11) |
|---|---|---|---|
| Private memory, idle, first screen | **12.6 MB** | 86 MB | 113 MB |
| Idle CPU, nothing animating | **0 %** | 0 % | 0.6 % |
| Idle CPU, GIFs on screen | 1.25 % | — | 0.6 % |
| After scrolling 2,000 rows | **56.6 MB**, 0 % CPU (240-image LRU) | — | 1,673 MB, 3 % CPU (image cache never evicts) |
| Search over 10,000 cards | 3 ms | 3 ms | not measured |
| Arabic shaping and RTL order | **correct** (joined letters, mixed-direction lines ordered right) | correct | **broken**: letters unjoined, runs drawn left to right |
| Japanese / Chinese / Cyrillic | correct | correct | correct |
| Animated GIF | works (own frame timer; built in from Slint 1.19) | works | works (built in) |
| Acrylic backdrop | `window-vibrancy` applies without error; not visually confirmed | same | not tried |
| IME in the search field | not verified (needs a person at the keyboard) | | not verified |

## Verdict

**Slint.** It passes every criterion that could be checked by machine, and the
software renderer idles at 12.6 MB — about 1/25 of the WebView build. GPUI is
out on Arabic alone, which is one of the ten shipped locales; its idle CPU, its
unbounded image cache and its packaging (weekly `gpui-pre` snapshots of Zed,
the wrapper crate renamed within the last month) are further reasons, not the
deciding one.

## Open points for Slint

- **License.** Slint is GPLv3, Royalty-free, or Commercial. The Royalty-free
  license allows this MIT app at no cost on the condition that the app
  discloses it uses Slint (`AboutSlint` widget or the Slint badge) — e.g. in the
  About panel. Owner's call.
- **Paragraph alignment.** Arabic lines shape and order correctly but sit
  left-aligned in a left-to-right layout; the Arabic locale needs the layout
  mirrored and text right-aligned explicitly.
- **IME** and the **acrylic look** need a hands-on check.
- **Renderer.** Software is the leanest and was enough here; femtovg costs
  ~70 MB in the GL driver. Skia was not tried.
- **Thumbnail cache.** Decode to card size, not 256 px, and cap by bytes: the
  56 MB after a long scroll is almost entirely the cache.
