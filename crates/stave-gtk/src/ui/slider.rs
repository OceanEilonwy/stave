//! A slider drawn the design's way: a 4 px track filled in the accent
//! colour up to the value, a white handle with a ring, and optionally a
//! blocked range hatched in red from where a door first hits
//! (Problems.dc.html). It is used for "together … apart"
//! (Exploded.dc.html, a small 12 px handle with a thin ring), focus mode's
//! scrub and a problem's swing (22 px handles).
//!
//! It is a relm4 component, [`SliderComponent`], and the worked example in
//! docs/relm4-migration.md: its model is the only state, only
//! [`SliderComponent::update`] changes it, and its widget's handlers only
//! send [`SliderInput`] messages. [`Slider`] keeps the older callback API
//! in front of it until its callers move to relm4 too.
//!
//! Only what people do sends [`SliderOutput::Changed`] (and so calls the
//! change callback); [`Slider::set_value`] never does, so the window can
//! move it without hearing back.
//!
//! It mirrors for right-to-left text (drawing, pointer and arrow keys),
//! moves with the scroll wheel, and tells a screen reader its value in
//! words and where it is blocked.

use std::cell::Ref;

use adw::prelude::*;
use gtk::{gdk, glib};
use relm4::{Component, ComponentParts, ComponentSender};
use stave_app::i18n::{fill_in, tr};

use super::relm4_shim::Bridge;

/// The design's red (#e01b24) and its pale stripe (#f6b8bb).
const BLOCKED: (f64, f64, f64) = (0.878, 0.106, 0.141);
const BLOCKED_PALE: (f64, f64, f64) = (0.965, 0.722, 0.733);

/// The least height, in pixels, so it is easy to hit with a finger or an
/// unsteady hand.
const MINIMUM_HEIGHT: i32 = 24;

/// How far one notch of the scroll wheel moves it, as a part of the range.
const SCROLL_STEP: f64 = 0.05;

/// What the slider draws: its range, its value, where it is blocked and
/// how its handle is drawn. It is small and `Copy`, so each redraw takes
/// a copy of it rather than sharing it with the drawing area.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Track {
    lower: f64,
    upper: f64,
    value: f64,
    /// Where the blocked range starts, in the slider's units.
    blocked_from: Option<f64>,
    /// The handle's diameter and its ring's width, in pixels.
    handle: f64,
    ring: f64,
}

/// The slider's state: what it draws, and its value in words for screen
/// readers.
#[derive(Clone, Debug, PartialEq)]
pub struct SliderComponent {
    track: Track,
    /// The value in words for screen readers ("45°"), if given.
    value_text: Option<String>,
}

/// What a slider is made with.
#[derive(Debug)]
pub struct SliderInit {
    /// What screen readers call it ("Door angle").
    pub label: String,
    /// Its height in pixels.
    pub height: i32,
    /// The handle's diameter in pixels.
    pub handle: f64,
    /// The width of the handle's ring in pixels.
    pub ring: f64,
}

/// A key that moves the slider.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Down, or the arrow towards the low end (Left, or Right when the
    /// text runs right to left): a hundredth down.
    Down,
    /// Up, or the arrow towards the high end: a hundredth up.
    Up,
    /// Page Down: a tenth down.
    PageDown,
    /// Page Up: a tenth up.
    PageUp,
    /// Home: to the lower end.
    Home,
    /// End: to the upper end.
    End,
}

/// Which way an arrow key moves the value: `1.0` up, `-1.0` down. Left
/// and Right swap when the text runs right to left; Up and Down don't.
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

/// `true` when the text runs right to left in `widget`, so a slider's low
/// end is on the right.
fn right_to_left(widget: &impl IsA<gtk::Widget>) -> bool {
    widget.direction() == gtk::TextDirection::Rtl
}

