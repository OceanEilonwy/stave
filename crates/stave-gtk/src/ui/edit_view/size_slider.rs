//! The slider of the size editor (`docs/gui.md`, "Sizes"): a track the
//! width of the editor, its ends past the size's stops hatched red, the
//! value it had marked with a tick, and a round thumb at the value, in
//! the tone of where the change stands (blue while it waits or is being
//! checked, green when it fits, red when it doesn't). Pressing anywhere on
//! it moves the thumb there; dragging moves it on. The slider never moves
//! under the pointer, so the drag's offsets are the pointer's travel.
//!
//! ```text
//!  ////|━━━━━━━━━━━━━━|━━━━━━━●────────────────|////
//!   blocked   low stop   was   value          high stop  blocked
//! ```
//!
//! Positions are steps of `grip_values::SLIDER_STEPS` along it; what a
//! position is worth is the session's (`grip_values::along`).

use adw::prelude::*;
use gtk::{gdk, glib};
use stave_app::grip_values::SLIDER_STEPS;

/// The thumb's radius, in pixels; the track is inset by it at each end.
const THUMB: f64 = 9.0;

/// Where a change stands, as the slider and the editor colour it (always
/// with words beside it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum Tone {
    /// Nothing tried yet.
    #[default]
    Waiting,
    /// Being checked.
    Checking,
    /// It fits.
    Fits,
    /// It doesn't fit.
    Refused,
}

impl Tone {
    /// The tone's colour: the accent blue, the link blue, green, red.
    fn rgb(self) -> (f64, f64, f64) {
        match self {
            Tone::Waiting => (0.208, 0.518, 0.894),
            Tone::Checking => (0.600, 0.757, 0.945),
            Tone::Fits => (0.341, 0.890, 0.537),
            Tone::Refused => (1.0, 0.482, 0.388),
        }
    }

    /// The tone's style class.
    pub(super) fn class(self) -> &'static str {
        match self {
            Tone::Waiting => "waiting",
            Tone::Checking => "checking",
            Tone::Fits => "fits",
            Tone::Refused => "refused",
        }
    }
}

/// What the slider draws, positions in steps along it.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct SliderLook {
    /// The size's stops, when it has a range: past them it is blocked.
    pub(super) stops: Option<(i32, i32)>,
    /// The value it had.
    pub(super) was: i32,
    /// The value now.
    pub(super) value: i32,
    /// Where the change stands.
    pub(super) tone: Tone,
}

/// What the slider reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SliderEvent {
    /// Pressed or dragged to a position.
    Moved(i32),
    /// Let go at a position.
    Released(i32),
    /// An arrow key: whole units up (positive) or down; ten with Ctrl.
    Nudged(i32),
    /// Enter.
    Enter,
    /// Escape.
    Escape,
}

/// The slider's widget. It shares no state: its draw function is given a
/// copy of what it draws at each [`SizeSlider::show`] (as `slider.rs`'s
/// is), and its gesture and key handlers only send the owner's message.
#[derive(Debug, Clone)]
pub(super) struct SizeSlider {
    /// The track, drawn.
    pub(super) area: gtk::DrawingArea,
}

impl SizeSlider {
    /// A slider named `name` for screen readers; what people do on it goes
    /// to `input` as `message(event)`.
    pub(super) fn new<M: 'static>(
        name: &str,
        input: &relm4::Sender<M>,
        message: fn(SliderEvent) -> M,
    ) -> SizeSlider {
        let area = gtk::DrawingArea::builder()
            .content_height(30)
            .hexpand(true)
            .focusable(true)
            .css_classes(["stave-size-slider"])
            .accessible_role(gtk::AccessibleRole::Slider)
            .build();
        area.set_cursor_from_name(Some("pointer"));
        area.update_property(&[gtk::accessible::Property::Label(name)]);
        let slider = SizeSlider { area };
        slider.show(SliderLook::default());
        connect_pointer(&slider.area, input, message);
        connect_keys(&slider.area, input, message);
        slider
    }

    /// Draws `look`: the draw function owns this copy until the next.
    pub(super) fn show(&self, look: SliderLook) {
        self.area.set_draw_func(move |_, context, width, height| {
            draw(context, f64::from(width), f64::from(height), look);
        });
    }
}

/// The position under `x` pixels from the slider's left edge.
fn position_at(area: &gtk::DrawingArea, x: f64) -> i32 {
    let track = (f64::from(area.width()) - 2.0 * THUMB).max(1.0);
    let fraction = ((x - THUMB) / track).clamp(0.0, 1.0);
    crate::convert::int_of(fraction * f64::from(SLIDER_STEPS))
}

/// The pixel at `position`, across a slider `width` wide.
fn x_at(width: f64, position: i32) -> f64 {
    THUMB + f64::from(position) / f64::from(SLIDER_STEPS) * (width - 2.0 * THUMB).max(1.0)
}

