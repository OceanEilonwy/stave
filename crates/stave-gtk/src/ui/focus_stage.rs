//! What focus mode and a selected problem put in the 3D view's area
//! (JointFocus.dc.html, Hinge.dc.html, Problems.dc.html):
//!
//! - the joint's step player: play, "Step 1 of 1 · push(rung, along:
//!   length)", how far the part is from home, and the apart … together
//!   scrub;
//! - for hardware, the line saying what the view looks at, three small
//!   views at fixed angles, and the swing card (play, the door's name, the
//!   slider and its angle);
//! - a problem's motion card: "Swing door to see where it hits", the
//!   slider with the blocked range hatched and the handle red past the
//!   contact.
//!
//! The window sets their values and hears back only from what people do.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::gdk;
use stave_app::problems::MotionCard;

use super::icons::{self, PathIcon};
use super::slider::Slider;
use super::viewport::{Sizing, Viewport};

/// A round blue play button, 40 px.
fn play_button(label: &str) -> (gtk::Button, PathIcon) {
    let icon = PathIcon::new(icons::PLAY, 14);
    let button = gtk::Button::builder()
        .child(&icon)
        .tooltip_text(label)
        .css_classes(["stave-play", "circular"])
        .valign(gtk::Align::Center)
        .build();
    button.update_property(&[gtk::accessible::Property::Label(label)]);
    (button, icon)
}

fn dim(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .css_classes(["stave-card-dim"])
        .build()
}

/// Shows play or pause on a play button.
pub fn show_playing(button: &gtk::Button, icon: &PathIcon, playing: bool, label: &str) {
    icon.set_shape(if playing { icons::PAUSE } else { icons::PLAY });
    let text = if playing { "Pause" } else { label };
    button.set_tooltip_text(Some(text));
    button.update_property(&[gtk::accessible::Property::Label(text)]);
}

/// The joint's step player, floating at the bottom of the view.
pub struct StepCard {
    /// The card.
    pub root: gtk::Box,
    /// Plays the joint going together.
    pub play: gtk::Button,
    /// The play button's icon.
    pub play_icon: PathIcon,
    step: gtk::Label,
    status: gtk::Label,
    /// Apart (0) to together (1).
    pub scrub: Slider,
}

impl StepCard {
    /// A hidden card.
    pub fn new() -> StepCard {
        let (play, play_icon) = play_button("Play the assembly");
        let step = gtk::Label::builder()
            .use_markup(true)
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        let status = gtk::Label::builder()
            .xalign(1.0)
            .css_classes(["stave-card-dim"])
            .build();
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        line.append(&step);
        line.append(&status);
        let scrub = Slider::new("Apart to together", 22, 22.0, 2.0);
        let ends = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let apart = dim("apart");
        apart.set_hexpand(true);
        apart.set_xalign(0.0);
        ends.append(&apart);
        ends.append(&dim("together"));
        let column = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .hexpand(true)
            .build();
        column.append(&line);
        column.append(&scrub.root);
        column.append(&ends);
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(14)
            .valign(gtk::Align::End)
            .margin_start(16)
            .margin_end(16)
            .margin_bottom(16)
            .css_classes(["stave-focus-card"])
            .visible(false)
            .build();
        root.update_property(&[gtk::accessible::Property::Label("Assembly steps")]);
        root.append(&play);
        root.append(&column);
        StepCard {
            root,
            play,
            play_icon,
            step,
            status,
            scrub,
        }
    }

    /// Shows the step ("Step 1 of 1 · push(rung, along: length)") and the
    /// distance from home.
    pub fn show_frame(&self, step: Option<&str>, status: &str) {
        let escape = |t: &str| gtk::glib::markup_escape_text(t).to_string();
        let markup = match step.map(|step| step.split_once(" · ").unwrap_or((step, ""))) {
            Some((lead, "")) => format!("<b>{}</b>", escape(lead)),
            Some((lead, text)) => format!(
                "<b>{}</b> · <span font_family=\"monospace\" size=\"92%\">{}</span>",
                escape(lead),
                escape(text)
            ),
            None => String::new(),
        };
        self.step.set_markup(&markup);
        self.step.set_tooltip_text(step);
        self.status.set_label(status);
    }
}

/// Hardware focus: the swing (or slide) card under the small views.
pub struct SwingCard {
    /// The card.
    pub root: gtk::Box,
    /// Plays the door opening.
    pub play: gtk::Button,
    /// The play button's icon.
    pub play_icon: PathIcon,
    name: gtk::Label,
    /// Closed (0) to fully open (1).
    pub scrub: Slider,
    value: gtk::Label,
}

impl SwingCard {
    /// A hidden card.
    pub fn new() -> SwingCard {
        let (play, play_icon) = play_button("Play the swing");
        let name = gtk::Label::builder()
            .css_classes(["heading", "stave-card-name"])
            .build();
        let scrub = Slider::new("Door angle", 22, 22.0, 2.0);
        let value = gtk::Label::builder()
            .xalign(1.0)
            .width_chars(6)
            .css_classes(["stave-card-value"])
            .build();
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(14)
            .css_classes(["stave-focus-card"])
            .visible(false)
            .build();
        root.update_property(&[gtk::accessible::Property::Label("Swing the door")]);
        root.append(&play);
        root.append(&name);
        root.append(&scrub.root);
        root.append(&value);
        SwingCard {
            root,
            play,
            play_icon,
            name,
            scrub,
            value,
        }
    }

