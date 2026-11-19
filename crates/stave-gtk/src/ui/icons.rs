//! The window's icons, drawn from SVG path data in the widget's text
//! colour, so they match the design exactly on every icon theme and follow
//! light and dark styles.

use std::cell::Cell;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

/// How an icon's paths are painted.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Paint {
    /// Outlined, with this stroke width in view-box units.
    Stroke(f32),
    /// Filled.
    Fill,
}

/// One icon: its paths in a square view box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shape {
    /// The view box's side.
    pub view_box: f32,
    /// How the paths are painted.
    pub paint: Paint,
    /// The paths, in SVG path syntax.
    pub paths: &'static [&'static str],
    /// Turned 45° about the centre, shrunk a little (the hammer).
    pub tilted: bool,
}

const fn stroke(width: f32, paths: &'static [&'static str]) -> Shape {
    Shape {
        view_box: 16.0,
        paint: Paint::Stroke(width),
        paths,
        tilted: false,
    }
}

/// The design: a box.
pub const PARTS: Shape = stroke(
    1.5,
    &[
        "M8 1.5L14 4.75v6.5L8 14.5L2 11.25v-6.5z",
        "M2 4.75L8 8L14 4.75M8 8v6.5",
    ],
);
/// The Cut list view: lines.
pub const CUT_LIST: Shape = stroke(1.5, &["M2.5 3.5h11M2.5 8h11M2.5 12.5h11"]);
/// The scene menu: a picture frame.
pub const SCENE: Shape = stroke(
    1.5,
    &[
        "M3.5 3h9a1.5 1.5 0 0 1 1.5 1.5v7a1.5 1.5 0 0 1-1.5 1.5h-9a1.5 1.5 0 0 1-1.5-1.5v-7a1.5 1.5 0 0 1 1.5-1.5z",
        "M2 6h12",
    ],
);
/// A small downward chevron (the scene menu).
pub const CHEVRON_DOWN: Shape = stroke(2.2, &["M4 6L8 10L12 6"]);
/// An open tree row's chevron.
pub const EXPANDED: Shape = stroke(2.0, &["M4 6L8 10L12 6"]);
/// A closed tree row's chevron.
pub const COLLAPSED: Shape = stroke(2.0, &["M6 4L10 8L6 12"]);
/// No problems: a tick.
pub const CHECK: Shape = stroke(1.8, &["M3.5 8.5L6.5 11.5L12.5 4.5"]);
/// Warnings only: an exclamation mark in a triangle, a different shape
/// from the errors' circle so the two differ without their colour.
pub const WARNING: Shape = stroke(1.6, &["M8 1.8L14.6 13.6H1.4z", "M8 6v3.6", "M8 11.4v0.8"]);
/// Errors: an exclamation mark in a circle.
pub const ALERT: Shape = stroke(
    1.8,
    &[
        "M14.5 8a6.5 6.5 0 1 1-13 0a6.5 6.5 0 1 1 13 0z",
        "M8 4.5v4.2",
        "M8 10.9v0.8",
    ],
);
/// Not checked yet: a dot in a circle.
pub const PENDING: Shape = stroke(1.6, &["M14 8a6 6 0 1 1-12 0a6 6 0 1 1 12 0z"]);
/// The main menu.
pub const MENU: Shape = stroke(1.5, &["M2.5 4h11M2.5 8h11M2.5 12h11"]);
/// The build plan: a hammer.
pub const HAMMER: Shape = Shape {
    view_box: 24.0,
    paint: Paint::Fill,
    paths: &[
        "M8 3h9a2.4 2.4 0 0 1 2.4 2.4V7a1.6 1.6 0 0 1-1.6 1.6H8.6c-2.1 0-3.7-1-4.2-2.4C5.2 4.3 6.4 3 8 3z",
        "M12 9.3a1.6 1.6 0 0 1 1.6 1.6v8.8a1.6 1.6 0 0 1-3.2 0v-8.8a1.6 1.6 0 0 1 1.6-1.6z",
    ],
    tilted: true,
};

/// Pull apart: arrows out from the middle.
pub const PULL_APART: Shape = stroke(
    1.5,
    &[
        "M8 1.5v4M8 10.5v4M1.5 8h4M10.5 8h4",
        "M6 3.5L8 1.5L10 3.5",
        "M6 12.5L8 14.5L10 12.5",
        "M3.5 6L1.5 8L3.5 10",
        "M12.5 6L14.5 8L12.5 10",
    ],
);
/// X-ray: a box drawn as if see-through, every edge showing.
pub const XRAY: Shape = stroke(
    1.5,
    &[
        "M2.5 5.5h8v8h-8z",
        "M5.5 2.5h8v8h-8z",
        "M2.5 5.5l3-3M10.5 5.5l3-3M10.5 13.5l3-3M2.5 13.5l3-3",
    ],
);
/// Play: a triangle.
pub const PLAY: Shape = Shape {
    view_box: 16.0,
    paint: Paint::Fill,
    paths: &["M4 2.5v11l9-5.5z"],
    tilted: false,
};
/// Pause: two bars.
pub const PAUSE: Shape = Shape {
    view_box: 16.0,
    paint: Paint::Fill,
    paths: &["M4 2.5h3v11h-3z", "M9 2.5h3v11h-3z"],
    tilted: false,
};
/// Fit to view: four corners.
pub const FIT: Shape = stroke(
    1.5,
    &[
        "M2 5.5V2H5.5",
        "M10.5 2H14V5.5",
        "M14 10.5V14H10.5",
        "M5.5 14H2V10.5",
    ],
);
/// Front view: a square face on.
pub const FRONT: Shape = stroke(
    1.5,
    &["M4 3h8a1 1 0 0 1 1 1v8a1 1 0 0 1-1 1h-8a1 1 0 0 1-1-1v-8a1 1 0 0 1 1-1z"],
);
/// Turn to the back: a circling arrow.
pub const TURN: Shape = stroke(1.5, &["M13 8a5 5 0 1 1-1.5-3.6", "M13 2.5v2.5h-2.5"]);
/// From above: a plan of a box in a corner.
pub const FROM_ABOVE: Shape = stroke(
    1.5,
    &[
        "M3.5 2h9a1.5 1.5 0 0 1 1.5 1.5v9a1.5 1.5 0 0 1-1.5 1.5h-9a1.5 1.5 0 0 1-1.5-1.5v-9a1.5 1.5 0 0 1 1.5-1.5z",
        "M2 10h7V2",
    ],
);
/// Close: a cross.
pub const CLOSE: Shape = stroke(2.0, &["M4 4L12 12M12 4L4 12"]);

