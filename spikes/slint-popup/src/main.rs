//! Phase-1 spike: the Rebuffer popup in Slint.
//!
//! Usage: `slint-popup <items.json> [--no-gif] [--acrylic]`
//!
//! What it has to prove: a virtualized grid of 10,000 cards scrolls without
//! decoding what is off screen; thumbnails decode off the UI thread; GIFs
//! animate; the search field takes IME input; Arabic shapes and runs right to
//! left; and the idle process is small and silent.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use slint::{Image, Model, ModelNotify, ModelRc, ModelTracker, Rgba8Pixel, SharedPixelBuffer, VecModel};
use spike_common::{Frame, Item};

slint::slint! {
    import { LineEdit, ListView } from "std-widgets.slint";

    export struct CardData {
        label: string,
        image: image,
        has-image: bool,
        kind: string,
    }

    export struct RowData {
        cards: [CardData],
    }

    component Card inherits Rectangle {
        in property <CardData> data;
        width: 176px;
        height: 150px;
        border-radius: 8px;
        background: #1e2230;
        border-width: 1px;
        border-color: #2c3244;
        clip: true;

        if data.has-image: Image {
            x: 0; y: 0;
            width: parent.width;
            height: parent.height - 22px;
            source: data.image;
            image-fit: cover;
        }
        if !data.has-image: Text {
            x: 10px; y: 8px;
            width: parent.width - 20px;
            height: parent.height - 36px;
            text: data.label;
            color: #dfe3ee;
            font-size: 12px;
            wrap: word-wrap;
            overflow: elide;
        }
        Text {
            x: 10px; y: parent.height - 20px;
            text: data.kind;
            color: #7d869c;
            font-size: 10px;
        }
    }

    export component MainWindow inherits Window {
        title: "Rebuffer — Slint spike";
        preferred-width: 780px;
        preferred-height: 620px;
        in property <bool> transparent-bg: false;
        background: transparent-bg ? transparent : #12141a;

        in property <[RowData]> rows;
        in property <string> status;
        in-out property <length> scroll-y <=> list.content-y;
        callback search(string);

        VerticalLayout {
            padding: 10px;
            spacing: 8px;
            LineEdit {
                placeholder-text: "Search / بحث / 検索";
                edited(text) => { root.search(text); }
            }
            list := ListView {
                for row in root.rows: HorizontalLayout {
                    spacing: 8px;
                    padding-bottom: 8px;
                    alignment: start;
                    for card in row.cards: Card { data: card; }
                }
            }
            Text { text: root.status; color: #7d869c; font-size: 11px; }
        }
    }
}

const COLUMNS: usize = 4;
const THUMB_MAX: u32 = 256;
/// Decoded stills kept in memory: a few screens' worth.
const CACHE_CAP: usize = 240;
/// How far from the row on screen a queued decode may be and still run.
const NEAR_ROWS: usize = 8;

struct Job {
    id: i64,
    path: String,
    animated: bool,
    row: usize,
}

/// A decoded picture coming back from the worker thread.
enum Decoded {
    Still(i64, spike_common::Rgba),
    Animated(i64, Vec<Frame>),
}

fn to_image(rgba: &spike_common::Rgba) -> Image {
    let buf = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&rgba.pixels, rgba.width, rgba.height);
    Image::from_rgba8(buf)
}

struct Animation {
    frames: Vec<(Image, Duration)>,
    current: usize,
    due: Instant,
}

/// Rows of cards over a filtered view of the items. `row_data` is only called
/// for rows the ListView has instantiated, which is what makes decoding lazy:
/// a card asks for its picture the first time it is on screen.
struct Rows {
    items: Rc<Vec<Item>>,
    /// Indices into `items` that match the current search.
    visible: RefCell<Vec<usize>>,
    stills: RefCell<HashMap<i64, Image>>,
    animations: RefCell<HashMap<i64, Animation>>,
    /// Ids asked of the worker and not yet answered or evicted. Shared with
    /// the worker, which forgets the ones it skips so they are asked again.
    requested: Arc<Mutex<HashSet<i64>>>,
    /// Decoded stills, oldest first, for eviction past `CACHE_CAP`.
    order: RefCell<VecDeque<i64>>,
    /// The row most recently instantiated: where the user is looking now.
    focus: Arc<AtomicUsize>,
    jobs: mpsc::Sender<Job>,
    animate: bool,
    notify: ModelNotify,
}

