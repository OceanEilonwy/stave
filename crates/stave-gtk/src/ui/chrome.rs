//! The smaller pieces of the window: the motion bar ("Open it", floating
//! at the bottom right of the 3D view) and the Cut list's placeholder page.

use adw::prelude::*;
use stave_app::i18n::{fill_in, tr};

/// The floating bar at the bottom right of the 3D view: a name, words at each
/// end and a slider ("door_l  closed ——— 110°").
pub struct MotionBar {
    /// The bar, to overlay on the view.
    pub root: gtk::Box,
    name: gtk::Label,
    start: gtk::Label,
    end: gtk::Label,
    /// The slider.
    pub scale: gtk::Scale,
}

impl MotionBar {
    /// A hidden bar.
    pub fn new() -> MotionBar {
        let name = gtk::Label::builder().css_classes(["heading"]).build();
        let start = gtk::Label::builder().css_classes(["stave-key"]).build();
        let end = gtk::Label::builder().css_classes(["stave-key"]).build();
        let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.1);
        scale.set_width_request(220);
        scale.set_draw_value(false);
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .css_classes(["stave-floating"])
            .halign(gtk::Align::Center)
            .valign(gtk::Align::End)
            .margin_bottom(14)
            .visible(false)
            .build();
        root.append(&name);
        root.append(&start);
        root.append(&scale);
        root.append(&end);
        MotionBar {
            root,
            name,
            start,
            end,
            scale,
        }
    }

    /// Lays the bar out in a row (docked in a corner) or standing upright
    /// (docked on the right edge), its slider turned to match.
    pub fn orientation_setter(&self) -> impl Fn(gtk::Orientation) + 'static {
        let root = self.root.clone();
        let scale = self.scale.clone();
        move |orientation| {
            root.set_orientation(orientation);
            scale.set_orientation(orientation);
            match orientation {
                gtk::Orientation::Vertical => {
                    scale.set_width_request(-1);
                    scale.set_height_request(160);
                }
                _ => {
                    scale.set_width_request(220);
                    scale.set_height_request(-1);
                }
            }
        }
    }

    /// Shows the bar for `name`, from `start` to `end` words, the slider
    /// from 0 to `full` at `value`; `None` hides it.
    pub fn show(&self, what: Option<(&str, &str, &str, f64, f64)>) {
        let Some((name, start, end, full, value)) = what else {
            self.root.set_visible(false);
            return;
        };
        self.name.set_label(name);
        self.start.set_label(start);
        self.end.set_label(end);
        self.scale.set_range(0.0, full.max(0.1));
        self.scale.set_value(value);
        self.scale
            .update_property(&[gtk::accessible::Property::Label(&fill_in(
                &tr("Open {name}"),
                &[("name", &name)],
            ))]);
        self.root.set_visible(true);
    }
}

/// The Cut list view, until it is built.
pub fn cut_list_page() -> adw::StatusPage {
    adw::StatusPage::builder()
        .title(tr("Cut list is coming"))
        .description(tr("Every board to cut, sized and rounded once, will be listed here. It isn't part of this version yet."))
        .css_classes(["stave-placeholder"])
        .build()
}
