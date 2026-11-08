//! Phase-1 spike: the Rebuffer popup in GPUI (through gpui-kit).
//!
//! Usage: `gpui-popup <items.json> [--no-gif] [--query <q>] [--scroll-test]`
//!
//! Same cards, same data and the same questions as the Slint spike.

use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::*;
use spike_common::Item;

const COLUMNS: usize = 4;
const ROW_HEIGHT: f32 = 158.0;

struct Popup {
    items: Rc<Vec<Item>>,
    visible: Vec<usize>,
    animate: bool,
    status: SharedString,
    search: Entity<InputState>,
    scroll: UniformListScrollHandle,
    _subscription: Subscription,
}

impl Popup {
    fn new(
        items: Rc<Vec<Item>>,
        animate: bool,
        built: Duration,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search / بحث / 検索"));
        let subscription = cx.subscribe(&search, |this: &mut Popup, state, event: &InputEvent, cx| {
            if let InputEvent::Change = event {
                let q = state.read(cx).value().to_string();
                this.apply_search(&q);
                cx.notify();
            }
        });
        let count = items.len();
        Popup {
            visible: (0..count).collect(),
            items,
            animate,
            status: format!("{count} cards · built in {built:?}").into(),
            search,
            scroll: UniformListScrollHandle::new(),
            _subscription: subscription,
        }
    }

    fn apply_search(&mut self, query: &str) {
        let t = Instant::now();
        let q = query.to_lowercase();
        self.visible = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, it)| q.is_empty() || spike_common::label(it).to_lowercase().contains(&q))
            .map(|(i, _)| i)
            .collect();
        self.status = format!("'{query}': {} matches in {:?}", self.visible.len(), t.elapsed()).into();
    }

    fn card(&self, item: &Item) -> impl IntoElement {
        let kind: SharedString = if item.ext.is_empty() { item.kind.clone() } else { item.ext.clone() }.into();
        let picture = item
            .animated
            .clone()
            .filter(|_| self.animate)
            .or_else(|| item.thumb.clone());
        let body = match picture {
            Some(path) => div()
                .w_full()
                .h(px(ROW_HEIGHT - 8.0 - 22.0))
                .child(img(PathBuf::from(path)).size_full().object_fit(ObjectFit::Cover))
                .into_any_element(),
            None => div()
                .px_2()
                .pt_2()
                .h(px(ROW_HEIGHT - 8.0 - 22.0))
                .overflow_hidden()
                .text_size(px(12.0))
                .text_color(rgb(0xdfe3ee))
                .child(SharedString::from(spike_common::label(item)))
                .into_any_element(),
        };
        div()
            .w(px(176.0))
            .h(px(ROW_HEIGHT - 8.0))
            .rounded(px(8.0))
            .bg(rgb(0x1e2230))
            .border_1()
            .border_color(rgb(0x2c3244))
            .overflow_hidden()
            .flex()
            .flex_col()
            .child(body)
            .child(div().px_2().text_size(px(10.0)).text_color(rgb(0x7d869c)).child(kind))
    }

    fn rows(&self, range: Range<usize>) -> Vec<AnyElement> {
        range
            .map(|row| {
                let start = row * COLUMNS;
                let end = (start + COLUMNS).min(self.visible.len());
                div()
                    .flex()
                    .flex_row()
                    .gap_2()
                    .h(px(ROW_HEIGHT))
                    .children(self.visible[start..end].iter().map(|&i| self.card(&self.items[i])))
                    .into_any_element()
            })
            .collect()
    }
}

impl Render for Popup {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let row_count = self.visible.len().div_ceil(COLUMNS);
        div()
            .size_full()
            .bg(rgb(0x12141a))
            .p_2()
            .flex()
            .flex_col()
            .gap_2()
            .child(Input::new(&self.search))
            .child(
                uniform_list(
                    "cards",
                    row_count,
                    cx.processor(|this: &mut Popup, range: Range<usize>, _window, _cx| this.rows(range)),
                )
                .track_scroll(&self.scroll)
                .flex_1(),
            )
            .child(div().text_size(px(11.0)).text_color(rgb(0x7d869c)).child(self.status.clone()))
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let data = PathBuf::from(args.next().expect("usage: gpui-popup <items.json> [--no-gif] [--query <q>] [--scroll-test]"));
    let rest: Vec<String> = args.collect();
    let animate = !rest.iter().any(|a| a == "--no-gif");
    let scroll_test = rest.iter().any(|a| a == "--scroll-test");
    let query = rest.iter().position(|a| a == "--query").and_then(|i| rest.get(i + 1)).cloned();

    let started = Instant::now();
    let items = Rc::new(spike_common::load(&data, 10_000));

    gpui_kit::application().run(move |cx: &mut App| {
        gpui_kit::init(cx);
        let bounds = Bounds::centered(None, size(px(780.0), px(620.0)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("Rebuffer — GPUI spike".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let items = items.clone();
        let query = query.clone();
        gpui_kit::open_window(options, cx, move |window, cx| {
            let view = cx.new(|cx| {
                let mut p = Popup::new(items, animate, started.elapsed(), window, cx);
                if let Some(q) = &query {
                    p.apply_search(q);
                }
                p
            });
            if scroll_test {
                let weak = view.downgrade();
                cx.spawn(async move |cx| {
                    let t0 = Instant::now();
                    for row in 0..2000usize {
                        cx.background_executor().timer(Duration::from_millis(16)).await;
                        let _ = weak.update(cx, |this, cx| {
                            this.scroll.scroll_to_item(row + 3, ScrollStrategy::Top);
                            cx.notify();
                        });
                    }
                    let _ = weak.update(cx, |this, cx| {
                        this.status = format!("scrolled 2000 rows in {:?}", t0.elapsed()).into();
                        cx.notify();
                    });
                })
                .detach();
            }
            view
        })
        .expect("window");
        eprintln!("ready in {:?}", started.elapsed());
    });
}