impl Rows {
    fn card(&self, item: &Item, row: usize) -> CardData {
        let mut image = None;
        if self.animate {
            if let Some(a) = self.animations.borrow().get(&item.id) {
                image = Some(a.frames[a.current].0.clone());
            }
        }
        if image.is_none() {
            image = self.stills.borrow().get(&item.id).cloned();
        }
        if image.is_none() {
            let wanted = item
                .animated
                .clone()
                .filter(|_| self.animate)
                .map(|p| (p, true))
                .or_else(|| item.thumb.clone().map(|p| (p, false)));
            if let Some((path, animated)) = wanted {
                if self.requested.lock().unwrap().insert(item.id) {
                    let _ = self.jobs.send(Job { id: item.id, path, animated, row });
                }
            }
        }
        CardData {
            label: spike_common::label(item).into(),
            has_image: image.is_some(),
            image: image.unwrap_or_default(),
            kind: if item.ext.is_empty() { item.kind.clone() } else { item.ext.clone() }.into(),
        }
    }

    fn apply_search(&self, query: &str) {
        let q = query.to_lowercase();
        let v: Vec<usize> = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, it)| q.is_empty() || spike_common::label(it).to_lowercase().contains(&q))
            .map(|(i, _)| i)
            .collect();
        *self.visible.borrow_mut() = v;
        self.notify.reset();
    }

    fn rows_holding(&self, id: i64) -> Vec<usize> {
        self.visible
            .borrow()
            .iter()
            .enumerate()
            .filter(|(_, &i)| self.items[i].id == id)
            .map(|(pos, _)| pos / COLUMNS)
            .collect()
    }
}

impl Model for Rows {
    type Data = RowData;

    fn row_count(&self) -> usize {
        self.visible.borrow().len().div_ceil(COLUMNS)
    }

    fn row_data(&self, row: usize) -> Option<RowData> {
        let visible = self.visible.borrow();
        let start = row * COLUMNS;
        if start >= visible.len() {
            return None;
        }
        let end = (start + COLUMNS).min(visible.len());
        self.focus.store(row, Ordering::Relaxed);
        let cards: Vec<CardData> = visible[start..end].iter().map(|&i| self.card(&self.items[i], row)).collect();
        Some(RowData { cards: ModelRc::new(VecModel::from(cards)) })
    }

    fn model_tracker(&self) -> &dyn ModelTracker {
        &self.notify
    }
}

thread_local! {
    static ROWS: RefCell<Option<Rc<Rows>>> = const { RefCell::new(None) };
}