    /// Names what moves.
    pub fn show_name(&self, name: &str, turns: bool) {
        self.name.set_label(name);
        self.scrub
            .root
            .update_property(&[gtk::accessible::Property::Label(if turns {
                "Door angle"
            } else {
                "How far it slides out"
            })]);
    }

    /// Shows the reading: "45°", "250 mm".
    pub fn show_value(&self, text: &str) {
        self.value.set_label(text);
        self.scrub.set_value_text(text);
    }
}

/// The line over hardware focus's view: what it looks at and which way is
/// which.
pub struct Caption {
    /// The line.
    pub root: gtk::Box,
    lead: gtk::Label,
    hint: gtk::Label,
}

impl Caption {
    /// A hidden line.
    pub fn new() -> Caption {
        let lead = gtk::Label::builder()
            .use_markup(true)
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        let hint = gtk::Label::builder()
            .css_classes(["stave-card-dim"])
            .ellipsize(gtk::pango::EllipsizeMode::Start)
            .build();
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(12)
            .css_classes(["stave-focus-caption"])
            .visible(false)
            .build();
        root.append(&lead);
        root.append(&hint);
        Caption { root, lead, hint }
    }

    /// "**Looking down** on the hinge, through the cabinet top" and a hint
    /// at the end.
    pub fn show(&self, text: &str, hint: &str) {
        let escape = |t: &str| gtk::glib::markup_escape_text(t).to_string();
        // The first two words in bold, as the design has it.
        let mut words = text.splitn(3, ' ');
        let markup = match (words.next(), words.next(), words.next()) {
            (Some(a), Some(b), Some(rest)) => {
                format!("<b>{} {}</b> {}", escape(a), escape(b), escape(rest))
            }
            _ => format!("<b>{}</b>", escape(text)),
        };
        self.lead.set_markup(&markup);
        self.hint.set_label(hint);
        self.hint.set_visible(!hint.is_empty());
    }
}

/// Called when the pictures need rendering again at a new size.
type ResizeHandler = Rc<dyn Fn()>;

/// Hardware focus's three small views at fixed angles. Each is rendered
/// at its slot's size in device pixels and drawn one texel to a pixel, so
/// it is sharp at any size and scale.
pub struct Pictures {
    /// The row.
    pub root: gtk::Box,
    /// The slots, one a picture, in order, with their captions.
    slots: RefCell<Vec<(Viewport, String)>>,
    /// The slots' size in device pixels when last reported.
    size: Rc<Cell<(u32, u32)>>,
    /// Called when the slots' size in device pixels changes.
    on_resize: Rc<RefCell<Option<ResizeHandler>>>,
}

impl Pictures {
    /// A hidden row.
    pub fn new() -> Pictures {
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(12)
            .homogeneous(true)
            .css_classes(["stave-pictures"])
            .visible(false)
            .build();
        Pictures {
            root,
            slots: RefCell::new(Vec::new()),
            size: Rc::new(Cell::new((0, 0))),
            on_resize: Rc::new(RefCell::new(None)),
        }
    }

    /// Calls `handler` when the pictures' size in device pixels changes
    /// (the window resized, or moved to a screen of another scale): they
    /// need rendering again at [`Pictures::device_size`].
    pub fn connect_resized(&self, handler: impl Fn() + 'static) {
        self.on_resize.replace(Some(Rc::new(handler)));
    }

    /// The size, in device pixels, to render the pictures at; `None`
    /// before they have been given room.
    pub fn device_size(&self) -> Option<(u32, u32)> {
        let slots = self.slots.borrow();
        let (slot, _) = slots.first()?;
        (slot.width() > 1 && slot.height() > 1).then(|| slot.device_size())
    }

    /// Lays out a picture for each caption, each a button that goes to its
    /// pose, empty until [`Pictures::show_textures`]. The same captions
    /// as last time keep the pictures as they are.
    pub fn show(&self, captions: &[String], on_choose: impl Fn(usize) + Clone + 'static) {
        let unchanged = self
            .slots
            .borrow()
            .iter()
            .map(|(_, caption)| caption)
            .eq(captions.iter());
        if unchanged {
            return;
        }
        while let Some(child) = self.root.first_child() {
            self.root.remove(&child);
        }
        self.size.set((0, 0));
        let count = captions.len();
        let mut slots = Vec::with_capacity(count);
        for (index, caption) in captions.iter().enumerate() {
            let picture = Viewport::picture(Sizing::Picture {
                natural_width: 320,
                minimum_height: 110,
            });
            let size = self.size.clone();
            let on_resize = self.on_resize.clone();
            picture.connect_resized(move |width, height| {
                if size.replace((width, height)) == (width, height) {
                    return;
                }
                // Cloned out so that the handler may set a new one.
                let handler = on_resize.borrow().clone();
                if let Some(handler) = handler {
                    handler();
                }
            });
            let words = if index == 0 {
                "closed"
            } else if index + 1 == count {
                "fully open"
            } else {
                "part way"
            };
            let label = gtk::Label::builder()
                .use_markup(true)
                .label(format!(
                    "<b>{}</b> {words}",
                    gtk::glib::markup_escape_text(caption)
                ))
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .css_classes(["stave-picture-caption"])
                .build();
            let cell = gtk::Box::new(gtk::Orientation::Vertical, 4);
            cell.append(&picture);
            cell.append(&label);
            let button = gtk::Button::builder()
                .child(&cell)
                .css_classes(["stave-picture"])
                .tooltip_text(format!("Show it at {caption}"))
                .build();
            let choose = on_choose.clone();
            button.connect_clicked(move |_| choose(index));
            self.root.append(&button);
            slots.push((picture, caption.clone()));
        }
        self.slots.replace(slots);
    }