impl Step {
    /// The step a key makes, if it makes one, in a slider whose text runs
    /// right to left or not.
    fn from_key(key: gdk::Key, right_to_left: bool) -> Option<Step> {
        if let Some(direction) = key_direction(key, right_to_left) {
            return Some(if direction > 0.0 {
                Step::Up
            } else {
                Step::Down
            });
        }
        match key {
            gdk::Key::Page_Down => Some(Step::PageDown),
            gdk::Key::Page_Up => Some(Step::PageUp),
            gdk::Key::Home => Some(Step::Home),
            gdk::Key::End => Some(Step::End),
            _ => None,
        }
    }
}

/// What a slider is told.
#[derive(Debug)]
pub enum SliderInput {
    /// People pressed or dragged to `x` pixels across a slider `width`
    /// pixels wide.
    PointedAt {
        /// Where, from the slider's left edge.
        x: f64,
        /// The slider's width when they did.
        width: f64,
        /// Whether its text ran right to left, so its low end was on the
        /// right.
        right_to_left: bool,
    },
    /// People pressed a key that moves it.
    Stepped(Step),
    /// People turned the scroll wheel `dx` notches across and `dy` down.
    Scrolled {
        /// Notches to the right.
        dx: f64,
        /// Notches down.
        dy: f64,
        /// Whether its text ran right to left: then right is lower.
        right_to_left: bool,
    },
    /// Move it, without telling anyone.
    SetValue(f64),
    /// Set its range, without telling anyone.
    SetRange {
        /// The lower end.
        lower: f64,
        /// The upper end (raised to `lower` if below it).
        upper: f64,
    },
    /// Hatch the track red from here to the end, describing it to screen
    /// readers in these words ("blocked from 0.6°") (`None`: nothing
    /// blocked).
    SetBlocked(Option<(f64, String)>),
    /// Say the value in words for screen readers ("45°"; "45°, blocked"
    /// once it is in the blocked range).
    SetValueText(String),
}

/// What a slider tells its owner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SliderOutput {
    /// People moved it to this value.
    Changed(f64),
}

impl Track {
    fn fraction(&self, value: f64) -> f64 {
        let span = self.upper - self.lower;
        if span <= 0.0 {
            0.0
        } else {
            ((value - self.lower) / span).clamp(0.0, 1.0)
        }
    }

    /// Sets the value as people did: `Some(value)` when it moved.
    fn change(&mut self, value: f64) -> Option<f64> {
        let value = value.clamp(self.lower, self.upper);
        if (value - self.value).abs() < f64::EPSILON {
            return None;
        }
        self.value = value;
        Some(value)
    }

    /// `true` when the value is in the blocked range.
    fn past_blocked(&self) -> bool {
        self.blocked_from.is_some_and(|from| self.value >= from)
    }
}

impl SliderComponent {
    /// Draws a copy of the slider as it is now; the drawing area keeps the
    /// copy until the next update hands it a new one.
    fn painter(&self) -> impl Fn(&gtk::DrawingArea, &gtk::cairo::Context, i32, i32) + 'static {
        let look = self.track;
        move |area, context, width, height| draw(&look, area, context, width, height)
    }

    /// The value in words for screen readers, saying when it is blocked:
    /// "2.6°, blocked".
    fn spoken_value(&self) -> Option<String> {
        let text = self.value_text.as_deref()?;
        Some(if self.track.past_blocked() {
            fill_in(&tr("{value}, blocked"), &[("value", &text)])
        } else {
            text.to_owned()
        })
    }
}

#[relm4::component(pub)]
impl Component for SliderComponent {
    type Init = SliderInit;
    type Input = SliderInput;
    type Output = SliderOutput;
    type CommandOutput = ();

