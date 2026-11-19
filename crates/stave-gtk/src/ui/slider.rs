//! A slider drawn the design's way: a 4 px track filled in the accent
//! colour up to the value, a white handle with a ring, and optionally a
//! blocked range hatched in red from where a door first hits
//! (Problems.dc.html). It is used for "together … apart"
//! (Exploded.dc.html, a small 12 px handle with a thin ring), focus mode's
//! scrub and a problem's swing (22 px handles).
//!
//! Only what people do calls the change callback; [`Slider::set_value`]
//! never does, so the window can move it without hearing back.
//!
//! It mirrors for right-to-left text (drawing, pointer and arrow keys),
//! moves with the scroll wheel, and tells a screen reader its value in
//! words and where it is blocked.

use stave_app::i18n::{fill_in, tr};
use std::cell::RefCell;
use std::rc::{Rc, Weak};

use adw::prelude::*;
use gtk::{gdk, glib};

/// Called with the new value when people move the slider.
type ChangeCallback = Rc<dyn Fn(f64)>;

/// The design's red (#e01b24) and its pale stripe (#f6b8bb).
const BLOCKED: (f64, f64, f64) = (0.878, 0.106, 0.141);
const BLOCKED_PALE: (f64, f64, f64) = (0.965, 0.722, 0.733);

/// The least height, in pixels, so it is easy to hit with a finger or an
/// unsteady hand.
const MINIMUM_HEIGHT: i32 = 24;

/// How far one notch of the scroll wheel moves it, as a part of the range.
const SCROLL_STEP: f64 = 0.05;

struct State {
    lower: f64,
    upper: f64,
    value: f64,
    /// Where the blocked range starts, in the slider's units.
    blocked_from: Option<f64>,
    /// The value in words for screen readers ("45°"), if given.
    value_text: Option<String>,
    /// The handle's diameter and its ring's width, in pixels.
    handle: f64,
    ring: f64,
    on_change: Option<ChangeCallback>,
}

impl State {
    fn fraction(&self, value: f64) -> f64 {
        let span = self.upper - self.lower;
        if span <= 0.0 {
            0.0
        } else {
            ((value - self.lower) / span).clamp(0.0, 1.0)
        }
    }

    /// `true` when the value is in the blocked range.
    fn past_blocked(&self) -> bool {
        self.blocked_from.is_some_and(|from| self.value >= from)
    }

    /// The value in words for screen readers, saying when it is blocked:
    /// "2.6°, blocked".
    fn spoken_value(&self) -> Option<String> {
        let text = self.value_text.as_deref()?;
        Some(if self.past_blocked() {
            fill_in(&tr("{value}, blocked"), &[("value", &text)])
        } else {
            text.to_owned()
        })
    }
}

/// Which way an arrow key or scroll moves the value: `1.0` up, `-1.0`
/// down. Left and Right swap when the text runs right to left; Up and
/// Down don't.
fn key_direction(key: gdk::Key, right_to_left: bool) -> Option<f64> {
    let towards = |right: bool| if right != right_to_left { 1.0 } else { -1.0 };
    match key {
        gdk::Key::Left => Some(towards(false)),
        gdk::Key::Right => Some(towards(true)),
        gdk::Key::Up => Some(1.0),
        gdk::Key::Down => Some(-1.0),
        _ => None,
    }
}

/// A slider.
#[derive(Clone)]
pub struct Slider {
    /// The widget.
    pub root: gtk::DrawingArea,
    state: Rc<RefCell<State>>,
}

/// A slider held weakly, for its own widget's handlers: the widget keeps
/// its controllers, so a strong hold there would keep it forever.
struct WeakSlider {
    root: glib::WeakRef<gtk::DrawingArea>,
    state: Weak<RefCell<State>>,
}

impl WeakSlider {
    /// The slider, while it is still there.
    fn upgrade(&self) -> Option<Slider> {
        Some(Slider {
            root: self.root.upgrade()?,
            state: self.state.upgrade()?,
        })
    }
}

