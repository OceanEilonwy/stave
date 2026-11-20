//! A number field that is dragged sideways to change it, as in Blender
//! and Figma (`docs/gui.md`, "Sizes"): it shows its value; pressing it
//! and moving left or right scrubs the value (Shift: finer, Ctrl: round
//! numbers); a click without moving turns it into an entry, where a value
//! is typed and Enter pressed (Escape puts the number back); with the
//! keyboard, the arrow keys step it and Enter writes it.
//!
//! ```text
//! ┌──────────────┐
//! │ ‹  500mm   › │   press and drag ◂▸ to scrub, click to type
//! └──────────────┘
//! ```
//!
//! The field decides nothing: it reports what the pointer and keyboard
//! do ([`ScrubEvent`]) to the closure it was made with. Unlike a handle
//! on the model the field never moves while it is dragged, so the
//! pointer's travel from where it was pressed is the whole of the scrub.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};
use stave_app::i18n::{fill_in, tr};

/// How far the pointer moves before a press is a scrub, not a click, in
/// pixels.
const SCRUB_THRESHOLD: f64 = 3.0;

/// What a field reports.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum ScrubEvent {
    /// The pointer pressed and began to move: a scrub starts.
    Began,
    /// The pointer moved `dx` pixels from where it was pressed (right is
    /// larger).
    Moved {
        /// Pixels from the press.
        dx: f64,
        /// Shift is held: finer.
        fine: bool,
        /// Ctrl is held: round numbers.
        round: bool,
    },
    /// The pointer let go of a scrub.
    Ended,
    /// Pressed and let go without moving: type a value.
    Clicked,
    /// Enter in the entry, with what was typed.
    Typed(String),
    /// Escape in the entry.
    Escaped,
    /// An arrow key on the focused field: steps up (positive) or down.
    Stepped {
        /// How many steps.
        steps: i32,
        /// Ctrl is held: round numbers.
        round: bool,
    },
    /// Enter on the focused field (not typing): write it.
    Enter,
}

/// The field's widgets.
#[derive(Debug, Clone)]
pub(super) struct ScrubField {
    /// The field: the number, or the entry it turns into.
    pub(super) root: gtk::Stack,
    /// The number, dragged.
    number: gtk::Label,
    /// The box around the number, which takes the pointer and the keys.
    display: gtk::Box,
    /// The entry.
    entry: gtk::Entry,
}

impl ScrubField {
    /// A field showing `value`, named `name` for screen readers, that
    /// reports to `heard`. A field that can't move (`movable` false)
    /// shows its value and takes nothing.
    pub(super) fn new(
        name: &str,
        value: &str,
        movable: bool,
        heard: impl Fn(ScrubEvent) + Clone + 'static,
    ) -> ScrubField {
        let number = gtk::Label::builder()
            .label(value)
            .xalign(0.5)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        let display = gtk::Box::builder()
            .css_classes(["stave-scrub"])
            .hexpand(true)
            .focusable(movable)
            .accessible_role(gtk::AccessibleRole::SpinButton)
            .build();
        let entry = gtk::Entry::builder()
            .text(value)
            .width_chars(7)
            .hexpand(true)
            .css_classes(["stave-scrub-entry"])
            .build();
        entry.update_property(&[gtk::accessible::Property::Label(&fill_in(
            &tr("Value of {size}"),
            &[("size", &name)],
        ))]);
        let root = gtk::Stack::builder()
            .hhomogeneous(true)
            .vhomogeneous(true)
            .hexpand(true)
            .build();
        root.add_named(&display, Some("number"));
        root.add_named(&entry, Some("entry"));
        root.set_visible_child_name("number");
        if movable {
            let less = gtk::Image::builder()
                .icon_name("pan-start-symbolic")
                .pixel_size(10)
                .css_classes(["stave-scrub-arrow"])
                .build();
            let more = gtk::Image::builder()
                .icon_name("pan-end-symbolic")
                .pixel_size(10)
                .css_classes(["stave-scrub-arrow"])
                .build();
            display.append(&less);
            display.append(&number);
            display.append(&more);
            display.set_cursor_from_name(Some("ew-resize"));
            display.set_tooltip_text(Some(&tr(
                "Drag left or right to change it (Shift: finer, Ctrl: round numbers). Click to type a value.",
            )));
            display.update_property(&[gtk::accessible::Property::Label(name)]);
            connect_pointer(&display, heard.clone());
            connect_keys(&display, heard.clone());
            connect_entry(&entry, heard);
        } else {
            display.add_css_class("locked");
            display.append(
                &gtk::Image::builder()
                    .icon_name("changes-prevent-symbolic")
                    .pixel_size(12)
                    .build(),
            );
            display.append(&number);
            display.update_property(&[gtk::accessible::Property::Label(name)]);
        }
        ScrubField {
            root,
            number,
            display,
            entry,
        }
    }

    /// Shows `value` in the number (and in the entry, unless it is being
    /// typed in).
    pub(super) fn set_value(&self, value: &str) {
        if self.number.label() != value {
            self.number.set_label(value);
        }
        if !self.typing() && self.entry.text() != value {
            self.entry.set_text(value);
        }
    }

    /// The value it shows.
    pub(super) fn value(&self) -> String {
        self.number.label().to_string()
    }

