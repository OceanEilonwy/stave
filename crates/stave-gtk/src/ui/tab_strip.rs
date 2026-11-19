//! The panels' headers: a slim strip of flush tabs, as in an IDE, each as
//! wide as its title (libadwaita's tab bar, which libpanel offers, draws
//! rounded tabs a fixed 220 px wide). A tab shows its panel, its × closes
//! it, dragging it docks the panel elsewhere (libpanel's drop targets take
//! a dragged `panel::Widget`), and the menu at the end holds the shown
//! panel's own menu ("Move to the Right", ...).
//!
//! A panel can show its own tabs in place of its one tab
//! ([`set_page_tabs`], given that window's panel widget, so each window's
//! panel shows its own): the Code panel's open files appear this way, one
//! tab each (name, and a yellow ● while unsaved), beside the Model's tab
//! in the frame they share. Clicking one shows the panel on that file.
//!
//! The strip is a GObject subclass, [`TabStrip`], because libpanel takes a
//! frame's header as a widget implementing `PanelFrameHeader`. Its tabs
//! are a relm4 component, [`TabStripComponent`] (docs/relm4-migration.md):
//! the component's model is the only state, the tabs' buttons only send
//! [`TabStripInput`] messages, and showing one of a panel's own tabs is a
//! [`TabStripOutput`]. The subclass owns the component and keeps the
//! older API ([`set_page_tabs`], [`page_tabs_changed`]) in front of it:
//! it gathers the tabs, from the frame's pages and the panels' own tabs,
//! when the main loop next runs, never inside whatever asked for them,
//! and sends them as [`TabStripInput::SetTabs`].

use std::cell::{Cell, OnceCell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gdk, glib};
use panel::prelude::*;
use panel::subclass::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender};

use super::icons::{self, PathIcon};
use super::relm4_shim::Bridge;

/// The icon name that gives a tab the Model's cube.
pub const MODEL_ICON: &str = "stave-model";

/// One of a panel's own tabs.
pub struct PageTab {
    /// The tab's words.
    pub title: String,
    /// Its tooltip.
    pub tooltip: String,
    /// It has unsaved changes: a yellow ● after the words.
    pub unsaved: bool,
    /// The worst problem in it, as a dot before the words.
    pub mark: Option<stave_app::problems::Mark>,
    /// It is the panel's current one.
    pub active: bool,
    /// Shows it (the panel is brought to the front first).
    pub show: Rc<dyn Fn()>,
}

type PageTabs = Rc<dyn Fn() -> Vec<PageTab>>;

thread_local! {
    /// Each panel's own tabs, by the panel widget itself: every window has
    /// its own Code panel, so an id ("code") would let the last window
    /// opened answer for all of them. A closed window's entry is pruned.
    /// Borrowed only to add an entry or clone one out, never across a
    /// call.
    static PAGE_TABS: RefCell<Vec<(glib::WeakRef<panel::Widget>, PageTabs)>> =
        const { RefCell::new(Vec::new()) };
    /// Every strip, to gather their tabs again when a panel's tabs change.
    static STRIPS: RefCell<Vec<glib::WeakRef<TabStrip>>> = const { RefCell::new(Vec::new()) };
    /// The strips' tabs are already to be gathered when the main loop next
    /// runs.
    static GATHER_QUEUED: Cell<bool> = const { Cell::new(false) };
}

/// Lets this panel show its own tabs (when `tabs` gives any) in place of
/// its one tab, in whatever frame holds it.
pub fn set_page_tabs(panel: &panel::Widget, tabs: impl Fn() -> Vec<PageTab> + 'static) {
    PAGE_TABS.with(|all| {
        let mut all = all.borrow_mut();
        all.retain(|(widget, _)| widget.upgrade().is_some_and(|widget| widget != *panel));
        all.push((panel.downgrade(), Rc::new(tabs)));
    });
    page_tabs_changed();
}

