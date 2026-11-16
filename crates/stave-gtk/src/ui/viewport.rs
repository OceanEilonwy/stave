//! The 3D view widget: shows the last rendered frame as a texture and
//! reports its size in device pixels when it changes.

use std::cell::RefCell;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

mod imp {
    use super::*;

    /// Called with the new size in device pixels.
    pub type ResizeHandler = Box<dyn Fn(u32, u32)>;

    #[derive(Default)]
    pub struct Viewport {
        pub frame: RefCell<Option<gtk::gdk::Texture>>,
        pub on_resize: RefCell<Option<ResizeHandler>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Viewport {
        const NAME: &'static str = "StaveViewport";
        type Type = super::Viewport;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Viewport {}

    impl WidgetImpl for Viewport {
        fn measure(&self, _orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            (120, 640, -1, -1)
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            self.parent_size_allocate(width, height, baseline);
            let (width, height) = self.obj().device_size();
            if let Some(handler) = self.on_resize.borrow().as_ref() {
                handler(width, height);
            }
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let widget = self.obj();
            let bounds =
                gtk::graphene::Rect::new(0.0, 0.0, widget.width() as f32, widget.height() as f32);
            if let Some(frame) = self.frame.borrow().as_ref() {
                snapshot.append_texture(frame, &bounds);
            }
        }
    }
}

glib::wrapper! {
    /// Shows rendered frames.
    pub struct Viewport(ObjectSubclass<imp::Viewport>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for Viewport {
    fn default() -> Self {
        glib::Object::builder()
            .property("hexpand", true)
            .property("vexpand", true)
            .property("focusable", true)
            .build()
    }
}

impl Viewport {
    /// The display's scale (2 on a HiDPI screen, maybe 1.25 or 1.5).
    pub fn scale(&self) -> f64 {
        self.native()
            .and_then(|native| native.surface())
            .map_or_else(|| f64::from(self.scale_factor()), |surface| surface.scale())
    }

    /// The widget's size in device pixels.
    pub fn device_size(&self) -> (u32, u32) {
        let scale = self.scale();
        let pixels = |logical: i32| -> u32 {
            let value = (f64::from(logical.max(1)) * scale).round();
            // Widget sizes are small positive numbers.
            value.clamp(1.0, 16384.0) as u32
        };
        (pixels(self.width()), pixels(self.height()))
    }

    /// Shows a frame.
    pub fn show_frame(&self, frame: gtk::gdk::Texture) {
        self.imp().frame.replace(Some(frame));
        self.queue_draw();
    }

    /// Calls `handler` with the size in device pixels whenever it changes.
    pub fn connect_resized(&self, handler: impl Fn(u32, u32) + 'static) {
        self.imp().on_resize.replace(Some(Box::new(handler)));
    }
}
