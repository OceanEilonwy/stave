//! The 3D view's floating controls (Main.dc.html, Exploded.dc.html): the
//! column of buttons at the top right (fit, front view, turn to the back,
//! then X-ray and Pull apart, whose play button and "together … apart"
//! slider pop out to its left while it is on), the "Shared wood at first
//! contact" key shown while a problem is posed, and focus mode's "From
//! above" / "In 3D" switch. `view_layout` says where each one goes.

use std::cell::Cell;

use adw::prelude::*;

use super::icons::{self, PathIcon};
use super::slider::Slider;

/// A button holding one icon, with a tooltip and an accessible name.
fn icon_button(shape: icons::Shape, label: &str, classes: &[&str]) -> gtk::Button {
    let button = gtk::Button::builder()
        .child(&PathIcon::new(shape, 16))
        .tooltip_text(label)
        .css_classes(classes.to_vec())
        .build();
    button.update_property(&[gtk::accessible::Property::Label(label)]);
    button
}

/// An icon button that stays pressed while its tool is on.
fn icon_toggle(shape: icons::Shape, label: &str, tooltip: &str) -> gtk::ToggleButton {
    let button = gtk::ToggleButton::builder()
        .child(&PathIcon::new(shape, 16))
        .tooltip_text(tooltip)
        .css_classes(["stave-camera-button", "flat"])
        .build();
    button.update_property(&[gtk::accessible::Property::Label(label)]);
    button
}

/// The view's own tools, in the camera column: X-ray and Pull apart, and
/// while Pull apart is on its play button and "together … apart" slider,
/// popping out to the left of its button.
pub struct ViewToolbar {
    /// See through everything but the selection.
    pub xray: gtk::ToggleButton,
    /// Pull apart, on or off.
    pub pull: gtk::ToggleButton,
    /// The play button and the slider, floating beside Pull apart while
    /// it is on.
    pub popout: gtk::Box,
    /// Plays the pulling apart from together to apart.
    pub play: gtk::Button,
    play_icon: PathIcon,
    /// How far apart, 0 to 1.
    pub slider: Slider,
    /// X-ray and Pull apart with the line above them, for the camera
    /// column.
    section: gtk::Box,
    /// Pull apart is on.
    on: Cell<bool>,
    /// The tools are offered (not while a problem is posed).
    offered: Cell<bool>,
}

impl ViewToolbar {
    /// The tools, off.
    pub fn new() -> ViewToolbar {
        let xray = icon_toggle(
            icons::XRAY,
            "X-ray",
            "X-ray: see through everything but the selection",
        );
        let pull = icon_toggle(
            icons::PULL_APART,
            "Pull apart",
            "Pull apart: move the parts away from each other along their joints",
        );
        let play_icon = PathIcon::new(icons::PLAY, 13);
        let play = gtk::Button::builder()
            .child(&play_icon)
            .tooltip_text("Play: pull apart from together")
            .css_classes(["stave-tool-play", "circular", "flat"])
            .valign(gtk::Align::Center)
            .build();
        play.update_property(&[gtk::accessible::Property::Label("Play pull apart")]);
        let slider = Slider::new("How far apart", 24, 12.0, 1.5);
        let ends = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        ends.append(
            &gtk::Label::builder()
                .label("together")
                .xalign(0.0)
                .hexpand(true)
                .css_classes(["stave-tool-end"])
                .build(),
        );
        ends.append(
            &gtk::Label::builder()
                .label("apart")
                .xalign(1.0)
                .css_classes(["stave-tool-end"])
                .build(),
        );
        let column = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .hexpand(true)
            .width_request(200)
            .css_classes(["stave-tool-slider"])
            .build();
        column.append(&slider.root);
        column.append(&ends);
        let popout = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(4)
            .css_classes(["stave-camera", "stave-pull-popout"])
            .name("pull-apart-slider")
            .visible(false)
            .build();
        popout.update_property(&[gtk::accessible::Property::Label("Pull apart")]);
        popout.append(&play);
        popout.append(&column);
        let section = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(4)
            .build();
        section.append(
            &gtk::Separator::builder()
                .orientation(gtk::Orientation::Horizontal)
                .css_classes(["stave-camera-separator"])
                .build(),
        );
        section.append(&xray);
        section.append(&pull);
        ViewToolbar {
            xray,
            pull,
            popout,
            play,
            play_icon,
            slider,
            section,
            on: Cell::new(false),
            offered: Cell::new(true),
        }
    }