impl Slider {
    fn downgrade(&self) -> WeakSlider {
        WeakSlider {
            root: self.root.downgrade(),
            state: Rc::downgrade(&self.state),
        }
    }

    /// A slider from 0 to 1, `height` pixels tall (at least 24), with a
    /// handle `handle` pixels across and a ring `ring` pixels wide.
    pub fn new(label: &str, height: i32, handle: f64, ring: f64) -> Slider {
        let root = gtk::DrawingArea::builder()
            .content_height(height.max(MINIMUM_HEIGHT))
            .hexpand(true)
            .focusable(true)
            .css_classes(["stave-slider"])
            .accessible_role(gtk::AccessibleRole::Slider)
            .build();
        root.update_property(&[gtk::accessible::Property::Label(label)]);
        let state = Rc::new(RefCell::new(State {
            lower: 0.0,
            upper: 1.0,
            value: 0.0,
            blocked_from: None,
            value_text: None,
            handle,
            ring,
            on_change: None,
        }));
        let slider = Slider { root, state };
        slider.connect();
        slider.announce();
        slider
    }

    fn connect(&self) {
        let state = self.state.clone();
        self.root
            .set_draw_func(move |area, context, width, height| {
                draw(&state.borrow(), area, context, width, height);
            });

        // Its focus ring comes and goes with the keyboard focus.
        self.root.connect_has_focus_notify(|area| area.queue_draw());
        // Right to left draws it the other way round.
        self.root
            .connect_direction_changed(|area, _| area.queue_draw());

        // Press or drag anywhere on it to set the value.
        let drag = gtk::GestureDrag::new();
        let this = self.downgrade();
        drag.connect_drag_begin(move |_, x, _| {
            let Some(this) = this.upgrade() else {
                return;
            };
            this.root.grab_focus();
            this.set_from_x(x);
        });
        let this = self.downgrade();
        drag.connect_drag_update(move |gesture, dx, _| {
            let Some(this) = this.upgrade() else {
                return;
            };
            if let Some((x, _)) = gesture.start_point() {
                this.set_from_x(x + dx);
            }
        });
        self.root.add_controller(drag);

        // Arrow keys move it by a hundredth, Page keys by a tenth.
        let keys = gtk::EventControllerKey::new();
        let this = self.downgrade();
        keys.connect_key_pressed(move |_, key, _, _| {
            let Some(this) = this.upgrade() else {
                return glib::Propagation::Proceed;
            };
            let (lower, upper, value) = {
                let state = this.state.borrow();
                (state.lower, state.upper, state.value)
            };
            let step = (upper - lower) / 100.0;
            let next = if let Some(direction) = key_direction(key, this.right_to_left()) {
                value + direction * step
            } else {
                match key {
                    gdk::Key::Page_Down => value - step * 10.0,
                    gdk::Key::Page_Up => value + step * 10.0,
                    gdk::Key::Home => lower,
                    gdk::Key::End => upper,
                    _ => return glib::Propagation::Proceed,
                }
            };
            this.change(next);
            glib::Propagation::Stop
        });
        self.root.add_controller(keys);

        // The wheel moves it a twentieth a notch: up or right (left, right
        // to left) for more.
        let scroll = gtk::EventControllerScroll::new(
            gtk::EventControllerScrollFlags::BOTH_AXES | gtk::EventControllerScrollFlags::DISCRETE,
        );
        let this = self.downgrade();
        scroll.connect_scroll(move |_, dx, dy| {
            let Some(this) = this.upgrade() else {
                return glib::Propagation::Proceed;
            };
            this.scroll(dx, dy);
            glib::Propagation::Stop
        });
        self.root.add_controller(scroll);
    }

    /// `true` when the text runs right to left, so the slider's low end is
    /// on the right.
    fn right_to_left(&self) -> bool {
        self.root.direction() == gtk::TextDirection::Rtl
    }

    /// Moves it by a scroll of `dx`, `dy` notches.
    fn scroll(&self, dx: f64, dy: f64) {
        let across = if self.right_to_left() { -dx } else { dx };
        let notches = across - dy;
        if notches == 0.0 {
            return;
        }
        let (lower, upper, value) = {
            let state = self.state.borrow();
            (state.lower, state.upper, state.value)
        };
        self.change(value + notches * SCROLL_STEP * (upper - lower));
    }