/// A panel's own tabs changed: every strip gathers its tabs again when
/// the main loop next runs (so a caller in the middle of a change is
/// never asked for its tabs from inside it).
pub fn page_tabs_changed() {
    if GATHER_QUEUED.replace(true) {
        return;
    }
    // Before GTK lays out and draws (its frame clock runs at a lower
    // priority), so a tab never shows stale for a frame.
    glib::idle_add_local_full(glib::Priority::HIGH_IDLE, || {
        GATHER_QUEUED.set(false);
        let strips: Vec<TabStrip> = STRIPS.with(|strips| {
            let mut strips = strips.borrow_mut();
            strips.retain(|strip| strip.upgrade().is_some());
            strips.iter().filter_map(glib::WeakRef::upgrade).collect()
        });
        for strip in strips {
            strip.send_tabs();
        }
        glib::ControlFlow::Break
    });
}

/// A panel's own tabs, if it has any.
fn page_tabs(panel: &panel::Widget) -> Vec<PageTab> {
    let tabs = PAGE_TABS.with(|all| {
        all.borrow()
            .iter()
            .find(|(widget, _)| widget.upgrade().is_some_and(|widget| widget == *panel))
            .map(|(_, tabs)| Rc::clone(tabs))
    });
    // Called outside the borrow: the tabs read the files page.
    tabs.map(|tabs| tabs()).unwrap_or_default()
}

/// One of a panel's own tabs as the strip shows it: [`PageTab`]'s words,
/// without its `show`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnTabLabel {
    /// Where it is among its panel's own tabs.
    pub index: usize,
    /// The tab's words.
    pub title: String,
    /// Its tooltip.
    pub tooltip: String,
    /// It has unsaved changes.
    pub unsaved: bool,
    /// The worst problem in it.
    pub mark: Option<stave_app::problems::Mark>,
    /// It is its panel's current one.
    pub active: bool,
}

/// What a tab is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TabKind {
    /// The panel itself: its icon and title, and a × to close it.
    Panel {
        /// The panel's title.
        title: String,
        /// The panel's icon name.
        icon: Option<String>,
    },
    /// One of the panel's own tabs (an open file).
    Own(OwnTabLabel),
}

/// One tab of the strip, as owned data.
#[derive(Clone, Debug)]
pub struct PageTabLabel {
    /// The panel the tab shows.
    pub page: glib::WeakRef<panel::Widget>,
    /// The panel is the one its frame shows.
    pub page_shown: bool,
    /// What the tab is.
    pub kind: TabKind,
}

/// The strip's tabs: the frame they follow and the tabs last gathered.
pub struct TabStripComponent {
    /// The frame whose pages are the tabs.
    frame: glib::WeakRef<panel::Frame>,
    /// Watching the frame's pages, to gather the tabs again.
    watch: Option<(gtk::SelectionModel, glib::SignalHandlerId)>,
    /// The tabs, as shown.
    tabs: Vec<PageTabLabel>,
    /// The shown panel's menu, at the strip's end.
    menu: gtk::MenuButton,
}

/// What the strip's tabs are made with.
#[derive(Debug)]
pub struct TabStripInit {
    /// The menu button at the strip's end, which shows the shown panel's
    /// menu.
    pub menu: gtk::MenuButton,
}

/// What the strip's tabs are told.
#[derive(Debug)]
pub enum TabStripInput {
    /// Follow this frame's pages (or none).
    SetFrame(Option<panel::Frame>),
    /// Show these tabs.
    SetTabs(Vec<PageTabLabel>),
    /// People clicked the tab at this place in the strip.
    ShowTab(usize),
    /// People clicked the × of the tab at this place in the strip.
    CloseTab(usize),
}

/// What the strip's tabs tell their owner.
#[derive(Debug)]
pub enum TabStripOutput {
    /// People chose one of a panel's own tabs (its frame now shows the
    /// panel): show it.
    Show {
        /// The panel.
        page: glib::WeakRef<panel::Widget>,
        /// Where the tab is among the panel's own tabs.
        index: usize,
    },
}

impl TabStripComponent {
    /// Follows a frame's pages (or none), and has the tabs gathered.
    fn follow(&mut self, frame: Option<&panel::Frame>) {
        self.stop_watching();
        self.frame.set(frame);
        if let Some(frame) = frame {
            let pages = frame.pages();
            // Captures nothing: whichever strips are alive gather again.
            let handler = pages.connect_items_changed(|_, _, _, _| page_tabs_changed());
            self.watch = Some((pages, handler));
        }
        page_tabs_changed();
    }

