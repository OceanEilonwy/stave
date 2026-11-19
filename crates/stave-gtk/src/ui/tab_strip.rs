//! The panels' headers: a slim strip of flush tabs, as in an IDE, each as
//! wide as its title (libadwaita's tab bar, which libpanel offers, draws
//! rounded tabs a fixed 220 px wide). A tab shows its panel, its × closes
//! it, dragging it docks the panel elsewhere (libpanel's drop targets take
//! a dragged `panel::Widget`), and the menu at the end holds the shown
//! panel's own menu ("Move to the Right", ...).
//!
//! For the keyboard and screen readers: the strip is a tab list and each
//! tab a tab, the shown one selected; showing another page restyles the
//! tabs in place, so the tab with the keyboard keeps it; one Tab stop per
//! tab (the × is for the pointer; the menu closes a panel); and
//! Ctrl+Page Up / Ctrl+Page Down show the previous and next tab while the
//! keyboard is anywhere in the frame.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gdk, glib};
use panel::prelude::*;
use panel::subclass::prelude::*;

mod imp {
    use std::cell::RefCell;

    use super::*;

    /// The strip's state.
    pub struct TabStrip {
        /// The frame whose pages are the tabs.
        pub frame: glib::WeakRef<panel::Frame>,
        /// The tabs' box: a tab list.
        pub tabs: gtk::Box,
        /// One tab per page, in the frame's order.
        pub shown_tabs: RefCell<Vec<super::Tab>>,
        /// Widgets the frame adds at the end, then the menu.
        pub end: gtk::Box,
        /// The shown panel's menu.
        pub menu: gtk::MenuButton,
        /// Watching the frame's pages, to remake the tabs.
        pub watch: RefCell<Option<(gtk::SelectionModel, glib::SignalHandlerId)>>,
        /// Ctrl+Page Up / Down on the frame, and the frame it is on.
        pub keys: RefCell<Option<(glib::WeakRef<panel::Frame>, gtk::ShortcutController)>>,
    }

    impl Default for TabStrip {
        fn default() -> Self {
            TabStrip {
                frame: glib::WeakRef::new(),
                tabs: gtk::Box::builder()
                    .accessible_role(gtk::AccessibleRole::TabList)
                    .build(),
                shown_tabs: RefCell::new(Vec::new()),
                end: gtk::Box::default(),
                menu: gtk::MenuButton::default(),
                watch: RefCell::new(None),
                keys: RefCell::new(None),
            }
        }
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
            self.obj().mark_shown();
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
        if let Some((old, keys)) = imp.keys.take()
            && let Some(old) = old.upgrade()
        {
            old.remove_controller(&keys);
        }
        imp.frame.set(frame);
        if let Some(frame) = frame {
            let keys = step_keys();
            frame.add_controller(keys.clone());
            imp.keys.replace(Some((frame.downgrade(), keys)));
        }
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

    /// Makes the tabs again (pages came or went): one per page, the
    /// shown one marked.
    fn refresh(&self) {
        let imp = self.imp();
        while let Some(child) = imp.tabs.first_child() {
            imp.tabs.remove(&child);
        }
        let mut tabs = Vec::new();
        if let Some(frame) = imp.frame.upgrade() {
            for index in 0..frame.n_pages() {
                if let Some(page) = frame.page(index) {
                    let made = tab(&frame, &page);
                    imp.tabs.append(&made.root);
                    tabs.push(made);
                }
            }
        }
        imp.shown_tabs.replace(tabs);
        self.mark_shown();
    }