    view! {
        gtk::DrawingArea::builder()
            .accessible_role(gtk::AccessibleRole::Slider)
            .build() -> gtk::DrawingArea {
            set_content_height: height.max(MINIMUM_HEIGHT),
            set_hexpand: true,
            set_focusable: true,
            add_css_class: "stave-slider",
            update_property: &[gtk::accessible::Property::Label(&label)],

            // A copy of the model to draw, and what screen readers hear,
            // after every update.
            #[watch]
            set_draw_func: model.painter(),
            #[watch]
            update_property: &[
                gtk::accessible::Property::ValueMin(model.track.lower),
                gtk::accessible::Property::ValueMax(model.track.upper),
                gtk::accessible::Property::ValueNow(model.track.value),
            ],

            // Its focus ring comes and goes with the keyboard focus.
            connect_has_focus_notify => |area| area.queue_draw(),
            // Right to left draws it the other way round.
            connect_direction_changed => |area, _| area.queue_draw(),

            // Press or drag anywhere on it to set the value.
            add_controller = gtk::GestureDrag {
                connect_drag_begin[sender] => move |gesture, x, _| {
                    let Some(area) = gesture.widget() else {
                        return;
                    };
                    area.grab_focus();
                    let width = f64::from(area.width());
                    let right_to_left = right_to_left(&area);
                    sender
                        .input_sender()
                        .emit(SliderInput::PointedAt { x, width, right_to_left });
                },
                connect_drag_update[sender] => move |gesture, dx, _| {
                    let (Some(area), Some((x, _))) = (gesture.widget(), gesture.start_point()) else {
                        return;
                    };
                    let width = f64::from(area.width());
                    let right_to_left = right_to_left(&area);
                    sender.input_sender().emit(SliderInput::PointedAt {
                        x: x + dx,
                        width,
                        right_to_left,
                    });
                },
            },

            // Arrow keys move it by a hundredth, Page keys by a tenth.
            add_controller = gtk::EventControllerKey {
                connect_key_pressed[sender] => move |controller, key, _, _| {
                    let backwards = controller.widget().is_some_and(|area| right_to_left(&area));
                    let Some(step) = Step::from_key(key, backwards) else {
                        return glib::Propagation::Proceed;
                    };
                    sender.input_sender().emit(SliderInput::Stepped(step));
                    glib::Propagation::Stop
                },
            },

            // The wheel moves it a twentieth a notch: up or right (left,
            // right to left) for more.
            add_controller = gtk::EventControllerScroll::new(
                gtk::EventControllerScrollFlags::BOTH_AXES
                    | gtk::EventControllerScrollFlags::DISCRETE,
            ) {
                connect_scroll[sender] => move |controller, dx, dy| {
                    let right_to_left = controller.widget().is_some_and(|area| right_to_left(&area));
                    sender
                        .input_sender()
                        .emit(SliderInput::Scrolled { dx, dy, right_to_left });
                    glib::Propagation::Stop
                },
            },
        }
    }