    fn stop_watching(&mut self) {
        if let Some((pages, handler)) = self.watch.take() {
            pages.disconnect(handler);
        }
    }

    /// Makes the tabs again: one per page (or one per own tab), the shown
    /// one marked, and the shown panel's menu at the end.
    fn show_tabs(&self, root: &gtk::Box, sender: &ComponentSender<Self>) {
        while let Some(child) = root.first_child() {
            root.remove(&child);
        }
        for (place, tab) in self.tabs.iter().enumerate() {
            let Some(page) = tab.page.upgrade() else {
                continue;
            };
            let widget = match &tab.kind {
                TabKind::Panel { title, icon } => {
                    panel_tab(place, title, icon.as_deref(), tab.page_shown, &page, sender)
                }
                TabKind::Own(own) => own_tab(place, own, tab.page_shown, &page, sender),
            };
            root.append(&widget);
        }
        let menu = self
            .frame
            .upgrade()
            .and_then(|frame| frame.visible_child())
            .and_then(|page| page.menu_model());
        self.menu.set_visible(menu.is_some());
        self.menu.set_menu_model(menu.as_ref());
    }
}

#[relm4::component(pub)]
impl Component for TabStripComponent {
    type Init = TabStripInit;
    type Input = TabStripInput;
    type Output = TabStripOutput;
    type CommandOutput = ();

    view! {
        gtk::Box {
            set_hexpand: true,
            add_css_class: "stave-tabs",
        }
    }

    fn init(
        init: Self::Init,
        root: Self::Root,
        _sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let TabStripInit { menu } = init;
        menu.set_visible(false);
        let model = TabStripComponent {
            frame: glib::WeakRef::new(),
            watch: None,
            tabs: Vec::new(),
            menu,
        };
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: TabStripInput, sender: ComponentSender<Self>, root: &Self::Root) {
        match message {
            TabStripInput::SetFrame(frame) => self.follow(frame.as_ref()),
            TabStripInput::SetTabs(tabs) => {
                self.tabs = tabs;
                self.show_tabs(root, &sender);
            }
            TabStripInput::ShowTab(place) => {
                let Some(tab) = self.tabs.get(place) else {
                    return;
                };
                // The frame's page-changed only has the tabs gathered
                // again, later: nothing comes back in here.
                if let (Some(frame), Some(page)) = (self.frame.upgrade(), tab.page.upgrade()) {
                    frame.set_visible_child(&page);
                }
                match &tab.kind {
                    TabKind::Panel { .. } => {}
                    TabKind::Own(own) => {
                        // Nobody listening is not a mistake.
                        let _ = sender.output(TabStripOutput::Show {
                            page: tab.page.clone(),
                            index: own.index,
                        });
                    }
                }
            }
            TabStripInput::CloseTab(place) => {
                if let Some(page) = self.tabs.get(place).and_then(|tab| tab.page.upgrade()) {
                    page.close();
                }
            }
        }
    }

    fn shutdown(&mut self, _widgets: &mut Self::Widgets, _output: relm4::Sender<Self::Output>) {
        self.stop_watching();
    }
}

/// Shows one of a panel's own tabs, as its owner said to: the tabs are
/// asked for again (they are closures, not data) and the one at `index`
/// is shown.
fn show_own_tab(output: TabStripOutput) {
    match output {
        TabStripOutput::Show { page, index } => {
            let Some(page) = page.upgrade() else {
                return;
            };
            if let Some(tab) = page_tabs(&page).into_iter().nth(index) {
                (tab.show)();
            }
        }
    }
}

/// The tabs, from a frame's pages and the panels' own tabs.
fn gather(frame: Option<&panel::Frame>) -> Vec<PageTabLabel> {
    let Some(frame) = frame else {
        return Vec::new();
    };
    let shown = frame.visible_child();
    let mut tabs = Vec::new();
    for index in 0..frame.n_pages() {
        let Some(page) = frame.page(index) else {
            continue;
        };
        let page_shown = shown.as_ref() == Some(&page);
        let own = page_tabs(&page);
        if own.is_empty() {
            tabs.push(PageTabLabel {
                page: page.downgrade(),
                page_shown,
                kind: TabKind::Panel {
                    title: page
                        .title()
                        .map(|title| title.to_string())
                        .unwrap_or_default(),
                    icon: page.icon_name().map(|icon| icon.to_string()),
                },
            });
        } else {
            tabs.extend(
                own.into_iter()
                    .enumerate()
                    .map(|(index, own)| PageTabLabel {
                        page: page.downgrade(),
                        page_shown,
                        kind: TabKind::Own(OwnTabLabel {
                            index,
                            title: own.title,
                            tooltip: own.tooltip,
                            unsaved: own.unsaved,
                            mark: own.mark,
                            active: own.active,
                        }),
                    }),
            );
        }
    }
    tabs
}

