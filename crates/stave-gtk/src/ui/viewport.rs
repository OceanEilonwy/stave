//! The 3D view widget: shows the last rendered frame as a texture and
//! reports its size in device pixels when it changes. Frames are rendered
//! at that size, so they are drawn one texel to a device pixel, sharp on a
//! HiDPI screen too.

use std::cell::{Cell, RefCell};

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
        pub sizing: Cell<super::Sizing>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Viewport {
        const NAME: &'static str = "StaveViewport";
        type Type = super::Viewport;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Viewport {
        fn constructed(&self) {
            self.parent_constructed();
            // A window moved to a screen of another scale keeps its
            // logical size: the device size changes all the same.
            self.obj()
                .connect_scale_factor_notify(|viewport| viewport.imp().report_size());
        }
    }

    impl Viewport {
        fn report_size(&self) {
            let (width, height) = self.obj().device_size();
            if let Some(handler) = self.on_resize.borrow().as_ref() {
                handler(width, height);
            }
        }
    }

    impl WidgetImpl for Viewport {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            match self.sizing.get() {
                super::Sizing::View => gtk::SizeRequestMode::ConstantSize,
                super::Sizing::Picture { .. } => gtk::SizeRequestMode::HeightForWidth,
            }
        }

        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            match self.sizing.get() {
                super::Sizing::View => (120, 640, -1, -1),
                super::Sizing::Picture {
                    natural_width,
                    minimum_height,
                } => match orientation {
                    gtk::Orientation::Vertical => {
                        // 16:9 at the width given, but never lower than
                        // the minimum.
                        let width = if for_size > 0 {
                            for_size
                        } else {
                            natural_width
                        };
                        let height = width.saturating_mul(9) / 16;
                        (minimum_height, height.max(minimum_height), -1, -1)
                    }
                    _ => (0, natural_width, -1, -1),
                },
            }
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            self.parent_size_allocate(width, height, baseline);
            self.report_size();
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let widget = self.obj();
            let bounds = gtk::graphene::Rect::new(
                0.0,
                0.0,
                crate::convert::f32_of_int(widget.width()),
                crate::convert::f32_of_int(widget.height()),
            );
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

/// How a [`Viewport`] asks for room.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sizing {
    /// The 3D view: whatever room there is.
    #[default]
    View,
    /// A small picture: as wide as its row gives it, 16:9 at that width
    /// but at least `minimum_height` logical pixels high.
    Picture {
        /// The width it would like, in logical pixels.
        natural_width: i32,
        /// The least height, in logical pixels.
        minimum_height: i32,
    },
}

impl Viewport {
    /// A small picture sized as `sizing` says, not expanding.
    pub fn picture(sizing: Sizing) -> Viewport {
        let viewport: Viewport = glib::Object::builder()
            .property("hexpand", true)
            .property("vexpand", false)
            .property("can-target", false)
            .build();
        viewport.imp().sizing.set(sizing);
        viewport.queue_resize();
        viewport
    }

    /// The display's scale (2 on a HiDPI screen, maybe 1.25 or 1.5).
    pub fn scale(&self) -> f64 {
        self.native()
            .and_then(|native| native.surface())
            .map_or_else(|| f64::from(self.scale_factor()), |surface| surface.scale())
    }

    /// The widget's size in device pixels.
    pub fn device_size(&self) -> (u32, u32) {
        let scale = self.scale();
        let pixels =
            |logical: i32| -> u32 { crate::convert::pixels(f64::from(logical.max(1)) * scale) };
        (pixels(self.width()), pixels(self.height()))
    }

    /// Shows a frame.
    pub fn show_frame(&self, frame: gtk::gdk::Texture) {
        self.imp().frame.replace(Some(frame));
        self.queue_draw();
    }

    /// Shows a frame of the live view straight from the GPU's mapped
    /// buffer: GTK reads the padded rows where they are, so they are not
    /// copied on the CPU, and the buffer goes back to the renderer when the
    /// next frame replaces this one. False when its size can't be a
    /// texture's.
    pub fn show_live(&self, frame: crate::render::LiveFrame) -> bool {
        let (Ok(width), Ok(height), Ok(stride)) = (
            i32::try_from(frame.width()),
            i32::try_from(frame.height()),
            usize::try_from(frame.stride()),
        ) else {
            return false;
        };
        let bytes = glib::Bytes::from_owned(frame);
        // Alpha is 1 everywhere, so the bytes are premultiplied already:
        // the format GTK draws without converting.
        let texture = gtk::gdk::MemoryTexture::new(
            width,
            height,
            gtk::gdk::MemoryFormat::R8g8b8a8Premultiplied,
            &bytes,
            stride,
        );
        self.show_frame(texture.upcast());
        true
    }

    /// Calls `handler` with the size in device pixels whenever it may have
    /// changed (on every allocation and change of scale).
    pub fn connect_resized(&self, handler: impl Fn(u32, u32) + 'static) {
        self.imp().on_resize.replace(Some(Box::new(handler)));
    }
}