    /// Marks the shown page's tab, in place: the tab the keyboard is on
    /// stays where it is.
    fn mark_shown(&self) {
        let imp = self.imp();
        let Some(frame) = imp.frame.upgrade() else {
            imp.menu.set_visible(false);
            return;
        };
        let shown = frame.visible_child();
        for tab in imp.shown_tabs.borrow().iter() {
            tab.mark(
                tab.page
                    .upgrade()
                    .is_some_and(|page| shown.as_ref() == Some(&page)),
            );
        }
        let menu = shown.as_ref().and_then(|page| page.menu_model());
        imp.menu.set_visible(menu.is_some());
        imp.menu.set_menu_model(menu.as_ref());
    }
}

/// One page's tab.
pub struct Tab {
    /// The page it shows.
    page: glib::WeakRef<panel::Widget>,
    /// The tab: the show button and the ×.
    root: gtk::Box,
    /// What the keyboard and a screen reader take for the tab.
    show: gtk::Button,
}

impl Tab {
    /// Marks the tab shown or not: its look and its selected state.
    fn mark(&self, shown: bool) {
        if shown {
            self.root.add_css_class("shown");
        } else {
            self.root.remove_css_class("shown");
        }
        self.show
            .update_state(&[gtk::accessible::State::Selected(Some(shown))]);
    }
}

/// Ctrl+Page Up and Ctrl+Page Down: the previous and the next tab of the
/// frame the keyboard is in. After the focused widget had its turn (the
/// text editor keeps its own Ctrl+Page keys).
fn step_keys() -> gtk::ShortcutController {
    let keys = gtk::ShortcutController::new();
    for (trigger, step) in [("<Control>Page_Up", -1), ("<Control>Page_Down", 1)] {
        let action = gtk::CallbackAction::new(move |widget, _| {
            match widget.downcast_ref::<panel::Frame>() {
                Some(frame) if show_step(frame, step) => glib::Propagation::Stop,
                _ => glib::Propagation::Proceed,
            }
        });
        keys.add_shortcut(gtk::Shortcut::new(
            gtk::ShortcutTrigger::parse_string(trigger),
            Some(action),
        ));
    }
    keys
}

/// Shows the page `step` places after the shown one (before it when
/// negative), going round. `false` when there is only one page or none.
fn show_step(frame: &panel::Frame, step: i32) -> bool {
    let count = frame.n_pages();
    if count < 2 {
        return false;
    }
    let shown = frame.visible_child();
    let current = (0..count)
        .find(|index| {
            frame
                .page(*index)
                .is_some_and(|page| shown.as_ref() == Some(&page))
        })
        .unwrap_or(0);
    let next = if step < 0 {
        current.checked_sub(1).unwrap_or(count - 1)
    } else {
        current.saturating_add(1) % count
    };
    let Some(page) = frame.page(next) else {
        return false;
    };
    frame.set_visible_child(&page);
    true
}

/// One tab: the panel's icon and title, a × to close it; clicking shows
/// it, dragging docks it elsewhere.
fn tab(frame: &panel::Frame, page: &panel::Widget) -> Tab {
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
        .accessible_role(gtk::AccessibleRole::Tab)
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
        .focusable(false)
        .build();
    close.update_property(&[gtk::accessible::Property::Label(&format!("Close {title}"))]);
    let page_weak = page.downgrade();
    close.connect_clicked(move |_| {
        if let Some(page) = page_weak.upgrade() {
            page.close();
        }
    });
    let root = gtk::Box::builder().css_classes(["stave-tab"]).build();
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
    Tab {
        page: page.downgrade(),
        root,
        show,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_stay_put_when_another_page_shows() {
        crate::ui::gtk_test::on_gtk_thread(tabs_stay_put);
    }

    /// Showing another page restyles the tabs in place (the focused tab
    /// survives), and Ctrl+Page Down / Up step through the pages, round.
    fn tabs_stay_put() {
        panel::init();
        let frame = panel::Frame::new();
        let strip = TabStrip::new();
        frame.set_header(Some(&strip));
        let pages: Vec<panel::Widget> = ["Parts", "Problems", "Details"]
            .into_iter()
            .map(|title| {
                let page = panel::Widget::builder().title(title).build();
                page.set_child(Some(&gtk::Label::new(Some(title))));
                frame.add(&page);
                page
            })
            .collect();
        let window = gtk::Window::builder().child(&frame).build();
        window.present();
        let context = glib::MainContext::default();
        for _ in 0..20 {
            context.iteration(false);
        }
        let roots = || -> Vec<gtk::Widget> {
            let tabs = &strip.imp().tabs;
            let mut roots = Vec::new();
            let mut child = tabs.first_child();
            while let Some(widget) = child {
                child = widget.next_sibling();
                roots.push(widget);
            }
            roots
        };
        let before = roots();
        assert_eq!(before.len(), 3);
        assert_eq!(
            strip.imp().tabs.accessible_role(),
            gtk::AccessibleRole::TabList
        );
        let show = |index: usize| strip.imp().shown_tabs.borrow()[index].show.clone();
        assert_eq!(show(0).accessible_role(), gtk::AccessibleRole::Tab);
        let focus = || gtk::prelude::GtkWindowExt::focus(&window);
        assert!(show(1).grab_focus());
        assert_eq!(focus().as_ref(), Some(show(1).upcast_ref()));
        frame.set_visible_child(&pages[1]);
        assert_eq!(roots(), before, "the tabs were made again");
        assert_eq!(
            focus().as_ref(),
            Some(show(1).upcast_ref()),
            "the tab lost the keyboard"
        );
        let shown: Vec<bool> = before
            .iter()
            .map(|tab| tab.has_css_class("shown"))
            .collect();
        assert_eq!(shown, [false, true, false]);
        // Ctrl+Page Down, then round from the last to the first.
        assert!(show_step(&frame, 1));
        assert_eq!(frame.visible_child().as_ref(), Some(&pages[2]));
        assert!(show_step(&frame, 1));
        assert_eq!(frame.visible_child().as_ref(), Some(&pages[0]));
        assert!(show_step(&frame, -1));
        assert_eq!(frame.visible_child().as_ref(), Some(&pages[2]));
        // The × is not a Tab stop.
        let close = before[0].last_child().unwrap();
        assert!(!close.is_focusable());
        window.destroy();
    }
}
