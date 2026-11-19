//! The 3D view widget: shows the last rendered frame as a texture and
//! reports its size in device pixels when it changes. Frames are rendered
//! at that size, so they are drawn one texel to a device pixel, sharp on a
//! HiDPI screen too.
//!
//! While the 3D view is being resized (a window dragged larger, a panel
//! slid open) it shows the last frame stretched, and reports the new size
//! once it has held still for [`SETTLE`], so the scene isn't drawn again at
//! every pixel of the drag.
//!
//! It is a GObject subclass (it draws in `snapshot` and sizes itself in
//! `measure`), so it stays a widget rather than a relm4 component; its
//! state is in [`Cell`]s, and nothing of it is borrowed while it calls the
//! resize handler, so the handler may call back into it.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

/// How long the 3D view's size must hold still before it is reported.
const SETTLE: Duration = Duration::from_millis(100);

mod imp {
    use super::*;

    /// Called with the new size in device pixels.
    pub type ResizeHandler = Rc<dyn Fn(u32, u32)>;

    #[derive(Default)]
    pub struct Viewport {
        /// The frame shown.
        pub frame: Cell<Option<gtk::gdk::Texture>>,
        /// Called with the new size in device pixels.
        pub on_resize: Cell<Option<ResizeHandler>>,
        /// How it asks for room.
        pub sizing: Cell<super::Sizing>,
        /// The size last reported, in device pixels.
        pub reported: Cell<(u32, u32)>,
        /// A report waiting for the size to hold still.
        pub settling: Cell<Option<glib::SourceId>>,
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
            self.obj().connect_scale_factor_notify(|viewport| {
                let imp = viewport.imp();
                imp.stop_settling();
                imp.report_size();
            });
        }
    }

    impl Viewport {
        /// The frame shown, copied out of its cell.
        pub(super) fn frame(&self) -> Option<gtk::gdk::Texture> {
            let frame = self.frame.take();
            self.frame.set(frame.clone());
            frame
        }

        /// Tells the handler the size in device pixels.
        pub(super) fn report_size(&self) {
            let (width, height) = self.obj().device_size();
            self.reported.set((width, height));
            // Cloned out so that the handler may set a new one, or come
            // back here through a synchronous allocation.
            let handler = self.on_resize.take();
            self.on_resize.set(handler.clone());
            if let Some(handler) = handler {
                handler(width, height);
            }
        }

        /// Drops a report waiting for the size to hold still.
        fn stop_settling(&self) {
            if let Some(source) = self.settling.take() {
                source.remove();
            }
        }

        /// Reports a new size: at once for the first, for a picture, or
        /// before anything is shown; for the 3D view showing a frame, once
        /// the size has held still for [`super::SETTLE`].
        fn size_changed(&self) {
            let size = self.obj().device_size();
            if size == self.reported.get() {
                // Back where it was: nothing to draw again.
                self.stop_settling();
                return;
            }
            let showing = self.frame().is_some() && self.reported.get().0 > 1;
            let settles = showing && self.sizing.get() == super::Sizing::View;
            self.stop_settling();
            if !settles {
                self.report_size();
                return;
            }
            let viewport = self.obj().downgrade();
            let source = glib::timeout_add_local_once(super::SETTLE, move || {
                if let Some(viewport) = viewport.upgrade() {
                    let imp = viewport.imp();
                    // It has run: there is nothing left to remove.
                    imp.settling.take();
                    imp.report_size();
                }
            });
            self.settling.set(Some(source));
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
            self.size_changed();
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let widget = self.obj();
            let bounds = gtk::graphene::Rect::new(
                0.0,
                0.0,
                crate::convert::f32_of_int(widget.width()),
                crate::convert::f32_of_int(widget.height()),
            );
            if let Some(frame) = self.frame() {
                snapshot.append_texture(&frame, &bounds);
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
        self.imp().frame.set(Some(frame));
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

    /// Calls `handler` with the size in device pixels when it changes: at
    /// once on a change of scale or the first allocation, and once a resize
    /// of the 3D view has held still for [`SETTLE`].
    pub fn connect_resized(&self, handler: impl Fn(u32, u32) + 'static) {
        self.imp().on_resize.set(Some(Rc::new(handler)));
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::ui::gtk_test::{on_gtk_thread, settle};

    #[test]
    fn the_resize_handler_may_call_back_into_the_viewport() {
        on_gtk_thread(handler_calls_back);
    }

    /// The handler runs with nothing of the viewport borrowed: it may
    /// show a frame, ask the size and connect a new handler from inside.
    fn handler_calls_back() {
        let viewport = Viewport::default();
        let heard: Rc<RefCell<Vec<(u32, u32)>>> = Rc::default();
        let log = Rc::clone(&heard);
        let weak = viewport.downgrade();
        viewport.connect_resized(move |width, height| {
            log.borrow_mut().push((width, height));
            let Some(viewport) = weak.upgrade() else {
                return;
            };
            let bytes = glib::Bytes::from_owned(vec![0_u8; 4]);
            let texture =
                gtk::gdk::MemoryTexture::new(1, 1, gtk::gdk::MemoryFormat::R8g8b8a8, &bytes, 4);
            viewport.show_frame(texture.upcast());
            let _ = viewport.device_size();
            let again = Rc::clone(&log);
            viewport.connect_resized(move |width, height| {
                again.borrow_mut().push((width + 1000, height));
            });
        });
        viewport.imp().report_size();
        viewport.imp().report_size();
        // Unallocated, it is a logical pixel square, at the screen's scale.
        let (width, height) = viewport.device_size();
        assert_eq!(
            *heard.borrow(),
            vec![(width, height), (width + 1000, height)],
            "the second report reached the handler set from inside the first"
        );
        assert!(viewport.imp().frame().is_some(), "the frame is shown");
    }

    #[test]
    fn a_dropped_viewport_frees_its_handler() {
        on_gtk_thread(viewport_is_freed);
    }

    fn viewport_is_freed() {
        let viewport = Viewport::picture(Sizing::Picture {
            natural_width: 320,
            minimum_height: 110,
        });
        let token = Rc::new(());
        let held = Rc::clone(&token);
        viewport.connect_resized(move |_, _| {
            let _ = &held;
        });
        let handler = Rc::downgrade(&token);
        drop(token);
        let weak = viewport.downgrade();
        drop(viewport);
        settle();
        assert!(weak.upgrade().is_none(), "the viewport is freed");
        assert!(handler.upgrade().is_none(), "its handler is freed");
    }
}