    fn set_from_x(&self, x: f64) {
        let width = f64::from(self.root.width());
        let (lower, upper, handle) = {
            let state = self.state.borrow();
            (state.lower, state.upper, state.handle)
        };
        let inset = handle / 2.0;
        let usable = (width - handle).max(1.0);
        let x = if self.right_to_left() { width - x } else { x };
        let fraction = ((x - inset) / usable).clamp(0.0, 1.0);
        self.change(lower + fraction * (upper - lower));
    }

    /// Sets the value as people did, and tells the callback.
    fn change(&self, value: f64) {
        let callback = {
            let mut state = self.state.borrow_mut();
            let value = value.clamp(state.lower, state.upper);
            if (value - state.value).abs() < f64::EPSILON {
                return;
            }
            state.value = value;
            state.on_change.clone()
        };
        self.root.queue_draw();
        self.announce();
        if let Some(callback) = callback {
            callback(self.value());
        }
    }

    fn announce(&self) {
        let state = self.state.borrow();
        self.root.update_property(&[
            gtk::accessible::Property::ValueMin(state.lower),
            gtk::accessible::Property::ValueMax(state.upper),
            gtk::accessible::Property::ValueNow(state.value),
        ]);
        if let Some(text) = state.spoken_value() {
            self.root
                .update_property(&[gtk::accessible::Property::ValueText(&text)]);
        }
    }

    /// Calls `callback` with the value whenever people move it.
    pub fn connect_changed(&self, callback: impl Fn(f64) + 'static) {
        self.state.borrow_mut().on_change = Some(Rc::new(callback));
    }

    /// The value.
    pub fn value(&self) -> f64 {
        self.state.borrow().value
    }

    /// Moves it, without calling back.
    pub fn set_value(&self, value: f64) {
        {
            let mut state = self.state.borrow_mut();
            state.value = value.clamp(state.lower, state.upper);
        }
        self.root.queue_draw();
        self.announce();
    }

    /// Sets its range, without calling back.
    pub fn set_range(&self, lower: f64, upper: f64) {
        {
            let mut state = self.state.borrow_mut();
            state.lower = lower;
            state.upper = upper.max(lower);
            state.value = state.value.clamp(state.lower, state.upper);
        }
        self.root.queue_draw();
        self.announce();
    }

    /// Hatches the track red from `from` to the end, and gives `words`
    /// ("blocked from 0.6°") as its description for screen readers
    /// (`None`: nothing blocked).
    pub fn set_blocked(&self, blocked: Option<(f64, &str)>) {
        self.state.borrow_mut().blocked_from = blocked.map(|(from, _)| from);
        match blocked {
            Some((_, words)) => self
                .root
                .update_property(&[gtk::accessible::Property::Description(words)]),
            None => self
                .root
                .reset_property(gtk::AccessibleProperty::Description),
        }
        self.root.queue_draw();
        self.announce();
    }

    /// Says the value in words for screen readers ("45°"; "45°, blocked"
    /// once it is in the blocked range).
    pub fn set_value_text(&self, text: &str) {
        self.state.borrow_mut().value_text = Some(text.to_owned());
        self.announce();
    }
}

fn rounded_bar(context: &gtk::cairo::Context, left: f64, right: f64, middle: f64, thickness: f64) {
    let radius = thickness / 2.0;
    if right - left < thickness {
        return;
    }
    context.new_sub_path();
    context.arc(
        right - radius,
        middle,
        radius,
        -std::f64::consts::FRAC_PI_2,
        std::f64::consts::FRAC_PI_2,
    );
    context.arc(
        left + radius,
        middle,
        radius,
        std::f64::consts::FRAC_PI_2,
        3.0 * std::f64::consts::FRAC_PI_2,
    );
    context.close_path();
}

