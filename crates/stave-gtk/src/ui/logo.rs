//! Stave's logo, the dovetail cube (the application icon), drawn from its
//! own path data in its own colours. Drawing it, not loading an image,
//! means it shows the same whether or not Stave is installed (no icon
//! theme to find it in) and on every system: GTK hands PNG and SVG
//! decoding to image loaders that a desktop session may lack or fail to
//! start, which drew the icon theme's "missing image" in its place.
//!
//! The faces are those of
//! `data/icons/hicolor/scalable/apps/io.github.OceanEilonwy.Stave.svg`
//! (its 120-unit view box), without the ground shadow: change both
//! together.

use std::cell::Cell;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

/// The logo's view box side.
const VIEW_BOX: f32 = 120.0;

/// The cube's faces, back to front: colour (as `#rrggbb`) and SVG path.
pub const FACES: [(&str, &str); 5] = [
    (
        "#7a5134",
        "M44.80 81.12 L51.56 64.22 L23.41 47.98 L30.17 72.67 L14.97 63.90 L14.97 86.00 L60.00 112.00 L60.00 89.90 Z",
    ),
    (
        "#573722",
        "M60.00 112.00 L105.03 86.00 L105.03 56.10 L89.83 64.88 L96.59 81.78 L68.44 98.03 L75.20 73.33 L60.00 82.10 Z",
    ),
    (
        "#edb866",
        "M14.97 63.90 L30.17 72.67 L23.41 47.98 L51.56 64.22 L44.80 81.12 L60.00 89.90 L60.00 60.00 L14.97 34.00 Z",
    ),
    (
        "#cf9447",
        "M60.00 60.00 L60.00 82.10 L75.20 73.33 L68.44 98.03 L96.59 81.78 L89.83 64.88 L105.03 56.10 L105.03 34.00 Z",
    ),
    (
        "#f6d79a",
        "M14.97 34.00 L60.00 60.00 L105.03 34.00 L60.00 8.00 Z",
    ),
];

mod imp {
    use super::*;

    pub struct Logo {
        pub size: Cell<i32>,
    }

    impl Default for Logo {
        fn default() -> Self {
            Logo {
                size: Cell::new(20),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Logo {
        const NAME: &'static str = "StaveLogo";
        type Type = super::Logo;
        type ParentType = gtk::Widget;

        fn class_init(class: &mut Self::Class) {
            class.set_css_name("stavelogo");
            // Decoration: the window's title names the project.
            class.set_accessible_role(gtk::AccessibleRole::Presentation);
        }
    }

    impl ObjectImpl for Logo {}

    impl WidgetImpl for Logo {
        fn measure(&self, _orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let size = self.size.get();
            (size, size, -1, -1)
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let widget = self.obj();
            let size = crate::convert::f32_of_int(self.size.get());
            let left = (crate::convert::f32_of_int(widget.width()) - size) / 2.0;
            let top = (crate::convert::f32_of_int(widget.height()) - size) / 2.0;
            snapshot.save();
            snapshot.translate(&gtk::graphene::Point::new(left, top));
            snapshot.scale(size / VIEW_BOX, size / VIEW_BOX);
            for (colour, data) in FACES {
                let (Ok(colour), Ok(path)) =
                    (gtk::gdk::RGBA::parse(colour), gtk::gsk::Path::parse(data))
                else {
                    continue;
                };
                snapshot.append_fill(&path, gtk::gsk::FillRule::Winding, &colour);
            }
            snapshot.restore();
        }
    }
}

glib::wrapper! {
    /// Stave's logo, the dovetail cube, drawn in its own colours.
    pub struct Logo(ObjectSubclass<imp::Logo>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Logo {
    /// The logo, `size` logical pixels square.
    pub fn new(size: i32) -> Logo {
        let logo: Logo = glib::Object::builder()
            .property("valign", gtk::Align::Center)
            .property("halign", gtk::Align::Center)
            .build();
        logo.imp().size.set(size);
        logo
    }

    /// Its side, in logical pixels.
    pub fn size(&self) -> i32 {
        self.imp().size.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SVG: &str = include_str!(
        "../../../../data/icons/hicolor/scalable/apps/io.github.OceanEilonwy.Stave.svg"
    );

    #[test]
    fn the_logo_draws_the_application_icons_faces() {
        for (colour, data) in FACES {
            assert!(
                SVG.contains(&format!("fill=\"{colour}\" d=\"{data}\"")),
                "{colour} face differs from the icon"
            );
        }
    }

    #[test]
    fn every_face_parses() {
        crate::ui::gtk_test::on_gtk_thread(|| {
            for (colour, data) in FACES {
                assert!(gtk::gdk::RGBA::parse(colour).is_ok(), "{colour}");
                assert!(gtk::gsk::Path::parse(data).is_ok(), "{data}");
            }
        });
    }
}