    /// Shows the rendered pictures, in the order of the captions.
    pub fn show_textures(&self, textures: Vec<gdk::Texture>) {
        for ((slot, _), texture) in self.slots.borrow().iter().zip(textures) {
            slot.show_frame(texture);
        }
    }

    /// Rings the picture at `current`.
    pub fn show_current(&self, current: Option<usize>) {
        let mut child = self.root.first_child();
        let mut index = 0;
        while let Some(widget) = child {
            if current == Some(index) {
                widget.add_css_class("current");
            } else {
                widget.remove_css_class("current");
            }
            index += 1;
            child = widget.next_sibling();
        }
    }
}

/// A selected problem's motion card: swing the door to see where it hits.
pub struct ProblemCard {
    /// The card, to overlay on the view.
    pub root: adw::Clamp,
    lead: gtk::Label,
    value: gtk::Label,
    /// Closed to fully open, in degrees or millimetres.
    pub scrub: Slider,
    start: gtk::Label,
    blocked: gtk::Label,
    end: gtk::Label,
}

impl ProblemCard {
    /// A hidden card.
    pub fn new() -> ProblemCard {
        let lead = gtk::Label::builder()
            .use_markup(true)
            .xalign(0.0)
            .hexpand(true)
            .build();
        let value = gtk::Label::builder()
            .css_classes(["stave-card-value", "blocked"])
            .build();
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        line.append(&lead);
        line.append(&value);
        let scrub = Slider::new("Swing it", 26, 22.0, 2.0);
        let start = dim("");
        start.set_xalign(0.0);
        start.set_hexpand(true);
        let blocked = gtk::Label::builder()
            .css_classes(["stave-card-blocked"])
            .hexpand(true)
            .build();
        let end = dim("");
        end.set_xalign(1.0);
        end.set_hexpand(true);
        let ends = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .homogeneous(true)
            .build();
        ends.append(&start);
        ends.append(&blocked);
        ends.append(&end);
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .css_classes(["stave-focus-card", "stave-problem-card"])
            .build();
        content.append(&line);
        content.append(&scrub.root);
        content.append(&ends);
        let root = adw::Clamp::builder()
            .maximum_size(620)
            .tightening_threshold(620)
            .child(&content)
            .valign(gtk::Align::End)
            .margin_start(16)
            .margin_end(16)
            .margin_bottom(16)
            .visible(false)
            .build();
        ProblemCard {
            root,
            lead,
            value,
            scrub,
            start,
            blocked,
            end,
        }
    }

    /// Shows the card for a motion, the slider at `amount` (radians or
    /// millimetres); `None` hides it.
    pub fn show(&self, card: Option<&MotionCard>, amount: f32) {
        let Some(card) = card else {
            self.root.set_visible(false);
            return;
        };
        let escape = |t: &str| gtk::glib::markup_escape_text(t).to_string();
        self.lead.set_markup(&format!(
            "<b>{}</b> <span alpha=\"70%\">to see where it hits</span>",
            escape(&card.lead())
        ));
        let (full, contact, value) = if card.turns {
            (
                f64::from(card.full).to_degrees(),
                f64::from(card.contact).to_degrees(),
                f64::from(amount).to_degrees(),
            )
        } else {
            (
                f64::from(card.full),
                f64::from(card.contact),
                f64::from(amount),
            )
        };
        self.scrub.set_range(0.0, full.max(0.1));
        self.scrub.set_blocked_from(Some(contact));
        self.scrub.set_value(value);
        self.scrub
            .root
            .update_property(&[gtk::accessible::Property::Label(&card.lead())]);
        self.start.set_label(&card.start);
        self.blocked.set_label(&card.blocked);
        self.end.set_label(&card.end);
        self.show_value(card, amount);
        self.root.set_visible(true);
    }

    /// Shows the reading at `amount`: red once it is past the contact.
    pub fn show_value(&self, card: &MotionCard, amount: f32) {
        let text = card.amount_text(amount);
        self.value.set_label(&text);
        self.scrub.set_value_text(&text);
        if amount >= card.contact {
            self.value.add_css_class("blocked");
        } else {
            self.value.remove_css_class("blocked");
        }
    }
}