fn draw(
    state: &State,
    area: &gtk::DrawingArea,
    context: &gtk::cairo::Context,
    width: i32,
    height: i32,
) {
    let (width, height) = (f64::from(width), f64::from(height));
    // The accent comes in as the widget's colour (`color: @accent_color`).
    let accent = area.color();
    let (accent_red, accent_green, accent_blue) = (
        f64::from(accent.red()),
        f64::from(accent.green()),
        f64::from(accent.blue()),
    );
    let style = adw::StyleManager::default();
    let (dark, high_contrast) = (style.is_dark(), style.is_high_contrast());
    // Right to left: the same picture, mirrored.
    if area.direction() == gtk::TextDirection::Rtl {
        context.translate(width, 0.0);
        context.scale(-1.0, 1.0);
    }
    let inset = state.handle / 2.0;
    let left = inset;
    let right = (width - inset).max(left);
    let middle = height / 2.0;
    let thickness = 4.0;
    let at = |fraction: f64| left + fraction * (right - left);
    let value_x = at(state.fraction(state.value));
    let blocked_x = state.blocked_from.map(|from| at(state.fraction(from)));

    // The empty track, stronger in high contrast.
    rounded_bar(context, left - 2.0, right + 2.0, middle, thickness);
    match (dark, high_contrast) {
        (true, false) => context.set_source_rgba(1.0, 1.0, 1.0, 0.18),
        (false, false) => context.set_source_rgba(0.0, 0.0, 0.08, 0.13),
        (true, true) => context.set_source_rgba(1.0, 1.0, 1.0, 0.5),
        (false, true) => context.set_source_rgba(0.0, 0.0, 0.08, 0.5),
    }
    let _ = context.fill();
    // Filled up to the value, or to where it is blocked.
    let fill_to = blocked_x.map_or(value_x, |blocked| value_x.min(blocked));
    rounded_bar(
        context,
        left - 2.0,
        fill_to.max(left + 2.0),
        middle,
        thickness,
    );
    context.set_source_rgb(accent_red, accent_green, accent_blue);
    let _ = context.fill();
    // The blocked range: red and pale red stripes at 45°.
    if let Some(blocked) = blocked_x {
        let _ = context.save();
        rounded_bar(context, blocked, right + 2.0, middle, thickness);
        context.clip();
        context.set_source_rgb(BLOCKED_PALE.0, BLOCKED_PALE.1, BLOCKED_PALE.2);
        let _ = context.paint();
        context.set_source_rgb(BLOCKED.0, BLOCKED.1, BLOCKED.2);
        let mut x = blocked - height;
        while x < right + height {
            context.move_to(x, middle + thickness);
            context.line_to(x + 2.0 * thickness, middle - thickness);
            context.line_to(x + 2.0 * thickness + 2.8, middle - thickness);
            context.line_to(x + 2.8, middle + thickness);
            context.close_path();
            x += 5.6;
        }
        let _ = context.fill();
        let _ = context.restore();
    }
    // The handle: white with a ring (red past the blocked point) and a
    // soft shadow.
    let radius = state.handle / 2.0;
    let past = blocked_x.is_some_and(|blocked| value_x >= blocked - 0.5);
    context.arc(
        value_x,
        middle + 1.0,
        radius + state.ring / 2.0,
        0.0,
        std::f64::consts::TAU,
    );
    context.set_source_rgba(0.0, 0.0, 0.0, 0.18);
    let _ = context.fill();
    context.arc(value_x, middle, radius, 0.0, std::f64::consts::TAU);
    context.set_source_rgb(1.0, 1.0, 1.0);
    let _ = context.fill_preserve();
    if high_contrast {
        // A dark edge outside the ring, so the white handle stands out
        // on a light background too.
        context.set_source_rgb(0.0, 0.0, 0.0);
        context.set_line_width(state.ring + 2.0);
        let _ = context.stroke_preserve();
    }
    if past {
        context.set_source_rgb(BLOCKED.0, BLOCKED.1, BLOCKED.2);
    } else {
        context.set_source_rgb(accent_red, accent_green, accent_blue);
    }
    context.set_line_width(state.ring);
    let _ = context.stroke();
    if area.has_focus() && area.has_visible_focus() {
        context.arc(
            value_x,
            middle,
            radius + state.ring + 2.0,
            0.0,
            std::f64::consts::TAU,
        );
        context.set_source_rgba(accent_red, accent_green, accent_blue, 0.5);
        context.set_line_width(2.0);
        let _ = context.stroke();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn left_and_right_swap_right_to_left() {
        assert_eq!(key_direction(gdk::Key::Right, false), Some(1.0));
        assert_eq!(key_direction(gdk::Key::Left, false), Some(-1.0));
        assert_eq!(key_direction(gdk::Key::Right, true), Some(-1.0));
        assert_eq!(key_direction(gdk::Key::Left, true), Some(1.0));
        assert_eq!(key_direction(gdk::Key::Up, true), Some(1.0));
        assert_eq!(key_direction(gdk::Key::Down, true), Some(-1.0));
        assert_eq!(key_direction(gdk::Key::Home, false), None);
    }

    #[test]
    fn it_is_heard_mirrored_scrolled_and_blocked() {
        crate::ui::gtk_test::on_gtk_thread(heard_mirrored_scrolled_and_blocked);
    }

    /// The pointer and the wheel follow the text's direction, it is never
    /// shorter than 24 px, and a screen reader hears where it is blocked
    /// and when the value is past that.
    fn heard_mirrored_scrolled_and_blocked() {
        let slider = Slider::new("Swing it", 18, 22.0, 2.0);
        assert_eq!(slider.root.content_height(), MINIMUM_HEIGHT);
        let window = gtk::Window::builder()
            .default_width(222)
            .child(&slider.root)
            .build();
        window.present();
        let context = glib::MainContext::default();
        for _ in 0..20 {
            context.iteration(false);
        }
        let width = f64::from(slider.root.width());
        assert!(width > 22.0, "the slider has room: {width}");
        // Near the left end: low left to right, high right to left.
        slider.set_from_x(11.0);
        assert!(slider.value() < 0.01, "{}", slider.value());
        slider.root.set_direction(gtk::TextDirection::Rtl);
        slider.set_from_x(11.0);
        assert!(slider.value() > 0.99, "{}", slider.value());
        // Right to left, the wheel's right is lower; up is always higher.
        slider.set_value(0.5);
        slider.scroll(1.0, 0.0);
        assert!((slider.value() - 0.45).abs() < 1e-9, "{}", slider.value());
        slider.scroll(0.0, -2.0);
        assert!((slider.value() - 0.55).abs() < 1e-9, "{}", slider.value());

        let description = gtk::AccessibleProperty::Description;
        assert!(!gtk::test_accessible_has_property(
            &slider.root,
            description
        ));
        slider.set_blocked(Some((0.6, "blocked from 0.6°")));
        assert!(gtk::test_accessible_has_property(&slider.root, description));
        slider.set_value_text("0.5°");
        assert_eq!(
            slider.state.borrow().spoken_value().as_deref(),
            Some("0.5°")
        );
        slider.set_value(0.7);
        slider.set_value_text("0.7°");
        assert_eq!(
            slider.state.borrow().spoken_value().as_deref(),
            Some("0.7°, blocked")
        );
        slider.set_blocked(None);
        assert!(!gtk::test_accessible_has_property(
            &slider.root,
            description
        ));
        assert_eq!(
            slider.state.borrow().spoken_value().as_deref(),
            Some("0.7°")
        );
        window.destroy();
    }

    #[test]
    fn a_dropped_slider_is_freed() {
        crate::ui::gtk_test::on_gtk_thread(slider_is_freed);
    }

    /// Its own widget's handlers hold it weakly, so letting go of the
    /// slider frees the widget, its state and the callback.
    fn slider_is_freed() {
        let slider = Slider::new("Together … apart", 24, 12.0, 1.0);
        slider.connect_changed(|_| {});
        let root = slider.root.downgrade();
        let state = Rc::downgrade(&slider.state);
        drop(slider);
        assert!(root.upgrade().is_none(), "the widget is freed");
        assert!(state.upgrade().is_none(), "the state is freed");
    }
}