/// Where a drag is now: its start (the gesture's own) plus `dx`, as a
/// position along the gesture's widget. `None` without a widget.
fn dragged_to(gesture: &gtk::GestureDrag, dx: f64) -> Option<i32> {
    let area = gesture.widget().and_downcast::<gtk::DrawingArea>()?;
    let (x, _) = gesture.start_point()?;
    Some(position_at(&area, x + dx))
}

/// Pressing and dragging: each reads only the gesture and sends.
fn connect_pointer<M: 'static>(
    area: &gtk::DrawingArea,
    input: &relm4::Sender<M>,
    message: fn(SliderEvent) -> M,
) {
    let drag = gtk::GestureDrag::builder()
        .button(gdk::BUTTON_PRIMARY)
        .build();
    {
        let input = input.clone();
        drag.connect_drag_begin(move |gesture, _, _| {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            if let Some(area) = gesture.widget() {
                area.grab_focus();
            }
            if let Some(position) = dragged_to(gesture, 0.0) {
                input.emit(message(SliderEvent::Moved(position)));
            }
        });
    }
    {
        let input = input.clone();
        drag.connect_drag_update(move |gesture, dx, _| {
            if let Some(position) = dragged_to(gesture, dx) {
                input.emit(message(SliderEvent::Moved(position)));
            }
        });
    }
    {
        let input = input.clone();
        drag.connect_drag_end(move |gesture, dx, _| {
            if let Some(position) = dragged_to(gesture, dx) {
                input.emit(message(SliderEvent::Released(position)));
            }
        });
    }
    area.add_controller(drag);
}

fn connect_keys<M: 'static>(
    area: &gtk::DrawingArea,
    input: &relm4::Sender<M>,
    message: fn(SliderEvent) -> M,
) {
    let input = input.clone();
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(move |_, key, _, modifiers| {
        let by = if modifiers.contains(gdk::ModifierType::CONTROL_MASK) {
            10
        } else {
            1
        };
        let event = match key {
            gdk::Key::Right | gdk::Key::Up => SliderEvent::Nudged(by),
            gdk::Key::Left | gdk::Key::Down => SliderEvent::Nudged(-by),
            gdk::Key::Return | gdk::Key::KP_Enter => SliderEvent::Enter,
            gdk::Key::Escape => SliderEvent::Escape,
            _ => return glib::Propagation::Proceed,
        };
        input.emit(message(event));
        glib::Propagation::Stop
    });
    area.add_controller(keys);
}

/// Draws the track, its blocked ends, the fill, the tick and the thumb.
fn draw(context: &gtk::cairo::Context, width: f64, height: f64, look: SliderLook) {
    let middle = height / 2.0;
    let (r, g, b) = look.tone.rgb();
    // Errors drawing a frame leave the frame as it is.
    let _ = (|| -> Result<(), gtk::cairo::Error> {
        // The rail.
        context.set_source_rgba(1.0, 1.0, 1.0, 0.14);
        context.rectangle(THUMB, middle - 2.0, (width - 2.0 * THUMB).max(0.0), 4.0);
        context.fill()?;
        // The blocked ends, hatched.
        if let Some((low, high)) = look.stops {
            for (from, to) in [(0.0, x_at(width, low)), (x_at(width, high), width)] {
                if to - from < 1.0 {
                    continue;
                }
                context.save()?;
                context.rectangle(from, middle - 6.0, to - from, 12.0);
                context.clip();
                context.set_source_rgba(1.0, 0.482, 0.388, 0.45);
                context.set_line_width(1.5);
                let mut x = from - 12.0;
                while x < to + 12.0 {
                    context.move_to(x, middle + 6.0);
                    context.line_to(x + 12.0, middle - 6.0);
                    x += 6.0;
                }
                context.stroke()?;
                context.restore()?;
            }
        }
        // The fill, from the low stop (or the start) to the value.
        let from = look.stops.map_or(THUMB, |(low, _)| x_at(width, low));
        let to = x_at(width, look.value);
        if to > from {
            context.set_source_rgb(r, g, b);
            context.rectangle(from, middle - 2.0, to - from, 4.0);
            context.fill()?;
        }
        // The value it had.
        context.set_source_rgba(1.0, 1.0, 1.0, 0.55);
        context.rectangle(x_at(width, look.was) - 1.0, middle - 9.0, 2.0, 18.0);
        context.fill()?;
        // The thumb.
        context.arc(to, middle, THUMB - 1.5, 0.0, std::f64::consts::TAU);
        context.set_source_rgb(1.0, 1.0, 1.0);
        context.fill_preserve()?;
        context.set_source_rgb(r, g, b);
        context.set_line_width(3.0);
        context.stroke()?;
        Ok(())
    })();
}
