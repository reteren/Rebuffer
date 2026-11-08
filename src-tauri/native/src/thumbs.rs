//! Card pictures: decoded off the UI thread, only for cards on screen, kept in
//! a bounded cache.
//!
//! The phase-1 spike showed what happens without the last two rules: a fast
//! scroll queued a decode for every row it passed and the cache grew to
//! 600 MB. Here a queued decode is skipped when the user has scrolled more
//! than `NEAR_ROWS` away from it, and stills are evicted oldest-first past
//! `CACHE_CAP`.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use slint::{Image, Rgba8Pixel, SharedPixelBuffer};

/// Long side of a decoded picture, in physical pixels. The largest tile is
/// 188x250 logical and a picture is cropped to fill it, so the short side has
/// to cover 188 or more; 320 does at 100-125 % scaling without visible blur.
const DECODE_MAX: u32 = 320;
/// Decoded stills kept in memory: a few screens' worth at the largest zoom.
const CACHE_CAP: usize = 160;
/// How far from the row on screen a queued decode may be and still run.
const NEAR_ROWS: usize = 8;

struct Job {
    id: i64,
    path: PathBuf,
    animated: bool,
    row: usize,
}

struct Rgba {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

enum Decoded {
    Still(Rgba),
    Animated(Vec<(Rgba, Duration)>),
}

struct Animation {
    frames: Vec<(Image, Duration)>,
    current: usize,
    due: Instant,
}

/// What the cache knows about one item's picture.
pub enum Picture {
    Ready(Image),
    /// Not decoded yet; a decode has been queued if there is anything to show.
    Pending,
}

struct Cache {
    stills: HashMap<i64, Image>,
    order: VecDeque<i64>,
    animations: HashMap<i64, Animation>,
}

thread_local! {
    static CACHE: RefCell<Cache> = RefCell::new(Cache {
        stills: HashMap::new(),
        order: VecDeque::new(),
        animations: HashMap::new(),
    });
}

/// The worker and the bookkeeping shared with it.
pub struct Thumbs {
    jobs: mpsc::Sender<Job>,
    requested: Arc<Mutex<HashSet<i64>>>,
    focus: Arc<AtomicUsize>,
}

impl Thumbs {
    /// Starts the decoder. `ready(id)` runs on the event loop each time a
    /// picture lands in the cache.
    pub fn start(ready: fn(i64)) -> Thumbs {
        let (jobs, rx) = mpsc::channel::<Job>();
        let requested: Arc<Mutex<HashSet<i64>>> = Arc::default();
        let focus = Arc::new(AtomicUsize::new(0));
        let (w_requested, w_focus) = (requested.clone(), focus.clone());
        std::thread::Builder::new()
            .name("native-thumbs".into())
            .spawn(move || decode_loop(rx, w_requested, w_focus, ready))
            .expect("spawning the thumbnail decoder");
        Thumbs {
            jobs,
            requested,
            focus,
        }
    }

    /// Records that `row` is on screen now.
    pub fn focus(&self, row: usize) {
        self.focus.store(row, Ordering::Relaxed);
    }

    /// The picture for `id`, queuing its decode on first sight. `animated`
    /// wins over `still` when both are given.
    pub fn picture(
        &self,
        id: i64,
        row: usize,
        still: Option<PathBuf>,
        animated: Option<PathBuf>,
    ) -> Picture {
        let found = CACHE.with(|c| {
            let c = c.borrow();
            if animated.is_some() {
                if let Some(a) = c.animations.get(&id) {
                    return Some(a.frames[a.current].0.clone());
                }
            }
            c.stills.get(&id).cloned()
        });
        if let Some(image) = found {
            return Picture::Ready(image);
        }
        let (path, is_animated) = match (animated, still) {
            (Some(p), _) => (p, true),
            (None, Some(p)) => (p, false),
            (None, None) => return Picture::Pending,
        };
        if self.requested.lock().expect("thumbs lock").insert(id) {
            let _ = self.jobs.send(Job {
                id,
                path,
                animated: is_animated,
                row,
            });
        }
        Picture::Pending
    }

    /// Advances every animation that is due. Returns the ids whose frame
    /// changed.
    pub fn tick(&self) -> Vec<i64> {
        let now = Instant::now();
        CACHE.with(|c| {
            let mut c = c.borrow_mut();
            let mut changed = Vec::new();
            for (id, a) in c.animations.iter_mut() {
                if now >= a.due {
                    a.current = (a.current + 1) % a.frames.len();
                    a.due = now + a.frames[a.current].1;
                    changed.push(*id);
                }
            }
            changed
        })
    }