mod imp {
    use super::*;

    /// The strip's widgets and the frame it heads.
    #[derive(Default)]
    pub struct TabStrip {
        /// The frame whose pages are the tabs (the `frame` property).
        pub frame: glib::WeakRef<panel::Frame>,
        /// Widgets the frame adds at the end, then the menu.
        pub end: gtk::Box,
        /// The shown panel's menu.
        pub menu: gtk::MenuButton,
        /// The tabs: the one owner of their component.
        pub(crate) tabs: OnceCell<Bridge<TabStripComponent>>,
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
                self.frame.set(frame.as_ref());
                if let Some(tabs) = self.tabs.get() {
                    tabs.send(TabStripInput::SetFrame(frame));
                }
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
            self.end.add_css_class("stave-tab-end");
            self.menu.set_icon_name("pan-down-symbolic");
            self.menu.set_tooltip_text(Some("Move or close this panel"));
            self.menu.add_css_class("flat");
            self.menu.add_css_class("stave-tab-menu");
            self.end.append(&self.menu);
            let tabs = Bridge::<TabStripComponent>::launch(TabStripInit {
                menu: self.menu.clone(),
            });
            tabs.connect(show_own_tab);
            tabs.widget().set_parent(&*obj);
            self.end.set_parent(&*obj);
            tabs.send(TabStripInput::SetFrame(self.frame.upgrade()));
            // Set once, here: constructed runs once.
            let _ = self.tabs.set(tabs);
            let weak = obj.downgrade();
            STRIPS.with(|strips| strips.borrow_mut().push(weak));
        }

        fn dispose(&self) {
            // The component shuts down when the strip is freed and drops
            // its bridge; its shutdown stops watching the frame.
            if let Some(tabs) = self.tabs.get() {
                tabs.widget().unparent();
            }
            self.end.unparent();
        }
    }

    impl WidgetImpl for TabStrip {}

    impl FrameHeaderImpl for TabStrip {
        fn page_changed(&self, _widget: Option<&panel::Widget>) {
            page_tabs_changed();
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

    /// Gathers the tabs from the frame and sends them to the component.
    fn send_tabs(&self) {
        let imp = self.imp();
        if let Some(tabs) = imp.tabs.get() {
            tabs.send(TabStripInput::SetTabs(gather(imp.frame.upgrade().as_ref())));
        }
    }
}

/// The classes of a tab's outer box, marked when it is the shown one.
fn tab_classes(shown: bool) -> Vec<&'static str> {
    if shown {
        vec!["stave-tab", "shown"]
    } else {
        vec!["stave-tab"]
    }
}

/// One tab: the panel's icon and title, a × to close it; clicking shows
/// it, dragging docks it elsewhere.
fn panel_tab(
    place: usize,
    title: &str,
    icon: Option<&str>,
    shown: bool,
    page: &panel::Widget,
    sender: &ComponentSender<TabStripComponent>,
) -> gtk::Box {
    let label = gtk::Label::builder()
        .label(title)
        .css_classes(["stave-tab-title"])
        .build();
    let inner = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    match icon {
        Some(MODEL_ICON) => {
            let cube = PathIcon::new(icons::PARTS, 14);
            cube.set_valign(gtk::Align::Center);
            inner.append(&cube);
        }
        Some(icon) => inner.append(&gtk::Image::from_icon_name(icon)),
        None => {}
    }
    inner.append(&label);
    let show = gtk::Button::builder()
        .child(&inner)
        .css_classes(["flat", "stave-tab-show"])
        .tooltip_text(title)
        .build();
    let input = sender.input_sender().clone();
    show.connect_clicked(move |_| input.emit(TabStripInput::ShowTab(place)));
    let close = gtk::Button::builder()
        .icon_name("window-close-symbolic")
        .css_classes(["flat", "stave-tab-close"])
        .tooltip_text(format!("Close {title}"))
        .build();
    let input = sender.input_sender().clone();
    close.connect_clicked(move |_| input.emit(TabStripInput::CloseTab(place)));
    let root = gtk::Box::builder().css_classes(tab_classes(shown)).build();
    root.append(&show);
    root.append(&close);
    carry_on_drag(&root, page);
    root
}

