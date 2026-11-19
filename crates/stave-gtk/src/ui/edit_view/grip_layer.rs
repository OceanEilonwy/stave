//! The layer the grips' handles and labels sit on over the 3D view: a
//! `gtk::Fixed` that takes no pointer events itself, so a click between
//! its children reaches the 3D view under it, while its children (the
//! handles, the numbers) take theirs. It lies under the window's floating
//! cards, so they are never covered by a grip.

use gtk::glib;
use gtk::subclass::prelude::*;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct GripLayer;

    #[glib::object_subclass]
    impl ObjectSubclass for GripLayer {
        const NAME: &'static str = "StaveGripLayer";
        type Type = super::GripLayer;
        type ParentType = gtk::Fixed;
    }

    impl ObjectImpl for GripLayer {}

    impl WidgetImpl for GripLayer {
        /// Only its children are under the pointer, never the layer.
        fn contains(&self, _x: f64, _y: f64) -> bool {
            false
        }
    }

    impl FixedImpl for GripLayer {}
}

glib::wrapper! {
    /// A `gtk::Fixed` the pointer passes through between its children.
    pub struct GripLayer(ObjectSubclass<imp::GripLayer>)
        @extends gtk::Fixed, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl GripLayer {
    /// An empty layer.
    pub fn new() -> GripLayer {
        glib::Object::new()
    }
}

impl Default for GripLayer {
    fn default() -> Self {
        GripLayer::new()
    }
}