    fn init(
        init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let SliderInit {
            label,
            height,
            handle,
            ring,
        } = init;
        let model = SliderComponent {
            track: Track {
                lower: 0.0,
                upper: 1.0,
                value: 0.0,
                blocked_from: None,
                handle,
                ring,
            },
            value_text: None,
        };
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: SliderInput, sender: ComponentSender<Self>, root: &Self::Root) {
        let track = &mut self.track;
        let moved = match message {
            SliderInput::PointedAt {
                x,
                width,
                right_to_left,
            } => {
                let inset = track.handle / 2.0;
                let usable = (width - track.handle).max(1.0);
                let x = if right_to_left { width - x } else { x };
                let fraction = ((x - inset) / usable).clamp(0.0, 1.0);
                track.change(track.lower + fraction * (track.upper - track.lower))
            }
            SliderInput::Stepped(step) => {
                let hundredth = (track.upper - track.lower) / 100.0;
                let next = match step {
                    Step::Down => track.value - hundredth,
                    Step::Up => track.value + hundredth,
                    Step::PageDown => track.value - hundredth * 10.0,
                    Step::PageUp => track.value + hundredth * 10.0,
                    Step::Home => track.lower,
                    Step::End => track.upper,
                };
                track.change(next)
            }
            SliderInput::Scrolled {
                dx,
                dy,
                right_to_left,
            } => {
                let across = if right_to_left { -dx } else { dx };
                let notches = across - dy;
                if notches == 0.0 {
                    None
                } else {
                    track.change(track.value + notches * SCROLL_STEP * (track.upper - track.lower))
                }
            }
            SliderInput::SetValue(value) => {
                track.value = value.clamp(track.lower, track.upper);
                None
            }
            SliderInput::SetRange { lower, upper } => {
                track.lower = lower;
                track.upper = upper.max(lower);
                track.value = track.value.clamp(track.lower, track.upper);
                None
            }
            SliderInput::SetBlocked(blocked) => {
                match &blocked {
                    Some((_, words)) => {
                        root.update_property(&[gtk::accessible::Property::Description(words)]);
                    }
                    None => root.reset_property(gtk::AccessibleProperty::Description),
                }
                track.blocked_from = blocked.map(|(from, _)| from);
                None
            }
            SliderInput::SetValueText(text) => {
                self.value_text = Some(text);
                None
            }
        };
        if let Some(text) = self.spoken_value() {
            root.update_property(&[gtk::accessible::Property::ValueText(&text)]);
        }
        if let Some(value) = moved {
            // Nobody listening is not a mistake: the owner may not want
            // to hear.
            let _ = sender.output(SliderOutput::Changed(value));
        }
    }
}

/// A slider, behind the callback API the window still uses: it owns the
/// component (dropping it shuts the component down), passes its setters
/// on as [`SliderInput`] messages and its [`SliderOutput`] to the
/// callback.
///
/// Setters are queued: they take effect when the main loop next runs,
/// after whatever called them has returned.
pub struct Slider {
    /// The widget.
    pub root: gtk::DrawingArea,
    bridge: Bridge<SliderComponent>,
}

impl Slider {
    /// A slider from 0 to 1, `height` pixels tall (at least 24), with a
    /// handle `handle` pixels across and a ring `ring` pixels wide.
    pub fn new(label: &str, height: i32, handle: f64, ring: f64) -> Slider {
        let bridge = Bridge::<SliderComponent>::launch(SliderInit {
            label: label.to_owned(),
            height,
            handle,
            ring,
        });
        Slider {
            root: bridge.widget().clone(),
            bridge,
        }
    }

    /// Calls `callback` with the value whenever people move it.
    pub fn connect_changed(&self, callback: impl Fn(f64) + 'static) {
        self.bridge.connect(move |output| match output {
            SliderOutput::Changed(value) => callback(value),
        });
    }