    pub fn has_animations(&self) -> bool {
        CACHE.with(|c| !c.borrow().animations.is_empty())
    }

    /// Drops the animations: frames are the expensive part of the cache and
    /// are not worth keeping while the popup is hidden.
    pub fn release_animations(&self) {
        let ids: Vec<i64> = CACHE.with(|c| {
            c.borrow_mut()
                .animations
                .drain()
                .map(|(id, _)| id)
                .collect()
        });
        let mut requested = self.requested.lock().expect("thumbs lock");
        for id in ids {
            requested.remove(&id);
        }
    }

    fn store(&self, id: i64, decoded: Decoded) {
        CACHE.with(|c| {
            let mut c = c.borrow_mut();
            match decoded {
                Decoded::Still(rgba) => {
                    c.stills.insert(id, to_image(&rgba));
                    c.order.push_back(id);
                    while c.order.len() > CACHE_CAP {
                        if let Some(old) = c.order.pop_front() {
                            c.stills.remove(&old);
                            self.requested.lock().expect("thumbs lock").remove(&old);
                        }
                    }
                }
                Decoded::Animated(frames) => {
                    let frames: Vec<(Image, Duration)> =
                        frames.iter().map(|(f, d)| (to_image(f), *d)).collect();
                    if let Some(first) = frames.first() {
                        let due = Instant::now() + first.1;
                        c.animations.insert(
                            id,
                            Animation {
                                frames,
                                current: 0,
                                due,
                            },
                        );
                    }
                }
            }
        });
    }
}

thread_local! {
    static THUMBS: RefCell<Option<std::rc::Rc<Thumbs>>> = const { RefCell::new(None) };
}

/// Registers the instance results are delivered to.
pub fn install(thumbs: std::rc::Rc<Thumbs>) {
    THUMBS.with(|t| *t.borrow_mut() = Some(thumbs));
}

fn decode_loop(
    rx: mpsc::Receiver<Job>,
    requested: Arc<Mutex<HashSet<i64>>>,
    focus: Arc<AtomicUsize>,
    ready: fn(i64),
) {
    // Newest first, and only what is still near the screen.
    let mut queue: Vec<Job> = Vec::new();
    loop {
        if queue.is_empty() {
            match rx.recv() {
                Ok(j) => queue.push(j),
                Err(_) => return,
            }
        }
        queue.extend(rx.try_iter());
        let Some(job) = queue.pop() else { continue };
        if job.row.abs_diff(focus.load(Ordering::Relaxed)) > NEAR_ROWS {
            requested.lock().expect("thumbs lock").remove(&job.id);
            continue;
        }
        let decoded = if job.animated {
            decode_gif(&job.path).map(Decoded::Animated)
        } else {
            decode_still(&job.path).map(Decoded::Still)
        };
        let Some(decoded) = decoded else {
            // Undecodable: the card keeps its text face, and is not retried.
            continue;
        };
        let id = job.id;
        let _ = slint::invoke_from_event_loop(move || {
            THUMBS.with(|t| {
                if let Some(t) = t.borrow().as_ref() {
                    t.store(id, decoded);
                }
            });
            ready(id);
        });
    }
}

fn decode_still(path: &std::path::Path) -> Option<Rgba> {
    let img = image::open(path).ok()?;
    let img = if img.width() > DECODE_MAX || img.height() > DECODE_MAX {
        img.thumbnail(DECODE_MAX, DECODE_MAX)
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

fn decode_gif(path: &std::path::Path) -> Option<Vec<(Rgba, Duration)>> {
    use image::AnimationDecoder;
    let file = std::io::BufReader::new(std::fs::File::open(path).ok()?);
    let frames = image::codecs::gif::GifDecoder::new(file)
        .ok()?
        .into_frames()
        .collect_frames()
        .ok()?;
    let out: Vec<(Rgba, Duration)> = frames
        .into_iter()
        .map(|f| {
            let (n, d) = f.delay().numer_denom_ms();
            // Browsers clamp tiny delays the same way; 0 ms would spin.
            let delay = Duration::from_millis(u64::from((n / d.max(1)).max(20)));
            let img =
                image::DynamicImage::ImageRgba8(f.into_buffer()).thumbnail(DECODE_MAX, DECODE_MAX);
            let rgba = img.to_rgba8();
            (
                Rgba {
                    width: rgba.width(),
                    height: rgba.height(),
                    pixels: rgba.into_raw(),
                },
                delay,
            )
        })
        .collect();
    (!out.is_empty()).then_some(out)
}

fn to_image(rgba: &Rgba) -> Image {
    Image::from_rgba8(SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(
        &rgba.pixels,
        rgba.width,
        rgba.height,
    ))
}