    /// Shows Pull apart on (with its slider at `amount`) or off.
    pub fn show(&self, on: bool, amount: f32, playing: bool) {
        self.on.set(on);
        self.pull.set_active(on);
        self.popout.set_visible(on && self.offered.get());
        self.slider.set_value(f64::from(amount));
        self.slider
            .set_value_text(&format!("{}%, parts apart", percent(amount)));
        self.play_icon
            .set_shape(if playing { icons::PAUSE } else { icons::PLAY });
        self.play.set_tooltip_text(Some(if playing {
            "Pause"
        } else {
            "Play: pull apart from together"
        }));
    }

    /// Offers the tools, or takes them away while a problem is posed or
    /// a joint is in focus.
    pub fn offer(&self, offered: bool) {
        self.offered.set(offered);
        self.section.set_visible(offered);
        self.popout.set_visible(offered && self.on.get());
    }
}

/// An explode amount as a whole percentage.
pub fn percent(amount: f32) -> i32 {
    let percent = (amount.clamp(0.0, 1.0) * 100.0).round();
    // In 0..=100, so the conversion is exact.
    crate::convert::int_of_f32(percent)
}

/// The column of buttons at the top right of the view: fit, front view
/// and turn to the back, then X-ray and Pull apart.
pub struct CameraToolbar {
    /// The column, to float on the view.
    pub root: gtk::Box,
    /// Frames everything (or the selection).
    pub fit: gtk::Button,
    /// Looks at the front.
    pub front: gtk::Button,
    /// Turns the view half way round.
    pub turn: gtk::Button,
}

impl CameraToolbar {
    /// The buttons, with the view's own tools at the foot.
    pub fn new(tools: &ViewToolbar) -> CameraToolbar {
        let fit = icon_button(
            icons::FIT,
            "Fit to view (F)",
            &["stave-camera-button", "flat"],
        );
        let front = icon_button(icons::FRONT, "Front view", &["stave-camera-button", "flat"]);
        let turn = icon_button(
            icons::TURN,
            "Turn to the back",
            &["stave-camera-button", "flat"],
        );
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(4)
            .css_classes(["stave-camera"])
            .name("camera-toolbar")
            .build();
        root.append(&fit);
        root.append(&front);
        root.append(&turn);
        root.append(&tools.section);
        // While Pull apart's slider pops out beside its button, the
        // column's corner meets it square.
        let column = root.downgrade();
        tools.popout.connect_visible_notify(move |popout| {
            if let Some(column) = column.upgrade() {
                if popout.is_visible() {
                    column.add_css_class("joined");
                } else {
                    column.remove_css_class("joined");
                }
            }
        });
        CameraToolbar {
            root,
            fit,
            front,
            turn,
        }
    }

    /// Lays the buttons out in a column (docked on the right edge) or a
    /// row (docked in a corner), X-ray and Pull apart with them.
    pub fn orientation_setter(&self, tools: &ViewToolbar) -> impl Fn(gtk::Orientation) + 'static {
        let root = self.root.clone();
        let section = tools.section.clone();
        move |orientation| {
            root.set_orientation(orientation);
            section.set_orientation(orientation);
            if let Some(line) = section
                .first_child()
                .and_then(|child| child.downcast::<gtk::Separator>().ok())
            {
                line.set_orientation(match orientation {
                    gtk::Orientation::Vertical => gtk::Orientation::Horizontal,
                    _ => gtk::Orientation::Vertical,
                });
            }
            match orientation {
                gtk::Orientation::Vertical => root.remove_css_class("flat-row"),
                _ => root.add_css_class("flat-row"),
            }
        }
    }
}