    /// The value, as of the last message the slider handled.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "kept for the slider's API; only tests read it now"
        )
    )]
    pub fn value(&self) -> f64 {
        self.model().track.value
    }

    /// Moves it, without calling back.
    pub fn set_value(&self, value: f64) {
        self.bridge.send(SliderInput::SetValue(value));
    }

    /// Sets its range, without calling back.
    pub fn set_range(&self, lower: f64, upper: f64) {
        self.bridge.send(SliderInput::SetRange { lower, upper });
    }

    /// Hatches the track red from `from` to the end, and gives `words`
    /// ("blocked from 0.6°") as its description for screen readers
    /// (`None`: nothing blocked).
    pub fn set_blocked(&self, blocked: Option<(f64, &str)>) {
        self.bridge.send(SliderInput::SetBlocked(
            blocked.map(|(from, words)| (from, words.to_owned())),
        ));
    }

    /// Says the value in words for screen readers ("45°"; "45°, blocked"
    /// once it is in the blocked range).
    pub fn set_value_text(&self, text: &str) {
        self.bridge.send(SliderInput::SetValueText(text.to_owned()));
    }

    fn model(&self) -> Ref<'_, SliderComponent> {
        self.bridge.model()
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
    state: &Track,
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
    if right_to_left(area) {
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
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;
    use crate::ui::gtk_test::{on_gtk_thread, settle};

    /// The slider's press-and-drag controller, to fire its signals as GTK
    /// would.
    fn drag_of(slider: &Slider) -> Option<gtk::GestureDrag> {
        let controllers = slider.root.observe_controllers();
        (0..controllers.n_items())
            .filter_map(|index| controllers.item(index))
            .find_map(|item| item.downcast::<gtk::GestureDrag>().ok())
    }

    /// Fires `drag-begin` at `x` pixels across, as a press there would.
    fn press(drag: &gtk::GestureDrag, x: f64) {
        drag.emit_by_name::<()>("drag-begin", &[&x, &0.0_f64]);
    }

    /// The slider's scroll-wheel controller, to fire its signal as GTK
    /// would.
    fn scroll_of(slider: &Slider) -> Option<gtk::EventControllerScroll> {
        let controllers = slider.root.observe_controllers();
        (0..controllers.n_items())
            .filter_map(|index| controllers.item(index))
            .find_map(|item| item.downcast::<gtk::EventControllerScroll>().ok())
    }

    /// Fires `scroll` by `dx`, `dy` notches, as the wheel would.
    fn wheel(scroll: &gtk::EventControllerScroll, dx: f64, dy: f64) {
        scroll.emit_by_name::<bool>("scroll", &[&dx, &dy]);
    }

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
        on_gtk_thread(heard_mirrored_scrolled_and_blocked);
    }

    /// The pointer and the wheel follow the text's direction, it is never
    /// shorter than 24 px, and a screen reader hears where it is blocked
    /// and when the value is past that. Presses and turns of the wheel go
    /// through the slider's own controllers, which read its direction.
    fn heard_mirrored_scrolled_and_blocked() {
        let slider = Slider::new("Swing it", 18, 22.0, 2.0);
        assert_eq!(slider.root.content_height(), MINIMUM_HEIGHT);
        let (Some(drag), Some(scroll)) = (drag_of(&slider), scroll_of(&slider)) else {
            panic!("the slider has drag and scroll controllers");
        };
        let window = gtk::Window::builder()
            .default_width(222)
            .child(&slider.root)
            .build();
        window.present();
        settle();
        let width = f64::from(slider.root.width());
        assert!(width > 22.0, "the slider has room: {width}");
        // Near the left end: low left to right, high right to left.
        press(&drag, 11.0);
        settle();
        assert!(slider.value() < 0.01, "{}", slider.value());
        slider.root.set_direction(gtk::TextDirection::Rtl);
        press(&drag, 11.0);
        settle();
        assert!(slider.value() > 0.99, "{}", slider.value());
        // Right to left, the wheel's right is lower; up is always higher.
        slider.set_value(0.5);
        wheel(&scroll, 1.0, 0.0);
        settle();
        assert!((slider.value() - 0.45).abs() < 1e-9, "{}", slider.value());
        wheel(&scroll, 0.0, -2.0);
        settle();
        assert!((slider.value() - 0.55).abs() < 1e-9, "{}", slider.value());

        let description = gtk::AccessibleProperty::Description;
        assert!(!gtk::test_accessible_has_property(
            &slider.root,
            description
        ));
        slider.set_blocked(Some((0.6, "blocked from 0.6°")));
        settle();
        assert!(gtk::test_accessible_has_property(&slider.root, description));
        slider.set_value_text("0.5°");
        settle();
        assert_eq!(slider.model().spoken_value().as_deref(), Some("0.5°"));
        slider.set_value(0.7);
        slider.set_value_text("0.7°");
        settle();
        assert_eq!(
            slider.model().spoken_value().as_deref(),
            Some("0.7°, blocked")
        );
        slider.set_blocked(None);
        settle();
        assert!(!gtk::test_accessible_has_property(
            &slider.root,
            description
        ));
        assert_eq!(slider.model().spoken_value().as_deref(), Some("0.7°"));
        window.destroy();
    }

    #[test]
    fn a_dropped_slider_is_freed() {
        on_gtk_thread(slider_is_freed);
    }

    /// Dropping the slider drops its controller, which shuts the
    /// component down when the main loop next runs: the widget, the
    /// model (and the input channel the runtime held) and the callback
    /// are all freed.
    fn slider_is_freed() {
        let slider = Slider::new("Together … apart", 24, 12.0, 1.0);
        let token = Rc::new(());
        let held = Rc::clone(&token);
        slider.connect_changed(move |_| {
            let _ = &held;
        });
        let callback = Rc::downgrade(&token);
        drop(token);
        let root = slider.root.downgrade();
        let input = slider.bridge.sender().clone();
        settle();
        assert!(root.upgrade().is_some(), "the widget lives while held");
        drop(slider);
        settle();
        assert!(root.upgrade().is_none(), "the widget is freed");
        assert!(
            input.send(SliderInput::SetValue(0.5)).is_err(),
            "the component's runtime, and with it the model, is gone"
        );
        assert!(callback.upgrade().is_none(), "the callback is freed");
    }

    #[test]
    fn setters_move_it_without_calling_back() {
        on_gtk_thread(setters_do_not_call_back);
    }

    fn setters_do_not_call_back() {
        let slider = Slider::new("Door angle", 22, 22.0, 2.0);
        let calls = Rc::new(RefCell::new(0));
        let counted = Rc::clone(&calls);
        slider.connect_changed(move |_| *counted.borrow_mut() += 1);
        slider.set_range(0.0, 2.0);
        slider.set_value(3.0);
        slider.set_blocked(Some((1.5, "blocked from 1.5")));
        slider.set_value_text("90°");
        settle();
        assert_eq!(slider.value(), 2.0, "clamped to the range");
        assert_eq!(slider.model().track.blocked_from, Some(1.5));
        assert_eq!(*calls.borrow(), 0, "setters never call back");
    }

    #[test]
    fn a_signal_fired_while_handling_an_output_is_queued_not_re_entered() {
        on_gtk_thread(signals_are_queued);
    }

    /// The old slider called its callback from inside its own change,
    /// with GTK's signal still on the stack, and a callback that moved
    /// the slider re-entered it. Now a press only queues a message; the
    /// update runs later, and its output reaches the callback after the
    /// update has returned (reading the model there would panic if it
    /// had not). A signal fired from inside the callback is queued too.
    fn signals_are_queued() {
        let slider = Rc::new(Slider::new("Door angle", 22, 22.0, 2.0));
        let Some(drag) = drag_of(&slider) else {
            panic!("the slider has a drag controller");
        };
        // (what the callback heard, the value it read, the value it read
        // after firing a press from inside)
        let heard: Rc<RefCell<Vec<(f64, f64, f64)>>> = Rc::default();
        let log = Rc::clone(&heard);
        let weak = Rc::downgrade(&slider);
        let inner = drag.clone();
        slider.connect_changed(move |value| {
            let Some(slider) = weak.upgrade() else {
                return;
            };
            let read = slider.value();
            let first = log.borrow().is_empty();
            if first {
                // Moving it from here is queued, and never calls back.
                slider.set_value(0.25);
                // GTK firing the slider's own signal from in here only
                // queues a message: the value doesn't change under us.
                press(&inner, 0.0);
            }
            log.borrow_mut().push((value, read, slider.value()));
        });

        // An unrealised slider is 0 px wide: any press past the handle is
        // the upper end.
        press(&drag, 1000.0);
        assert!(heard.borrow().is_empty(), "nothing has run yet");
        assert_eq!(slider.value(), 0.0, "the press is only queued");

        settle();
        assert_eq!(
            *heard.borrow(),
            vec![(1.0, 1.0, 1.0), (0.0, 0.0, 0.0)],
            "each output after its update, the inner press after the first"
        );
        assert_eq!(slider.value(), 0.0, "the inner press came last");
    }
}