mod imp {
    use super::*;

    pub struct PathIcon {
        pub shape: Cell<Shape>,
        pub size: Cell<i32>,
    }

    impl Default for PathIcon {
        fn default() -> Self {
            PathIcon {
                shape: Cell::new(MENU),
                size: Cell::new(16),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PathIcon {
        const NAME: &'static str = "StavePathIcon";
        type Type = super::PathIcon;
        type ParentType = gtk::Widget;

        fn class_init(class: &mut Self::Class) {
            class.set_css_name("staveicon");
            class.set_accessible_role(gtk::AccessibleRole::Img);
        }
    }

    impl ObjectImpl for PathIcon {}

    impl WidgetImpl for PathIcon {
        fn measure(&self, _orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let size = self.size.get();
            (size, size, -1, -1)
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let widget = self.obj();
            let shape = self.shape.get();
            let size = crate::convert::f32_of_int(self.size.get());
            let colour = widget.color();
            let scale = size / shape.view_box;
            // Centred in whatever space the widget was given.
            let left = (crate::convert::f32_of_int(widget.width()) - size) / 2.0;
            let top = (crate::convert::f32_of_int(widget.height()) - size) / 2.0;
            snapshot.save();
            snapshot.translate(&gtk::graphene::Point::new(left, top));
            snapshot.scale(scale, scale);
            if shape.tilted {
                let centre = shape.view_box / 2.0;
                snapshot.translate(&gtk::graphene::Point::new(centre, centre));
                snapshot.rotate(45.0);
                snapshot.scale(0.92, 0.92);
                snapshot.translate(&gtk::graphene::Point::new(-centre, -centre - 0.2));
            }
            for data in shape.paths {
                let Ok(path) = gtk::gsk::Path::parse(data) else {
                    continue;
                };
                match shape.paint {
                    Paint::Stroke(width) => {
                        let stroke = gtk::gsk::Stroke::new(width);
                        stroke.set_line_join(gtk::gsk::LineJoin::Round);
                        stroke.set_line_cap(gtk::gsk::LineCap::Round);
                        snapshot.append_stroke(&path, &stroke, &colour);
                    }
                    Paint::Fill => {
                        snapshot.append_fill(&path, gtk::gsk::FillRule::Winding, &colour);
                    }
                }
            }
            snapshot.restore();
        }
    }
}

glib::wrapper! {
    /// An icon drawn from SVG path data in the text colour.
    pub struct PathIcon(ObjectSubclass<imp::PathIcon>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl PathIcon {
    /// An icon of `shape`, `size` logical pixels square.
    pub fn new(shape: Shape, size: i32) -> PathIcon {
        let icon: PathIcon = glib::Object::builder()
            .property("valign", gtk::Align::Center)
            .property("halign", gtk::Align::Center)
            .build();
        icon.imp().shape.set(shape);
        icon.imp().size.set(size);
        icon
    }

    /// The shape it draws.
    pub fn shape(&self) -> Shape {
        self.imp().shape.get()
    }

    /// Draws another shape.
    pub fn set_shape(&self, shape: Shape) {
        if self.imp().shape.get() != shape {
            self.imp().shape.set(shape);
            self.queue_draw();
        }
    }
}

/// A coloured mark: a rounded square or a circle, `size` pixels.
pub fn swatch(colour: gtk::gdk::RGBA, round: bool, size: i32) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::builder()
        .content_width(size)
        .content_height(size)
        .valign(gtk::Align::Center)
        .build();
    area.set_draw_func(move |_, context, width, height| {
        let (w, h) = (f64::from(width), f64::from(height));
        let radius = if round { w.min(h) / 2.0 } else { 2.0 };
        context.new_sub_path();
        context.arc(
            w - radius,
            radius,
            radius,
            -std::f64::consts::FRAC_PI_2,
            0.0,
        );
        context.arc(
            w - radius,
            h - radius,
            radius,
            0.0,
            std::f64::consts::FRAC_PI_2,
        );
        context.arc(
            radius,
            h - radius,
            radius,
            std::f64::consts::FRAC_PI_2,
            std::f64::consts::PI,
        );
        context.arc(
            radius,
            radius,
            radius,
            std::f64::consts::PI,
            3.0 * std::f64::consts::FRAC_PI_2,
        );
        context.close_path();
        context.set_source_rgba(
            f64::from(colour.red()),
            f64::from(colour.green()),
            f64::from(colour.blue()),
            f64::from(colour.alpha()),
        );
        // A failed fill only leaves the mark blank.
        let _ = context.fill();
    });
    area
}