/// One of a panel's own tabs (an open file): its problem dot, its words
/// and a yellow ● while unsaved; clicking shows the panel on it, dragging
/// docks the whole panel elsewhere.
fn own_tab(
    place: usize,
    own: &OwnTabLabel,
    page_shown: bool,
    page: &panel::Widget,
    sender: &ComponentSender<TabStripComponent>,
) -> gtk::Box {
    let inner = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let dot = super::style::mark_dot(own.mark);
    inner.append(&dot);
    inner.append(
        &gtk::Label::builder()
            .label(&own.title)
            .css_classes(["stave-tab-title", "stave-file-tab-name"])
            .build(),
    );
    if own.unsaved {
        let unsaved = gtk::Box::builder()
            .css_classes(["stave-mark", "unsaved"])
            .valign(gtk::Align::Center)
            .tooltip_text("Not saved (Ctrl+S saves)")
            .build();
        unsaved.update_property(&[gtk::accessible::Property::Label("not saved")]);
        inner.append(&unsaved);
    }
    let show = gtk::Button::builder()
        .child(&inner)
        .css_classes(["flat", "stave-tab-show", "stave-file-tab"])
        .tooltip_text(&own.tooltip)
        .build();
    let input = sender.input_sender().clone();
    show.connect_clicked(move |_| input.emit(TabStripInput::ShowTab(place)));
    let root = gtk::Box::builder()
        .css_classes(tab_classes(page_shown && own.active))
        .build();
    root.append(&show);
    carry_on_drag(&root, page);
    root
}