/// The key shown while a problem is posed: a red square and "Shared wood
/// at first contact".
pub fn contact_key() -> gtk::Box {
    let root = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Start)
        .margin_top(52)
        .css_classes(["stave-contact-key"])
        .visible(false)
        .build();
    root.append(&icons::swatch(
        gtk::gdk::RGBA::new(0.878, 0.106, 0.141, 1.0),
        false,
        10,
    ));
    root.append(&gtk::Label::new(Some("Shared wood at first contact")));
    root
}

/// Focus mode's switch between the view from above and the view in 3D.
pub struct PlanSwitch {
    /// The switch, to overlay on the view.
    pub root: gtk::Box,
    /// From above (plan).
    pub from_above: gtk::ToggleButton,
    /// In 3D.
    pub in_3d: gtk::ToggleButton,
}

impl PlanSwitch {
    /// The switch, in 3D.
    pub fn new() -> PlanSwitch {
        let toggle = |shape: icons::Shape, label: &str| {
            let button = gtk::ToggleButton::builder()
                .child(&PathIcon::new(shape, 17))
                .tooltip_text(label)
                .css_classes(["stave-plan-button"])
                .build();
            button.update_property(&[gtk::accessible::Property::Label(label)]);
            button
        };
        let from_above = toggle(icons::FROM_ABOVE, "From above");
        let in_3d = toggle(icons::PARTS, "In 3D");
        in_3d.set_group(Some(&from_above));
        in_3d.set_active(true);
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(2)
            .halign(gtk::Align::End)
            .valign(gtk::Align::Start)
            .margin_top(12)
            .margin_end(12)
            .css_classes(["stave-plan-switch"])
            .visible(false)
            .build();
        root.update_property(&[gtk::accessible::Property::Label("Hinge view")]);
        root.append(&from_above);
        root.append(&in_3d);
        PlanSwitch {
            root,
            from_above,
            in_3d,
        }
    }

    /// Shows which view is on.
    pub fn show(&self, plan: bool) {
        if plan {
            self.from_above.set_active(true);
        } else {
            self.in_3d.set_active(true);
        }
    }
}

/// What the 3D view's area shows when there is nothing to draw (still
/// checking, a library folder, a design that doesn't solve): an icon, a
/// title, what it means, and "Open a Project…" where that helps. Never a
/// blank area.
pub struct EmptyView {
    /// The page, on the stage colour.
    pub root: gtk::Box,
    title: gtk::Label,
    description: gtk::Label,
    /// Opens another project folder (`win.open`).
    pub open: gtk::Button,
}

impl EmptyView {
    /// The page, saying "Checking…".
    pub fn new() -> EmptyView {
        let icon = PathIcon::new(icons::PARTS, 48);
        icon.add_css_class("stave-empty-icon");
        let title = gtk::Label::builder()
            .label("Checking…")
            .wrap(true)
            .justify(gtk::Justification::Center)
            .css_classes(["stave-empty-title"])
            .build();
        let description = gtk::Label::builder()
            .wrap(true)
            .justify(gtk::Justification::Center)
            .max_width_chars(48)
            .css_classes(["stave-empty-description"])
            .visible(false)
            .build();
        let open = gtk::Button::builder()
            .label("Open a Project…")
            .action_name("win.open")
            .halign(gtk::Align::Center)
            .css_classes(["pill", "suggested-action"])
            .visible(false)
            .build();
        let column = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .valign(gtk::Align::Center)
            .halign(gtk::Align::Center)
            .vexpand(true)
            .margin_start(24)
            .margin_end(24)
            .build();
        column.append(&icon);
        column.append(&title);
        column.append(&description);
        column.append(&open);
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["stave-stage", "stave-empty"])
            .build();
        root.append(&column);
        EmptyView {
            root,
            title,
            description,
            open,
        }
    }

    /// Says why there is nothing to draw.
    pub fn show(&self, title: &str, description: &str, offers_open: bool) {
        self.title.set_label(title);
        self.description.set_label(description);
        self.description.set_visible(!description.is_empty());
        self.open.set_visible(offers_open);
    }
}
