//! Middle-click autoscroll for the popup's grid, as browsers have it.
//!
//! Pressing the wheel drops an anchor mark; the further the pointer moves
//! above or below it, the faster the grid scrolls that way. A press-and-drag
//! ends when the wheel is released; a plain click stays on until the next
//! click, Esc, a wheel turn or the window losing focus.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use slint::winit_030::winit::event::{ElementState, MouseButton, WindowEvent};
use slint::winit_030::winit::keyboard::{Key, NamedKey};
use slint::winit_030::winit::window::CursorIcon;
use slint::winit_030::{EventResult, WinitWindowAccessor};
use slint::ComponentHandle;

use crate::ui::PopupWindow;

/// Around the anchor, in logical pixels: no scrolling inside it.
const DEAD_ZONE: f32 = 12.0;
/// Scroll speed per pixel past the dead zone, logical pixels per second.
const SPEED_PER_PX: f32 = 9.0;
const MAX_SPEED: f32 = 9000.0;
/// Moved this far while the wheel is held: the release ends the scroll.
const DRAG_DISTANCE: f32 = 6.0;
const TICK: Duration = Duration::from_millis(16);

#[derive(Clone, Copy)]
struct Session {
    anchor: (f32, f32),
    /// The wheel is still down from the press that started it.
    held: bool,
    /// The pointer left the press point while the wheel was held.
    dragged: bool,
    last_tick: Instant,
}

pub struct Autoscroll {
    ui: slint::Weak<PopupWindow>,
    /// Logical cursor position inside the window.
    cursor: Rc<Cell<(f32, f32)>>,
    session: Rc<RefCell<Option<Session>>>,
    timer: Rc<slint::Timer>,
}

impl Autoscroll {
    pub fn new(ui: &PopupWindow) -> Self {
        Self {
            ui: ui.as_weak(),
            cursor: Rc::new(Cell::new((0.0, 0.0))),
            session: Rc::new(RefCell::new(None)),
            timer: Rc::new(slint::Timer::default()),
        }
    }

    pub fn active(&self) -> bool {
        self.session.borrow().is_some()
    }

    /// Takes the window events autoscroll owns; `None` leaves the event to
    /// the rest of the popup.
    pub fn handle(&self, event: &WindowEvent, scale: f32) -> Option<EventResult> {
        match event {
            WindowEvent::CursorMoved { position, .. } => {
                let p = (position.x as f32 / scale, position.y as f32 / scale);
                self.cursor.set(p);
                let mut session = self.session.borrow_mut();
                let s = session.as_mut()?;
                if s.held && distance(p, s.anchor) > DRAG_DISTANCE {
                    s.dragged = true;
                }
                let icon = direction_icon(p.1 - s.anchor.1);
                drop(session);
                self.set_cursor(icon);
                // No hover effects under a scrolling pointer.
                Some(EventResult::PreventDefault)
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let pressed = *state == ElementState::Pressed;
                if !self.active() {
                    if pressed && *button == MouseButton::Middle && self.start() {
                        return Some(EventResult::PreventDefault);
                    }
                    return None;
                }
                if !pressed {
                    let end = {
                        let mut session = self.session.borrow_mut();
                        let s = session.as_mut()?;
                        let was_held = s.held && *button == MouseButton::Middle;
                        if was_held {
                            s.held = false;
                        }
                        was_held && s.dragged
                    };
                    if end {
                        self.stop();
                    }
                    return Some(EventResult::PreventDefault);
                }
                // Any press while scrolling only ends it, as in a browser.
                self.stop();
                Some(EventResult::PreventDefault)
            }
            WindowEvent::MouseWheel { .. } if self.active() => {
                self.stop();
                None
            }
            WindowEvent::KeyboardInput { event, .. } if self.active() => {
                if event.state == ElementState::Pressed
                    && event.logical_key == Key::Named(NamedKey::Escape)
                {
                    self.stop();
                    return Some(EventResult::PreventDefault);
                }
                None
            }
            // The popup hides on focus loss; nothing should keep scrolling.
            WindowEvent::Focused(false) if self.active() => {
                self.stop();
                None
            }
            _ => None,
        }
    }

    /// Starts at the cursor if there is anything to scroll.
    fn start(&self) -> bool {
        let Some(ui) = self.ui.upgrade() else {
            return false;
        };
        if ui.get_list_content_height() <= ui.get_list_height() + 1.0 {
            return false;
        }
        let anchor = self.cursor.get();
        *self.session.borrow_mut() = Some(Session {
            anchor,
            held: true,
            dragged: false,
            last_tick: Instant::now(),
        });
        ui.set_autoscroll_x(anchor.0);
        ui.set_autoscroll_y(anchor.1);
        ui.set_autoscroll(true);
        self.set_cursor(CursorIcon::AllScroll);

        let session = self.session.clone();
        let cursor = self.cursor.clone();
        let weak = self.ui.clone();
        self.timer.start(slint::TimerMode::Repeated, TICK, move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let mut session = session.borrow_mut();
            let Some(s) = session.as_mut() else {
                return;
            };
            let now = Instant::now();
            let dt = now.duration_since(s.last_tick).as_secs_f32().min(0.1);
            s.last_tick = now;
            let speed = speed(cursor.get().1 - s.anchor.1);
            if speed == 0.0 {
                return;
            }
            let max = (ui.get_list_content_height() - ui.get_list_height()).max(0.0);
            let y = (ui.get_scroll_y() - speed * dt).clamp(-max, 0.0);
            ui.set_scroll_y(y);
        });
        true
    }

    pub fn stop(&self) {
        if self.session.borrow_mut().take().is_none() {
            return;
        }
        self.timer.stop();
        if let Some(ui) = self.ui.upgrade() {
            ui.set_autoscroll(false);
        }
        self.set_cursor(CursorIcon::Default);
    }

    fn set_cursor(&self, icon: CursorIcon) {
        if let Some(ui) = self.ui.upgrade() {
            ui.window().with_winit_window(|w| w.set_cursor(icon));
        }
    }
}

/// Scroll speed for a pointer `dy` logical pixels below the anchor (negative
/// above): positive scrolls down.
fn speed(dy: f32) -> f32 {
    let past = dy.abs() - DEAD_ZONE;
    if past <= 0.0 {
        return 0.0;
    }
    (past * SPEED_PER_PX).min(MAX_SPEED) * dy.signum()
}

fn direction_icon(dy: f32) -> CursorIcon {
    if dy.abs() <= DEAD_ZONE {
        CursorIcon::AllScroll
    } else if dy < 0.0 {
        CursorIcon::NResize
    } else {
        CursorIcon::SResize
    }
}

fn distance(a: (f32, f32), b: (f32, f32)) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_moves_inside_the_dead_zone() {
        assert_eq!(speed(0.0), 0.0);
        assert_eq!(speed(DEAD_ZONE), 0.0);
        assert_eq!(speed(-DEAD_ZONE), 0.0);
    }

    #[test]
    fn speed_grows_with_distance_and_follows_the_side() {
        assert!(speed(40.0) > speed(20.0));
        assert!(speed(20.0) > 0.0);
        assert!(speed(-20.0) < 0.0);
        assert_eq!(speed(100_000.0), MAX_SPEED);
    }
}
