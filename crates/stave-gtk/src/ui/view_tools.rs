//! The 3D view's floating controls (Main.dc.html, Exploded.dc.html): the
//! view toolbar at the top left ("Pull apart", which grows a play button
//! and a "together … apart" slider while it is on), the camera buttons at
//! the top right (fit, front view, turn to the back), the "Shared wood at
//! first contact" key shown while a problem is posed, and focus mode's
//! "From above" / "In 3D" switch.

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

/// The view toolbar at the top left: "Pull apart", and while it is on a
/// play button and the "together … apart" slider.
pub struct ViewToolbar {
    /// The toolbar, to overlay on the view.
    pub root: gtk::Box,
    /// Pull apart, on or off.
    pub pull: gtk::ToggleButton,
    /// Plays the pulling apart from together to apart.
    pub play: gtk::Button,
    play_icon: PathIcon,
    /// How far apart, 0 to 1.
    pub slider: Slider,
    extra: gtk::Box,
}

impl ViewToolbar {
    /// The toolbar, off.
    pub fn new() -> ViewToolbar {
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        content.append(&PathIcon::new(icons::PULL_APART, 16));
        content.append(&gtk::Label::new(Some("Pull apart")));
        let pull = gtk::ToggleButton::builder()
            .child(&content)
            .tooltip_text("Pull the parts apart along their joints")
            .css_classes(["stave-tool"])
            .build();
        let separator = gtk::Separator::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["stave-tool-separator"])
            .build();
        let play_icon = PathIcon::new(icons::PLAY, 13);
        let play = gtk::Button::builder()
            .child(&play_icon)
            .tooltip_text("Play: pull apart from together")
            .css_classes(["stave-tool-play", "circular", "flat"])
            .valign(gtk::Align::Center)
            .build();
        play.update_property(&[gtk::accessible::Property::Label("Play pull apart")]);
        let slider = Slider::new("How far apart", 18, 12.0, 1.5);
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
            .width_request(240)
            .css_classes(["stave-tool-slider"])
            .build();
        column.append(&slider.root);
        column.append(&ends);
        let extra = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        extra.append(&separator);
        extra.append(&play);
        extra.append(&column);
        extra.set_visible(false);
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(4)
            .halign(gtk::Align::Start)
            .valign(gtk::Align::Start)
            .margin_top(12)
            .margin_start(12)
            .css_classes(["stave-toolbar"])
            .name("view-toolbar")
            .build();
        root.append(&pull);
        root.append(&extra);
        ViewToolbar {
            root,
            pull,
            play,
            play_icon,
            slider,
            extra,
        }
    }

    /// Shows the toolbar on (with its slider at `amount`) or off.
    pub fn show(&self, on: bool, amount: f32, playing: bool) {
        self.pull.set_active(on);
        self.extra.set_visible(on);
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
}

/// An explode amount as a whole percentage.
pub fn percent(amount: f32) -> i32 {
    let percent = (amount.clamp(0.0, 1.0) * 100.0).round();
    // In 0..=100, so the conversion is exact.
    crate::convert::int_of_f32(percent)
}

/// The camera buttons at the top right.
pub struct CameraToolbar {
    /// The toolbar, to overlay on the view.
    pub root: gtk::Box,
    /// Frames everything (or the selection).
    pub fit: gtk::Button,
    /// Looks at the front.
    pub front: gtk::Button,
    /// Turns the view half way round.
    pub turn: gtk::Button,
}

impl CameraToolbar {
    /// The buttons.
    pub fn new() -> CameraToolbar {
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
            .halign(gtk::Align::End)
            .valign(gtk::Align::Start)
            .margin_top(14)
            .margin_end(14)
            .css_classes(["stave-camera"])
            .name("camera-toolbar")
            .build();
        root.append(&fit);
        root.append(&front);
        root.append(&turn);
        CameraToolbar {
            root,
            fit,
            front,
            turn,
        }
    }
}

/// The key shown while a problem is posed: a red square and "Shared wood
/// at first contact".
pub fn contact_key() -> gtk::Box {
    let root = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .halign(gtk::Align::Start)
        .valign(gtk::Align::Start)
        .margin_top(14)
        .margin_start(14)
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