    /// Marks it as being scrubbed (or not).
    pub(super) fn set_scrubbing(&self, scrubbing: bool) {
        if scrubbing {
            self.display.add_css_class("scrubbing");
        } else {
            self.display.remove_css_class("scrubbing");
        }
    }

    /// `true` while the entry shows.
    pub(super) fn typing(&self) -> bool {
        self.root.visible_child_name().as_deref() == Some("entry")
    }

    /// Turns it into the entry holding `text` (all of it selected unless
    /// it is marked refused), with the keyboard.
    pub(super) fn open_entry(&self, text: &str, refused: bool) {
        self.root.set_visible_child_name("entry");
        if self.entry.text() != text {
            self.entry.set_text(text);
        }
        if refused {
            self.entry.add_css_class("error");
        } else {
            self.entry.remove_css_class("error");
        }
        self.entry.grab_focus();
        if !refused {
            self.entry.select_region(0, -1);
        }
    }

    /// Back to the number.
    pub(super) fn close_entry(&self) {
        self.entry.remove_css_class("error");
        self.root.set_visible_child_name("number");
    }

    /// Types `text` into the entry, as a person would, for a script.
    pub(super) fn type_text(&self, text: &str) {
        self.open_entry(text, false);
        self.entry.set_text(text);
    }
}

/// A press that moves past the threshold scrubs; one that doesn't is a
/// click. The field stays where it is, so the gesture's offsets are the
/// pointer's travel.
fn connect_pointer(display: &gtk::Box, heard: impl Fn(ScrubEvent) + Clone + 'static) {
    let drag = gtk::GestureDrag::builder()
        .button(gdk::BUTTON_PRIMARY)
        .build();
    let scrubbing = Rc::new(Cell::new(false));
    {
        let scrubbing = scrubbing.clone();
        drag.connect_drag_begin(move |_, _, _| scrubbing.set(false));
    }
    {
        let scrubbing = scrubbing.clone();
        let heard = heard.clone();
        drag.connect_drag_update(move |gesture, dx, dy| {
            if !scrubbing.get() {
                if dx.hypot(dy) < SCRUB_THRESHOLD {
                    return;
                }
                scrubbing.set(true);
                gesture.set_state(gtk::EventSequenceState::Claimed);
                heard(ScrubEvent::Began);
            }
            let state = gesture.current_event_state();
            heard(ScrubEvent::Moved {
                dx,
                fine: state.contains(gdk::ModifierType::SHIFT_MASK),
                round: state.contains(gdk::ModifierType::CONTROL_MASK),
            });
        });
    }
    drag.connect_drag_end(move |_, _, _| {
        if scrubbing.replace(false) {
            heard(ScrubEvent::Ended);
        } else {
            heard(ScrubEvent::Clicked);
        }
    });
    display.add_controller(drag);
}

/// The arrow keys step it; Enter writes it.
fn connect_keys(display: &gtk::Box, heard: impl Fn(ScrubEvent) + 'static) {
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(move |_, key, _, modifiers| {
        let round = modifiers.contains(gdk::ModifierType::CONTROL_MASK);
        let steps = match key {
            gdk::Key::Right | gdk::Key::Up => 1,
            gdk::Key::Left | gdk::Key::Down => -1,
            gdk::Key::Return | gdk::Key::KP_Enter => {
                heard(ScrubEvent::Enter);
                return glib::Propagation::Stop;
            }
            _ => return glib::Propagation::Proceed,
        };
        heard(ScrubEvent::Stepped { steps, round });
        glib::Propagation::Stop
    });
    display.add_controller(keys);
}

/// Enter in the entry reports what was typed; Escape reports itself.
fn connect_entry(entry: &gtk::Entry, heard: impl Fn(ScrubEvent) + Clone + 'static) {
    {
        let heard = heard.clone();
        entry.connect_activate(move |entry| {
            heard(ScrubEvent::Typed(entry.text().trim().to_owned()));
        });
    }
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(move |_, key, _, _| {
        if key != gdk::Key::Escape {
            return glib::Propagation::Proceed;
        }
        heard(ScrubEvent::Escaped);
        glib::Propagation::Stop
    });
    entry.add_controller(keys);
}

/// The fields' look: a dark rounded field, its arrows showing on hover.
pub(super) const STYLE: &str = r#"
.stave-scrub { min-height: 22px; padding: 0 4px; border-radius: 4px; background: alpha(black, 0.18);
  font-family: var(--monospace-font-family, monospace); font-size: 0.818rem; }
.stave-scrub:hover { background: alpha(black, 0.28); }
.stave-scrub:focus-visible { outline: 2px solid alpha(@accent_color, 0.5); outline-offset: -2px; }
.stave-scrub.scrubbing { background: alpha(@accent_color, 0.35); }
.stave-scrub .stave-scrub-arrow { opacity: 0; }
.stave-scrub:hover .stave-scrub-arrow, .stave-scrub.scrubbing .stave-scrub-arrow { opacity: 0.6; }
.stave-scrub.locked { opacity: 0.6; padding: 0 6px; }
.stave-scrub.locked image { margin-right: 4px; }
entry.stave-scrub-entry { min-height: 22px; font-size: 0.818rem; padding-top: 0; padding-bottom: 0;
  font-family: var(--monospace-font-family, monospace); }
"#;