/// Dragging a tab carries its panel; libpanel's drop targets (the edges,
/// the middle, other frames) take it.
fn carry_on_drag(root: &gtk::Box, page: &panel::Widget) {
    let drag = gtk::DragSource::builder()
        .actions(gdk::DragAction::MOVE)
        .build();
    let page_weak = page.downgrade();
    drag.connect_prepare(move |_, _, _| {
        let page = page_weak.upgrade()?;
        Some(gdk::ContentProvider::for_value(&page.to_value()))
    });
    drag.connect_drag_begin(|source, _| {
        if let Some(root) = source.widget() {
            source.set_icon(Some(&gtk::WidgetPaintable::new(Some(&root))), 0, 0);
        }
    });
    root.add_controller(drag);
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::ui::gtk_test::{on_gtk_thread, settle};

    /// The strip's tab boxes, in order.
    fn tab_boxes(strip: &TabStrip) -> Vec<gtk::Widget> {
        let Some(tabs) = strip.imp().tabs.get() else {
            return Vec::new();
        };
        let mut boxes = Vec::new();
        let mut child = tabs.widget().first_child();
        while let Some(current) = child {
            child = current.next_sibling();
            boxes.push(current);
        }
        boxes
    }

    /// The button that shows the tab at `place`.
    fn show_button(strip: &TabStrip, place: usize) -> gtk::Button {
        let Some(button) = tab_boxes(strip)
            .get(place)
            .and_then(gtk::Widget::first_child)
            .and_then(|child| child.downcast::<gtk::Button>().ok())
        else {
            panic!("the strip has a tab at {place}");
        };
        button
    }

    /// A frame headed by a strip, holding two panels.
    fn frame_with_two_panels() -> (panel::Frame, TabStrip, panel::Widget, panel::Widget) {
        panel::init();
        let frame = panel::Frame::new();
        let strip = TabStrip::new();
        frame.set_header(Some(&strip));
        let model = panel::Widget::builder().title("Model").build();
        let code = panel::Widget::builder().title("Code").build();
        frame.add(&model);
        frame.add(&code);
        frame.set_visible_child(&model);
        (frame, strip, model, code)
    }

    #[test]
    fn a_dropped_strip_is_freed() {
        on_gtk_thread(strip_is_freed);
    }

    /// Dropping the frame drops its strip, which drops the component's
    /// bridge: the strip, the tabs' widget and the component's runtime
    /// (and with it the model) are all freed. The frame's pages are
    /// watched by a handler that holds nothing of the strip.
    fn strip_is_freed() {
        let (frame, strip, model, code) = frame_with_two_panels();
        settle();
        assert_eq!(tab_boxes(&strip).len(), 2, "one tab per panel");
        let Some(tabs) = strip.imp().tabs.get() else {
            panic!("the strip has its tabs");
        };
        let root = tabs.widget().downgrade();
        let input = tabs.sender().clone();
        let weak = strip.downgrade();
        drop(strip);
        drop((model, code));
        settle();
        assert!(weak.upgrade().is_some(), "the frame holds its strip");
        drop(frame);
        settle();
        assert!(weak.upgrade().is_none(), "the strip is freed");
        assert!(root.upgrade().is_none(), "the tabs' widget is freed");
        assert!(
            input.send(TabStripInput::SetTabs(Vec::new())).is_err(),
            "the component's runtime, and with it the model, is gone"
        );
    }

    #[test]
    fn a_tab_clicked_while_a_tab_is_shown_is_queued_not_re_entered() {
        on_gtk_thread(clicks_are_queued);
    }

    /// The old strip showed a panel and called its own tab's `show` from
    /// inside the click, and showing the panel remade the strip from
    /// inside it too. Now a click only queues a message; the frame shows
    /// the panel when it is handled, the own tab's `show` runs after that
    /// update has returned, and a click GTK fires from inside `show` is
    /// queued in turn.
    fn clicks_are_queued() {
        let (frame, strip, model, code) = frame_with_two_panels();
        // (which own tab was shown, how many tabs the strip had then)
        let shown: Rc<RefCell<Vec<usize>>> = Rc::default();
        let weak_strip = strip.downgrade();
        let log = Rc::clone(&shown);
        set_page_tabs(&code, move || {
            (0..2)
                .map(|index| {
                    let log = Rc::clone(&log);
                    let strip = weak_strip.clone();
                    PageTab {
                        title: format!("file{index}.stave"),
                        tooltip: String::new(),
                        unsaved: index == 1,
                        mark: None,
                        active: index == 0,
                        show: Rc::new(move || {
                            let first = log.borrow().is_empty();
                            log.borrow_mut().push(index);
                            if first && let Some(strip) = strip.upgrade() {
                                // GTK firing another tab's click from in
                                // here only queues a message.
                                show_button(&strip, 2).emit_clicked();
                            }
                        }),
                    }
                })
                .collect()
        });
        assert!(tab_boxes(&strip).is_empty(), "gathered only on the loop");
        settle();
        assert_eq!(tab_boxes(&strip).len(), 3, "the Model, then two files");

        show_button(&strip, 1).emit_clicked();
        assert!(shown.borrow().is_empty(), "nothing has run yet");
        assert_eq!(
            frame.visible_child().as_ref(),
            Some(&model),
            "the click is only queued"
        );

        settle();
        assert_eq!(frame.visible_child().as_ref(), Some(&code));
        assert_eq!(
            *shown.borrow(),
            vec![0, 1],
            "each after its update, the inner click after the first"
        );
    }

    #[test]
    fn closing_a_tab_closes_its_panel() {
        on_gtk_thread(close_closes);
    }

    fn close_closes() {
        let (frame, strip, _model, code) = frame_with_two_panels();
        settle();
        let Some(close) = tab_boxes(&strip)
            .get(1)
            .and_then(gtk::Widget::last_child)
            .and_then(|child| child.downcast::<gtk::Button>().ok())
        else {
            panic!("the Code tab has a ×");
        };
        close.emit_clicked();
        assert_eq!(frame.n_pages(), 2, "only queued");
        settle();
        assert_eq!(frame.n_pages(), 1, "the Code panel is closed");
        assert!(code.parent().is_none());
        assert_eq!(tab_boxes(&strip).len(), 1, "its tab is gone");
    }
}
