//! The panels' headers: a slim strip of flush tabs, as in an IDE, each as
//! wide as its title (libadwaita's tab bar, which libpanel offers, draws
//! rounded tabs a fixed 220 px wide). A tab shows its panel, its × closes
//! it, dragging it docks the panel elsewhere (libpanel's drop targets take
//! a dragged `panel::Widget`), and the menu at the end holds the shown
//! panel's own menu ("Move to the Right", ...).

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gdk, glib};
use panel::prelude::*;
use panel::subclass::prelude::*;

mod imp {
    use std::cell::RefCell;

    use super::*;

    /// The strip's state.
    #[derive(Default)]
    pub struct TabStrip {
        /// The frame whose pages are the tabs.
        pub frame: glib::WeakRef<panel::Frame>,
        /// The tabs.
        pub tabs: gtk::Box,
        /// Widgets the frame adds at the end, then the menu.
        pub end: gtk::Box,
        /// The shown panel's menu.
        pub menu: gtk::MenuButton,
        /// Watching the frame's pages, to remake the tabs.
        pub watch: RefCell<Option<(gtk::SelectionModel, glib::SignalHandlerId)>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for TabStrip {
        const NAME: &'static str = "StaveTabStrip";
        type Type = super::TabStrip;
        type ParentType = gtk::Widget;
        type Interfaces = (panel::FrameHeader,);

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk::BoxLayout>();
            klass.set_css_name("stavetabstrip");
        }
    }

    impl ObjectImpl for TabStrip {
        fn properties() -> &'static [glib::ParamSpec] {
            use std::sync::OnceLock;
            static PROPERTIES: OnceLock<Vec<glib::ParamSpec>> = OnceLock::new();
            PROPERTIES.get_or_init(|| {
                vec![glib::ParamSpecOverride::for_interface::<panel::FrameHeader>("frame")]
            })
        }

        fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
            if pspec.name() == "frame" {
                let frame = value.get::<Option<panel::Frame>>().ok().flatten();
                self.obj().watch_frame(frame.as_ref());
            }
        }

        fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "frame" => self.frame.upgrade().to_value(),
                _ => None::<panel::Frame>.to_value(),
            }
        }

        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            self.tabs.set_hexpand(true);
            self.tabs.add_css_class("stave-tabs");
            self.end.add_css_class("stave-tab-end");
            self.menu.set_icon_name("pan-down-symbolic");
            self.menu.set_tooltip_text(Some("Move or close this panel"));
            self.menu.add_css_class("flat");
            self.menu.add_css_class("stave-tab-menu");
            self.end.append(&self.menu);
            self.tabs.set_parent(&*obj);
            self.end.set_parent(&*obj);
        }

        fn dispose(&self) {
            self.obj().watch_frame(None);
            self.tabs.unparent();
            self.end.unparent();
        }
    }

    impl WidgetImpl for TabStrip {}

    impl FrameHeaderImpl for TabStrip {
        fn page_changed(&self, _widget: Option<&panel::Widget>) {
            self.obj().refresh();
        }

        fn can_drop(&self, _widget: &panel::Widget) -> bool {
            true
        }

        fn add_prefix(&self, _priority: i32, widget: &panel::Widget) {
            self.end.prepend(widget.upcast_ref::<gtk::Widget>());
        }

        fn add_suffix(&self, _priority: i32, widget: &panel::Widget) {
            self.end
                .insert_child_after(widget.upcast_ref::<gtk::Widget>(), None::<&gtk::Widget>);
        }
    }
}

glib::wrapper! {
    /// A frame's header: flush tabs and the shown panel's menu.
    pub struct TabStrip(ObjectSubclass<imp::TabStrip>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, panel::FrameHeader;
}

impl Default for TabStrip {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl TabStrip {
    /// A strip, given to a frame with `Frame::set_header`.
    pub fn new() -> TabStrip {
        TabStrip::default()
    }

    /// Follows a frame's pages (or none).
    fn watch_frame(&self, frame: Option<&panel::Frame>) {
        let imp = self.imp();
        if let Some((pages, handler)) = imp.watch.take() {
            pages.disconnect(handler);
        }
        imp.frame.set(frame);
        if let Some(frame) = frame {
            let pages = frame.pages();
            let strip = self.downgrade();
            let handler = pages.connect_items_changed(move |_, _, _, _| {
                if let Some(strip) = strip.upgrade() {
                    strip.refresh();
                }
            });
            imp.watch.replace(Some((pages, handler)));
        }
        self.refresh();
    }

    /// Makes the tabs again: one per page, the shown one marked.
    fn refresh(&self) {
        let imp = self.imp();
        while let Some(child) = imp.tabs.first_child() {
            imp.tabs.remove(&child);
        }
        let Some(frame) = imp.frame.upgrade() else {
            imp.menu.set_visible(false);
            return;
        };
        let shown = frame.visible_child();
        for index in 0..frame.n_pages() {
            if let Some(page) = frame.page(index) {
                imp.tabs
                    .append(&tab(&frame, &page, shown.as_ref() == Some(&page)));
            }
        }
        let menu = shown.as_ref().and_then(|page| page.menu_model());
        imp.menu.set_visible(menu.is_some());
        imp.menu.set_menu_model(menu.as_ref());
    }
}

/// One tab: the panel's icon and title, a × to close it; clicking shows
/// it, dragging docks it elsewhere.
fn tab(frame: &panel::Frame, page: &panel::Widget, shown: bool) -> gtk::Box {
    let title = page.title().map(|t| t.to_string()).unwrap_or_default();
    let label = gtk::Label::builder()
        .label(&title)
        .css_classes(["stave-tab-title"])
        .build();
    let inner = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    if let Some(icon) = page.icon_name() {
        inner.append(&gtk::Image::from_icon_name(&icon));
    }
    inner.append(&label);
    let show = gtk::Button::builder()
        .child(&inner)
        .css_classes(["flat", "stave-tab-show"])
        .tooltip_text(&title)
        .build();
    let frame_weak = frame.downgrade();
    let page_weak = page.downgrade();
    show.connect_clicked(move |_| {
        if let (Some(frame), Some(page)) = (frame_weak.upgrade(), page_weak.upgrade()) {
            frame.set_visible_child(&page);
        }
    });
    let close = gtk::Button::builder()
        .icon_name("window-close-symbolic")
        .css_classes(["flat", "stave-tab-close"])
        .tooltip_text(format!("Close {title}"))
        .build();
    let page_weak = page.downgrade();
    close.connect_clicked(move |_| {
        if let Some(page) = page_weak.upgrade() {
            page.close();
        }
    });
    let root = gtk::Box::builder()
        .css_classes(if shown {
            vec!["stave-tab", "shown"]
        } else {
            vec!["stave-tab"]
        })
        .build();
    root.append(&show);
    root.append(&close);
    // Dragging a tab carries its panel; libpanel's drop targets (the
    // edges, the middle, other frames) take it.
    let drag = gtk::DragSource::builder()
        .actions(gdk::DragAction::MOVE)
        .build();
    let page_weak = page.downgrade();
    drag.connect_prepare(move |_, _, _| {
        let page = page_weak.upgrade()?;
        Some(gdk::ContentProvider::for_value(&page.to_value()))
    });
    let root_weak = root.downgrade();
    drag.connect_drag_begin(move |source, _| {
        if let Some(root) = root_weak.upgrade() {
            source.set_icon(Some(&gtk::WidgetPaintable::new(Some(&root))), 0, 0);
        }
    });
    root.add_controller(drag);
    root
}
