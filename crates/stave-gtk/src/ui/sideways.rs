//! A widget that shows its child turned a quarter turn clockwise, so a
//! label reads from top to bottom: the open files' tabs on the rail at the
//! window's right edge. GTK 4 labels can't be turned themselves; the child
//! is measured with width and height swapped and allocated with a
//! rotation.

use std::cell::RefCell;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Sideways {
        pub child: RefCell<Option<gtk::Widget>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Sideways {
        const NAME: &'static str = "StaveSideways";
        type Type = super::Sideways;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Sideways {
        fn dispose(&self) {
            if let Some(child) = self.child.take() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for Sideways {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::ConstantSize
        }

        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let Some(child) = self.child.borrow().clone() else {
                return (0, 0, -1, -1);
            };
            // Turned, the child's width is our height and its height our
            // width.
            let across = match orientation {
                gtk::Orientation::Horizontal => gtk::Orientation::Vertical,
                _ => gtk::Orientation::Horizontal,
            };
            let (minimum, natural, _, _) = child.measure(across, -1);
            (minimum, natural, -1, -1)
        }

        fn size_allocate(&self, width: i32, height: i32, _baseline: i32) {
            let Some(child) = self.child.borrow().clone() else {
                return;
            };
            let transform = gtk::gsk::Transform::new()
                .translate(&gtk::graphene::Point::new(
                    crate::convert::f32_of_int(width),
                    0.0,
                ))
                .rotate(90.0);
            child.allocate(height, width, -1, Some(transform));
        }
    }
}

glib::wrapper! {
    /// Shows its child turned a quarter turn clockwise.
    pub struct Sideways(ObjectSubclass<imp::Sideways>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Sideways {
    /// `child`, turned to read from top to bottom.
    pub fn new(child: &impl IsA<gtk::Widget>) -> Sideways {
        let sideways: Sideways = glib::Object::new();
        child.set_parent(&sideways);
        sideways
            .imp()
            .child
            .replace(Some(child.clone().upcast::<gtk::Widget>()));
        sideways
    }
}