fn main() {
    let mut args = std::env::args().skip(1);
    let data = PathBuf::from(args.next().expect("usage: slint-popup <items.json> [--no-gif] [--acrylic]"));
    let rest: Vec<String> = args.collect();
    let animate = !rest.iter().any(|a| a == "--no-gif");
    let acrylic = rest.iter().any(|a| a == "--acrylic");
    let scroll_test = rest.iter().any(|a| a == "--scroll-test");
    let query = rest.iter().position(|a| a == "--query").and_then(|i| rest.get(i + 1)).cloned();

    let started = Instant::now();
    let items = Rc::new(spike_common::load(&data, 10_000));
    let ui = MainWindow::new().expect("window");
    ui.set_transparent_bg(acrylic);

    // One decoding thread. Results come back through the event loop, which is
    // the only place Slint images may be created.
    let (job_tx, job_rx) = mpsc::channel::<Job>();
    let requested: Arc<Mutex<HashSet<i64>>> = Arc::default();
    let focus = Arc::new(AtomicUsize::new(0));
    let weak = ui.as_weak();
    let (worker_requested, worker_focus) = (requested.clone(), focus.clone());
    std::thread::spawn(move || {
        // Newest first, and only what is still near the screen: a fast scroll
        // queues every row it passes, and decoding those would be work for
        // pictures nobody will see.
        let mut queue: Vec<Job> = Vec::new();
        loop {
            if queue.is_empty() {
                match job_rx.recv() {
                    Ok(j) => queue.push(j),
                    Err(_) => return,
                }
            }
            queue.extend(job_rx.try_iter());
            let Some(Job { id, path, animated, row }) = queue.pop() else { continue };
            if row.abs_diff(worker_focus.load(Ordering::Relaxed)) > NEAR_ROWS {
                worker_requested.lock().unwrap().remove(&id);
                continue;
            }
            let decoded = if animated {
                spike_common::decode_gif(&path, THUMB_MAX).map(|f| Decoded::Animated(id, f))
            } else {
                spike_common::decode_thumb(&path, THUMB_MAX).map(|r| Decoded::Still(id, r))
            };
            let Some(decoded) = decoded else { continue };
            let _ = weak.upgrade_in_event_loop(move |_| {
                ROWS.with(|r| {
                    let Some(rows) = r.borrow().clone() else { return };
                    let id = match decoded {
                        Decoded::Still(id, rgba) => {
                            rows.stills.borrow_mut().insert(id, to_image(&rgba));
                            let mut order = rows.order.borrow_mut();
                            order.push_back(id);
                            while order.len() > CACHE_CAP {
                                if let Some(old) = order.pop_front() {
                                    rows.stills.borrow_mut().remove(&old);
                                    rows.requested.lock().unwrap().remove(&old);
                                }
                            }
                            id
                        }
                        Decoded::Animated(id, frames) => {
                            let frames = frames.iter().map(|f| (to_image(&f.image), f.delay)).collect::<Vec<_>>();
                            let due = Instant::now() + frames[0].1;
                            rows.animations.borrow_mut().insert(id, Animation { frames, current: 0, due });
                            id
                        }
                    };
                    for row in rows.rows_holding(id) {
                        rows.notify.row_changed(row);
                    }
                });
            });
        }
    });

    let rows = Rc::new(Rows {
        items: items.clone(),
        visible: RefCell::new((0..items.len()).collect()),
        stills: RefCell::new(HashMap::new()),
        animations: RefCell::new(HashMap::new()),
        requested,
        order: RefCell::new(VecDeque::new()),
        focus,
        jobs: job_tx,
        animate,
        notify: ModelNotify::default(),
    });
    ROWS.with(|r| *r.borrow_mut() = Some(rows.clone()));
    ui.set_rows(ModelRc::from(rows.clone()));
    ui.set_status(format!("{} cards · built in {:?}", items.len(), started.elapsed()).into());

    let search_rows = rows.clone();
    let weak = ui.as_weak();
    ui.on_search(move |q| {
        let t = Instant::now();
        search_rows.apply_search(q.as_str());
        if let Some(ui) = weak.upgrade() {
            ui.set_status(format!("{} matches in {:?}", search_rows.visible.borrow().len(), t.elapsed()).into());
        }
    });

    // GIF frames: only items that have been decoded, i.e. that were on screen.
    let anim_rows = rows.clone();
    let timer = slint::Timer::default();
    if animate {
        timer.start(slint::TimerMode::Repeated, Duration::from_millis(20), move || {
            let now = Instant::now();
            let mut changed = Vec::new();
            for (id, a) in anim_rows.animations.borrow_mut().iter_mut() {
                if now >= a.due {
                    a.current = (a.current + 1) % a.frames.len();
                    a.due = now + a.frames[a.current].1;
                    changed.push(*id);
                }
            }
            for id in changed {
                for row in anim_rows.rows_holding(id) {
                    anim_rows.notify.row_changed(row);
                }
            }
        });
    }

    if acrylic {
        let weak = ui.as_weak();
        slint::Timer::single_shot(Duration::from_millis(200), move || {
            if let Some(ui) = weak.upgrade() {
                let handle = ui.window().window_handle();
                match window_vibrancy::apply_acrylic(&handle, Some((18, 20, 24, 200))) {
                    Ok(()) => eprintln!("acrylic applied"),
                    Err(e) => eprintln!("acrylic failed: {e}"),
                }
            }
        });
    }

    if let Some(q) = query {
        let t = Instant::now();
        rows.apply_search(&q);
        ui.set_status(format!("'{q}': {} matches in {:?}", rows.visible.borrow().len(), t.elapsed()).into());
    }

    // Scroll 2,000 rows (8,000 cards) down at one row per 16 ms tick: far
    // faster than a person scrolls, and every row on the way is instantiated.
    let scroll_timer = slint::Timer::default();
    if scroll_test {
        let weak = ui.as_weak();
        let t0 = RefCell::new(None::<Instant>);
        let done = std::cell::Cell::new(false);
        scroll_timer.start(slint::TimerMode::Repeated, Duration::from_millis(16), move || {
            let Some(ui) = weak.upgrade() else { return };
            if done.get() {
                return;
            }
            let start = *t0.borrow_mut().get_or_insert_with(Instant::now);
            let y = ui.get_scroll_y() - 158.0 * 4.0;
            ui.set_scroll_y(y);
            if y <= -158.0 * 2000.0 {
                ui.set_status(format!("scrolled 2000 rows in {:?}", start.elapsed()).into());
                done.set(true);
            }
        });
    }

    eprintln!("ready in {:?}", started.elapsed());
    ui.run().expect("event loop");
}
