//! The panels' headers: a slim strip of flush tabs, as in an IDE, each as
//! wide as its title (libadwaita's tab bar, which libpanel offers, draws
//! rounded tabs a fixed 220 px wide). A tab shows its panel, its × closes
//! it (the Model's tab has none: the 3D view can't be closed), dragging it
//! docks the panel elsewhere (libpanel's drop targets take a dragged
//! `panel::Widget`), and the menu at the end holds the shown panel's own
//! menu ("Move to the Right", ...).
//!
//! For the keyboard and screen readers: the strip is a tab list and each
//! tab a tab, the shown one selected; showing another page restyles the
//! tabs in place, so the tab with the keyboard keeps it; and one Tab stop
//! per tab (the × is for the pointer; the menu closes a panel). Ctrl+Page
//! Up / Ctrl+Page Down, which show the previous and next tab while the
//! keyboard is anywhere in the frame, are the workbench's, on its frames.
//!
//! A panel can show its own tabs in place of its one tab: the Code
//! panel's open files appear this way, one tab each (name, and a yellow ●
//! while unsaved), beside the Model's tab in the frame they share.
//! Clicking one shows the panel on that file. The window's workbench
//! keeps every panel's own tabs ([`OwnTabs`], data, by panel) for its
//! dock and hands each of its strips a copy whenever they change; a
//! strip's chosen own tab goes back to the workbench as
//! [`TabStripOutput::Show`], forwarded when the strip is made. Nothing
//! is kept anywhere else: every window's workbench has its own strips and
//! its own copy.
//!
//! The strip reads its frame (its pages, the one shown) and never changes
//! it: the frame and its pages are the workbench's dock. A tab clicked or
//! closed is an output ([`TabStripOutput::ShowPanel`],
//! [`TabStripOutput::Show`], [`TabStripOutput::Close`]) that the
//! workbench carries out on its own panel, and the strip restyles when
//! the frame says another page shows.
//!
//! The strip is a GObject subclass, [`TabStrip`], because libpanel takes a
//! frame's header as a widget implementing `PanelFrameHeader`. Its tabs
//! are a relm4 component, [`TabStripComponent`] (docs/relm4-migration.md):
//! the component's model is the only state, the tabs' buttons and keys
//! only send [`TabStripInput`] messages, and what a click asks of the
//! dock is a [`TabStripOutput`]. The subclass owns the component's
//! controller. The component gathers its tabs, from the frame's pages and
//! its copy of the panels' own tabs, in its own update: when the frame's
//! pages change ([`TabStripInput::Gather`], queued by the pages' handler)
//! or the own tabs do ([`TabStripInput::SetOwnTabs`]), never inside
//! whatever changed them; another page shown is only
//! [`TabStripInput::PageChanged`], which restyles the tabs already made.

use stave_app::i18n::{fill_in, tr};
use std::cell::OnceCell;
use std::collections::BTreeMap;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gdk, glib};
use panel::prelude::*;
use panel::subclass::prelude::*;
use relm4::{Component, ComponentController, ComponentParts, ComponentSender, Controller};

use super::icons::{self, PathIcon};
use super::workbench::PanelId;

/// The icon name that gives a tab the Model's cube.
pub const MODEL_ICON: &str = "stave-model";

/// One of a panel's own tabs, as its owner hands it over.
#[derive(Clone, Debug, PartialEq, Eq)]
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
}

/// Every panel's own tabs, by panel, as its owner last handed them over:
/// a panel missing (or with none) shows its one tab.
pub type OwnTabs = BTreeMap<PanelId, Vec<PageTab>>;

/// Which panel a page is, read from its id.
pub(super) fn panel_of(page: &panel::Widget) -> Option<PanelId> {
    // Read as a property: `id()` asserts on a panel that has none.
    let id = page.property::<Option<String>>("id");
    id.as_deref().and_then(PanelId::from_word)
}

/// One of a panel's own tabs as the strip shows it: [`PageTab`]'s words
/// and its place among its panel's own tabs.
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

/// Whether a panel's tab can close it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Closing {
    /// A × closes the panel.
    Closable,
    /// No ×: the 3D view can't be closed (its tab is still dragged).
    Fixed,
}

/// What a tab is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TabKind {
    /// The panel itself: its icon and title, and (unless it is fixed) a ×
    /// to close it.
    Panel {
        /// The panel's title.
        title: String,
        /// The panel's icon name.
        icon: Option<String>,
        /// Whether the tab has a ×.
        closing: Closing,
    },
    /// One of the panel's own tabs (an open file).
    Own {
        /// The panel whose tab it is.
        panel: PanelId,
        /// The tab.
        tab: OwnTabLabel,
    },
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

/// One tab as made: what is restyled in place when another page shows.
#[derive(Debug)]
struct MadeTab {
    /// Its place in the strip's tabs.
    place: usize,
    /// The tab: the show button and the ×.
    root: gtk::Box,
    /// What the keyboard and a screen reader take for the tab.
    show: gtk::Button,
}

impl MadeTab {
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

/// The strip's tabs: the frame they follow, the panels' own tabs as last
/// handed over, and the tabs last gathered.
pub struct TabStripComponent {
    /// The frame whose pages are the tabs.
    frame: glib::WeakRef<panel::Frame>,
    /// Watching the frame's pages, to gather the tabs again.
    watch: Option<(gtk::SelectionModel, glib::SignalHandlerId)>,
    /// The panels' own tabs, the strip's copy.
    own: OwnTabs,
    /// The tabs, as shown.
    tabs: Vec<PageTabLabel>,
    /// The tabs' widgets, one per entry of `tabs`.
    made: Vec<MadeTab>,
}

/// What the strip's tabs are made with.
#[derive(Debug)]
pub struct TabStripInit {
    /// The panels' own tabs as they are now.
    pub own: OwnTabs,
}

/// What the strip's tabs are told.
#[derive(Debug)]
pub enum TabStripInput {
    /// Follow this frame's pages (or none).
    SetFrame(Option<panel::Frame>),
    /// The panels' own tabs are these now: the tabs are gathered again.
    SetOwnTabs(OwnTabs),
    /// The frame's pages came or went: the tabs are gathered again, made
    /// again, and the one at the place the keyboard was takes it back.
    Gather,
    /// The frame shows another page: the tabs already made are restyled
    /// in place, so the tab with the keyboard keeps it.
    PageChanged,
    /// People clicked the tab at this place in the strip.
    ShowTab(usize),
    /// People clicked the × of the tab at this place in the strip.
    CloseTab(usize),
}

/// What the strip's tabs tell their owner, the workbench, whose dock the
/// frame and its pages are: it carries each out on its own panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TabStripOutput {
    /// People clicked a panel's tab: show the panel in its frame.
    ShowPanel(PanelId),
    /// People chose one of a panel's own tabs: show the panel in its
    /// frame, on that tab.
    Show {
        /// The panel.
        panel: PanelId,
        /// Where the tab is among the panel's own tabs.
        index: usize,
    },
    /// People clicked a panel's ×: close the panel.
    Close(PanelId),
}

impl TabStripComponent {
    /// Follows a frame's pages (or none), and has the tabs gathered.
    fn follow(&mut self, frame: Option<&panel::Frame>, sender: &ComponentSender<Self>) {
        self.stop_watching();
        self.frame.set(frame);
        if let Some(frame) = frame {
            let pages = frame.pages();
            // Only sends: the tabs are gathered when the main loop next
            // runs, never inside the change.
            let input = sender.input_sender().clone();
            let handler =
                pages.connect_items_changed(move |_, _, _, _| input.emit(TabStripInput::Gather));
            self.watch = Some((pages, handler));
        }
        sender.input_sender().emit(TabStripInput::Gather);
    }

    /// Stops watching the frame's pages.
    fn stop_watching(&mut self) {
        if let Some((pages, handler)) = self.watch.take() {
            pages.disconnect(handler);
        }
    }

    /// Gathers the tabs from the frame and the own tabs, and makes them
    /// again.
    fn gather(&mut self, widgets: &TabStripComponentWidgets, sender: &ComponentSender<Self>) {
        self.tabs = gather(self.frame.upgrade().as_ref(), &self.own);
        self.show_tabs(widgets, sender);
    }

    /// Makes the tabs again: one per page (or one per own tab), the shown
    /// one marked, and the shown panel's menu at the end. The tab at the
    /// place the keyboard was takes it back.
    fn show_tabs(&mut self, widgets: &TabStripComponentWidgets, sender: &ComponentSender<Self>) {
        let list = &widgets.list;
        let focused = self.made.iter().position(|made| made.show.has_focus());
        while let Some(child) = list.first_child() {
            list.remove(&child);
        }
        self.made.clear();
        for (place, tab) in self.tabs.iter().enumerate() {
            let Some(page) = tab.page.upgrade() else {
                continue;
            };
            let made = match &tab.kind {
                TabKind::Panel {
                    title,
                    icon,
                    closing,
                } => panel_tab(place, title, icon.as_deref(), *closing, &page, sender),
                TabKind::Own { tab, .. } => own_tab(place, tab, &page, sender),
            };
            list.append(&made.root);
            self.made.push(made);
        }
        if let Some(place) = focused
            && let Some(made) = self.made.get(place.min(self.made.len().saturating_sub(1)))
        {
            made.show.grab_focus();
        }
        self.mark_shown(widgets);
    }

    /// Marks the shown page's tab, in place (the tab the keyboard is on
    /// stays where it is), and puts the shown panel's menu at the end. A
    /// panel's own tab is marked only when it is its panel's current one.
    fn mark_shown(&mut self, widgets: &TabStripComponentWidgets) {
        let shown = self.frame.upgrade().and_then(|frame| frame.visible_child());
        for tab in &mut self.tabs {
            tab.page_shown = tab
                .page
                .upgrade()
                .is_some_and(|page| shown.as_ref() == Some(&page));
        }
        for made in &self.made {
            let marked = self.tabs.get(made.place).is_some_and(|tab| {
                tab.page_shown
                    && match &tab.kind {
                        TabKind::Panel { .. } => true,
                        TabKind::Own { tab, .. } => tab.active,
                    }
            });
            made.mark(marked);
            // The strip scrolls when its tabs don't fit: the marked one is
            // brought into sight, in the component's own viewport.
            if marked {
                widgets.viewport.scroll_to(&made.root, None);
            }
        }
        let menu = shown.as_ref().and_then(|page| page.menu_model());
        widgets.menu.set_visible(menu.is_some());
        widgets.menu.set_menu_model(menu.as_ref());
    }
}

#[relm4::component(pub)]
impl Component for TabStripComponent {
    type Init = TabStripInit;
    type Input = TabStripInput;
    type Output = TabStripOutput;
    type CommandOutput = ();

    view! {
        // The tabs, scrolling sideways, then the shown panel's menu at
        // the end, which stays put while the tabs scroll.
        gtk::Box {
            set_hexpand: true,

            // Holds the tabs, scrolling them sideways when there are more
            // than fit: a strip as wide as all its tabs would widen its
            // dock column with every file opened.
            gtk::ScrolledWindow {
                set_hscrollbar_policy: gtk::PolicyType::External,
                set_vscrollbar_policy: gtk::PolicyType::Never,
                set_propagate_natural_width: true,
                set_hexpand: true,

                #[name = "viewport"]
                gtk::Viewport {
                    #[name = "list"]
                    #[wrap(Some)]
                    set_child = &gtk::Box::builder()
                        .accessible_role(gtk::AccessibleRole::TabList)
                        .build() -> gtk::Box {
                        set_hexpand: true,
                        add_css_class: "stave-tabs",
                    },
                },
            },

            gtk::Box {
                add_css_class: "stave-tab-end",

                #[name = "menu"]
                gtk::MenuButton {
                    set_icon_name: "pan-down-symbolic",
                    set_tooltip_text: Some(&tr("Move or close this panel")),
                    update_property: &[gtk::accessible::Property::Label(&tr("Panel menu"))],
                    add_css_class: "flat",
                    add_css_class: "stave-tab-menu",
                    set_visible: false,
                },
            },
        }
    }

    fn init(
        init: Self::Init,
        root: Self::Root,
        _sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let TabStripInit { own } = init;
        let model = TabStripComponent {
            frame: glib::WeakRef::new(),
            watch: None,
            own,
            tabs: Vec::new(),
            made: Vec::new(),
        };
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: TabStripInput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            TabStripInput::SetFrame(frame) => self.follow(frame.as_ref(), &sender),
            TabStripInput::SetOwnTabs(own) => {
                if own != self.own {
                    self.own = own;
                    self.gather(widgets, &sender);
                }
            }
            TabStripInput::Gather => self.gather(widgets, &sender),
            TabStripInput::PageChanged => self.mark_shown(widgets),
            TabStripInput::ShowTab(place) => {
                let Some(tab) = self.tabs.get(place) else {
                    return;
                };
                let output = match &tab.kind {
                    TabKind::Panel { .. } => tab
                        .page
                        .upgrade()
                        .as_ref()
                        .and_then(panel_of)
                        .map(TabStripOutput::ShowPanel),
                    TabKind::Own { panel, tab } => Some(TabStripOutput::Show {
                        panel: *panel,
                        index: tab.index,
                    }),
                };
                if let Some(output) = output {
                    // Nobody listening is not a mistake.
                    let _ = sender.output(output);
                }
            }
            TabStripInput::CloseTab(place) => {
                if let Some(panel) = self
                    .tabs
                    .get(place)
                    .and_then(|tab| tab.page.upgrade())
                    .as_ref()
                    .and_then(panel_of)
                {
                    let _ = sender.output(TabStripOutput::Close(panel));
                }
            }
        }
    }

    fn shutdown(&mut self, _widgets: &mut Self::Widgets, _output: relm4::Sender<Self::Output>) {
        self.stop_watching();
    }
}

/// Whether a panel's tab has a ×: the 3D view's has none.
fn closing_of(page: &panel::Widget) -> Closing {
    if panel_of(page) == Some(PanelId::View) {
        Closing::Fixed
    } else {
        Closing::Closable
    }
}

/// The tabs, from a frame's pages and the panels' own tabs.
fn gather(frame: Option<&panel::Frame>, own_tabs: &OwnTabs) -> Vec<PageTabLabel> {
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
        let owned = panel_of(&page)
            .and_then(|panel| Some((panel, own_tabs.get(&panel)?)))
            .filter(|(_, own)| !own.is_empty());
        let Some((panel, own)) = owned else {
            tabs.push(PageTabLabel {
                page: page.downgrade(),
                page_shown,
                kind: TabKind::Panel {
                    title: page
                        .title()
                        .map(|title| title.to_string())
                        .unwrap_or_default(),
                    icon: page.icon_name().map(|icon| icon.to_string()),
                    closing: closing_of(&page),
                },
            });
            continue;
        };
        tabs.extend(own.iter().enumerate().map(|(index, own)| PageTabLabel {
            page: page.downgrade(),
            page_shown,
            kind: TabKind::Own {
                panel,
                tab: OwnTabLabel {
                    index,
                    title: own.title.clone(),
                    tooltip: own.tooltip.clone(),
                    unsaved: own.unsaved,
                    mark: own.mark,
                    active: own.active,
                },
            },
        }));
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
        /// Widgets the frame adds at the end (libpanel's `add_prefix` and
        /// `add_suffix`), after the tabs and their menu.
        pub end: gtk::Box,
        /// The tabs: the one owner of their component, set once, by
        /// [`super::TabStrip::new`].
        pub(crate) tabs: OnceCell<Controller<TabStripComponent>>,
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
                self.send(TabStripInput::SetFrame(frame));
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
            self.end.set_parent(&*obj);
        }

        fn dispose(&self) {
            // The component shuts down when the strip is freed and drops
            // its controller; its shutdown stops watching the frame.
            if let Some(tabs) = self.tabs.get() {
                tabs.widget().unparent();
            }
            self.end.unparent();
        }
    }

    impl TabStrip {
        /// Queues `input` for the tabs (once they are launched; once they
        /// have shut down there is nobody to tell).
        pub(super) fn send(&self, input: TabStripInput) {
            if let Some(tabs) = self.tabs.get() {
                let _ = tabs.sender().send(input);
            }
        }
    }

    impl WidgetImpl for TabStrip {}

    impl FrameHeaderImpl for TabStrip {
        fn page_changed(&self, _widget: Option<&panel::Widget>) {
            // Only restyled: the tab the keyboard is on stays where it is.
            self.send(TabStripInput::PageChanged);
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

impl TabStrip {
    /// A strip, given to a frame with `Frame::set_header`, showing the
    /// panels' own tabs as `own` has them. A chosen own tab goes to
    /// `owner` as `message(output)`.
    pub fn new<M: 'static>(
        own: OwnTabs,
        owner: &relm4::Sender<M>,
        message: impl Fn(TabStripOutput) -> M + 'static,
    ) -> TabStrip {
        let strip: TabStrip = glib::Object::new();
        let imp = strip.imp();
        let tabs = TabStripComponent::builder()
            .launch(TabStripInit { own })
            .forward(owner, message);
        // Before the frame's own end widgets.
        tabs.widget()
            .insert_before(&strip, Some(imp.end.upcast_ref::<gtk::Widget>()));
        let _ = tabs
            .sender()
            .send(TabStripInput::SetFrame(imp.frame.upgrade()));
        // Set once: the strip was made just now.
        let _ = imp.tabs.set(tabs);
        strip
    }

    /// The panels' own tabs are these now: the strip shows them when the
    /// main loop next runs.
    pub fn show_own_tabs(&self, own: OwnTabs) {
        self.imp().send(TabStripInput::SetOwnTabs(own));
    }
}

/// One tab: the panel's icon and title, a × to close it (not the 3D
/// view's, which can't be closed); clicking shows it, dragging docks it
/// elsewhere.
fn panel_tab(
    place: usize,
    title: &str,
    icon: Option<&str>,
    closing: Closing,
    page: &panel::Widget,
    sender: &ComponentSender<TabStripComponent>,
) -> MadeTab {
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
        .accessible_role(gtk::AccessibleRole::Tab)
        .child(&inner)
        .css_classes(["flat", "stave-tab-show"])
        .tooltip_text(title)
        .build();
    let input = sender.input_sender().clone();
    show.connect_clicked(move |_| input.emit(TabStripInput::ShowTab(place)));
    let root = gtk::Box::builder().css_classes(["stave-tab"]).build();
    root.append(&show);
    match closing {
        Closing::Fixed => root.add_css_class("stave-tab-fixed"),
        Closing::Closable => {
            let close_words = fill_in(&tr("Close {title}"), &[("title", &title)]);
            // Not a Tab stop: one per tab, and the menu closes a panel too.
            let close = gtk::Button::builder()
                .icon_name("window-close-symbolic")
                .css_classes(["flat", "stave-tab-close"])
                .tooltip_text(&close_words)
                .focusable(false)
                .build();
            close.update_property(&[gtk::accessible::Property::Label(&close_words)]);
            let input = sender.input_sender().clone();
            close.connect_clicked(move |_| input.emit(TabStripInput::CloseTab(place)));
            root.append(&close);
        }
    }
    carry_on_drag(&root, page);
    MadeTab { place, root, show }
}

/// One of a panel's own tabs (an open file): its problem dot, its words
/// and a yellow ● while unsaved; clicking shows the panel on it, dragging
/// docks the whole panel elsewhere.
fn own_tab(
    place: usize,
    own: &OwnTabLabel,
    page: &panel::Widget,
    sender: &ComponentSender<TabStripComponent>,
) -> MadeTab {
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
            .tooltip_text(tr("Not saved (Ctrl+S saves)"))
            .build();
        unsaved.update_property(&[gtk::accessible::Property::Label(&tr("not saved"))]);
        inner.append(&unsaved);
    }
    let show = gtk::Button::builder()
        .accessible_role(gtk::AccessibleRole::Tab)
        .child(&inner)
        .css_classes(["flat", "stave-tab-show", "stave-file-tab"])
        .tooltip_text(&own.tooltip)
        .build();
    let input = sender.input_sender().clone();
    show.connect_clicked(move |_| input.emit(TabStripInput::ShowTab(place)));
    let root = gtk::Box::builder().css_classes(["stave-tab"]).build();
    root.append(&show);
    carry_on_drag(&root, page);
    MadeTab { place, root, show }
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
    use std::rc::Rc;

    use super::*;
    use crate::ui::gtk_test::{handle_each, on_gtk_thread, recorder, settle, unnamed_buttons};

    /// A strip with no own tabs, whose outputs nobody hears.
    fn strip() -> TabStrip {
        let (owner, _) = recorder();
        TabStrip::new(OwnTabs::new(), &owner, std::convert::identity)
    }

    /// Two open files as the Code panel's own tabs, the first current.
    fn two_files() -> OwnTabs {
        OwnTabs::from([(
            PanelId::Code,
            (0..2)
                .map(|index| PageTab {
                    title: format!("file{index}.stave"),
                    tooltip: String::new(),
                    unsaved: index == 1,
                    mark: None,
                    active: index == 0,
                })
                .collect(),
        )])
    }

    /// The strip's list of tabs: in the component's root, the scroller,
    /// its viewport, then the list.
    fn tab_list(strip: &TabStrip) -> Option<gtk::Widget> {
        let tabs = strip.imp().tabs.get()?;
        tabs.widget()
            .first_child()
            .and_downcast::<gtk::ScrolledWindow>()?
            .child()?
            .first_child()
    }

    /// The strip's tab boxes, in order.
    fn tab_boxes(strip: &TabStrip) -> Vec<gtk::Widget> {
        let mut boxes = Vec::new();
        let mut child = tab_list(strip).and_then(|list| list.first_child());
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

    /// A frame headed by `strip`, holding two panels: the Model and the
    /// Code panel.
    fn frame_with_two_panels_under(
        strip: TabStrip,
    ) -> (panel::Frame, TabStrip, panel::Widget, panel::Widget) {
        panel::init();
        let frame = panel::Frame::new();
        frame.set_header(Some(&strip));
        let model = panel::Widget::builder().title("Model").build();
        model.set_id(PanelId::View.word());
        let code = panel::Widget::builder().title("Code").build();
        code.set_id(PanelId::Code.word());
        frame.add(&model);
        frame.add(&code);
        frame.set_visible_child(&model);
        (frame, strip, model, code)
    }

    /// A frame headed by a strip nobody hears, holding two panels.
    fn frame_with_two_panels() -> (panel::Frame, TabStrip, panel::Widget, panel::Widget) {
        frame_with_two_panels_under(strip())
    }

    #[test]
    fn the_strip_shows_the_shown_panels_menu_on_its_own_button() {
        on_gtk_thread(menu_is_the_strips);
    }

    /// The menu button at the strip's end is the component's own: it
    /// shows the shown panel's menu, and hides while the shown panel has
    /// none, restyled from `PageChanged` like the tabs.
    fn menu_is_the_strips() {
        let (frame, strip, model, code) = frame_with_two_panels();
        let menu = gtk::gio::Menu::new();
        menu.append(Some("Move to the Right"), None);
        code.set_menu_model(Some(&menu));
        settle();
        let Some(tabs) = strip.imp().tabs.get() else {
            panic!("the strip has its tabs");
        };
        let Some(button) = tabs
            .widget()
            .last_child()
            .and_then(|end| end.first_child())
            .and_then(|button| button.downcast::<gtk::MenuButton>().ok())
        else {
            panic!("the strip ends with its menu button");
        };
        assert!(!button.is_visible(), "the Model panel has no menu");
        frame.set_visible_child(&code);
        settle();
        assert!(button.is_visible(), "the Code panel's menu");
        assert_eq!(button.menu_model().as_ref(), Some(menu.upcast_ref()));
        frame.set_visible_child(&model);
        settle();
        assert!(!button.is_visible());
        drop((frame, strip));
    }

    #[test]
    fn a_dropped_strip_is_freed() {
        on_gtk_thread(strip_is_freed);
    }

    /// Dropping the frame drops its strip, which drops the component's
    /// controller: the strip, the tabs' widget and the component's runtime
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
            input.send(TabStripInput::Gather).is_err(),
            "the component's runtime, and with it the model, is gone"
        );
    }

    #[test]
    fn a_tab_clicked_while_a_tab_is_shown_is_queued_not_re_entered() {
        on_gtk_thread(clicks_are_queued);
    }

    /// The old strip showed a panel and called its own tab's `show` from
    /// inside the click, and showing the panel remade the strip from
    /// inside it too. Now a click only queues a message; the chosen tab
    /// reaches the owner after that update has returned (the owner, whose
    /// dock the frame is, shows the panel: the strip never changes the
    /// frame), and a click GTK fires while the owner handles it is queued
    /// in turn.
    fn clicks_are_queued() {
        let (owner, outputs) = relm4::channel();
        let (frame, strip, model, code) = frame_with_two_panels_under(TabStrip::new(
            OwnTabs::new(),
            &owner,
            std::convert::identity,
        ));
        // (which panel's own tab was shown, and which)
        let shown: Rc<RefCell<Vec<(PanelId, usize)>>> = Rc::default();
        let weak_strip = strip.downgrade();
        let log = Rc::clone(&shown);
        handle_each(outputs, move |output| {
            let TabStripOutput::Show { panel, index } = output else {
                return;
            };
            let first = log.borrow().is_empty();
            log.borrow_mut().push((panel, index));
            if first && let Some(strip) = weak_strip.upgrade() {
                // GTK firing another tab's click from in here only queues
                // a message.
                show_button(&strip, 2).emit_clicked();
            }
        });
        settle();
        assert_eq!(tab_boxes(&strip).len(), 2, "one tab per panel");
        strip.show_own_tabs(two_files());
        assert_eq!(tab_boxes(&strip).len(), 2, "gathered only on the loop");
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
        assert_eq!(
            *shown.borrow(),
            vec![(PanelId::Code, 0), (PanelId::Code, 1)],
            "each after its update, the inner click after the first"
        );
        assert_eq!(
            frame.visible_child().as_ref(),
            Some(&model),
            "the strip leaves the frame to its owner"
        );
        drop(code);
    }

    #[test]
    fn a_panel_tab_clicked_asks_its_owner_to_show_it() {
        on_gtk_thread(panel_tab_asks);
    }

    /// Clicking a panel's tab asks the owner to show that panel, by its
    /// id, and changes nothing of the frame itself.
    fn panel_tab_asks() {
        let (owner, heard) = recorder::<TabStripOutput>();
        let (frame, strip, model, _code) = frame_with_two_panels_under(TabStrip::new(
            OwnTabs::new(),
            &owner,
            std::convert::identity,
        ));
        settle();
        show_button(&strip, 1).emit_clicked();
        settle();
        assert_eq!(
            *heard.borrow(),
            vec![TabStripOutput::ShowPanel(PanelId::Code)]
        );
        assert_eq!(frame.visible_child().as_ref(), Some(&model));
    }

    #[test]
    fn more_tabs_never_make_the_strip_wider_than_it_must_be() {
        on_gtk_thread(more_tabs_scroll);
    }

    /// Selecting things in the 3D view opens their files as the Code
    /// panel's own tabs. The strip used to be as wide as all its tabs at
    /// least, so each file opened widened the dock column holding it and
    /// the 3D view jumped narrower. The tabs now scroll sideways: the
    /// strip's least width stays what it was with two tabs.
    fn more_tabs_scroll() {
        let (owner, _) = recorder::<TabStripOutput>();
        let (_frame, strip, _model, _code) =
            frame_with_two_panels_under(TabStrip::new(two_files(), &owner, std::convert::identity));
        settle();
        let least = |strip: &TabStrip| strip.measure(gtk::Orientation::Horizontal, -1).0;
        let before = least(&strip);
        let many: Vec<PageTab> = (0..12)
            .map(|index| PageTab {
                title: format!("a_long_file_name_{index}.stave"),
                tooltip: String::new(),
                unsaved: false,
                mark: None,
                active: index == 11,
            })
            .collect();
        let mut own = OwnTabs::new();
        own.insert(PanelId::Code, many);
        strip.show_own_tabs(own);
        settle();
        assert_eq!(tab_boxes(&strip).len(), 13, "the Model, then twelve files");
        assert!(
            least(&strip) <= before,
            "the strip asks for {} px with twelve files, {before} px with two",
            least(&strip)
        );
    }

    #[test]
    fn the_menu_stays_in_sight_while_the_tabs_scroll() {
        on_gtk_thread(menu_stays_while_tabs_scroll);
    }

    /// The tabs scroll inside the strip; the panel menu at its end is
    /// outside the scroller, so however many tabs there are it stays
    /// shown, whole, inside the strip, and the shown tab is scrolled to.
    fn menu_stays_while_tabs_scroll() {
        let (owner, _) = recorder::<TabStripOutput>();
        let (frame, strip, _model, code) = frame_with_two_panels_under(TabStrip::new(
            OwnTabs::new(),
            &owner,
            std::convert::identity,
        ));
        let menu = gtk::gio::Menu::new();
        menu.append(Some("Move to the Right"), None);
        code.set_menu_model(Some(&menu));
        frame.set_visible_child(&code);
        let window = gtk::Window::builder()
            .default_width(320)
            .default_height(240)
            .child(&frame)
            .build();
        window.present();
        let many: Vec<PageTab> = (0..12)
            .map(|index| PageTab {
                title: format!("a_long_file_name_{index}.stave"),
                tooltip: String::new(),
                unsaved: false,
                mark: None,
                active: index == 11,
            })
            .collect();
        let mut own = OwnTabs::new();
        own.insert(PanelId::Code, many);
        strip.show_own_tabs(own);
        let context = glib::MainContext::default();
        for _ in 0..20 {
            context.iteration(false);
        }
        settle();
        assert_eq!(tab_boxes(&strip).len(), 13);
        let Some(tabs) = strip.imp().tabs.get() else {
            panic!("the strip has its tabs");
        };
        let Some(button) = tabs
            .widget()
            .last_child()
            .and_then(|end| end.first_child())
            .and_then(|button| button.downcast::<gtk::MenuButton>().ok())
        else {
            panic!("the strip ends with its menu button");
        };
        assert!(button.is_visible() && button.is_mapped(), "the menu shows");
        let Some(bounds) = button.compute_bounds(&strip) else {
            panic!("the menu is laid out in the strip");
        };
        assert!(
            bounds.x() >= 0.0 && bounds.x() + bounds.width() <= strip.width() as f32 + 0.5,
            "the menu is whole inside the strip ({bounds:?}, {} px wide)",
            strip.width()
        );
        let Some(scroller) = tabs
            .widget()
            .first_child()
            .and_downcast::<gtk::ScrolledWindow>()
        else {
            panic!("the tabs are in a scroller");
        };
        assert!(
            scroller.hadjustment().upper() > scroller.hadjustment().page_size(),
            "the tabs overflow and scroll"
        );
        window.destroy();
    }

    #[test]
    fn a_strip_shows_the_own_tabs_it_starts_with_and_drops_them() {
        on_gtk_thread(own_tabs_come_and_go);
    }

    /// A strip made while a panel has own tabs shows them at once (a
    /// frame made later, a panel dragged somewhere new); handed none, the
    /// panel shows its one tab again. Own tabs of a panel the frame
    /// doesn't hold change nothing.
    fn own_tabs_come_and_go() {
        let (owner, _) = recorder::<TabStripOutput>();
        let (_frame, strip, _model, _code) =
            frame_with_two_panels_under(TabStrip::new(two_files(), &owner, std::convert::identity));
        settle();
        assert_eq!(tab_boxes(&strip).len(), 3, "the Model, then two files");
        let mut others = OwnTabs::new();
        others.insert(
            PanelId::Details,
            two_files().remove(&PanelId::Code).unwrap_or_default(),
        );
        strip.show_own_tabs(others);
        settle();
        assert_eq!(tab_boxes(&strip).len(), 2, "the Model and the Code panel");
    }

    #[test]
    fn closing_a_tab_closes_its_panel() {
        on_gtk_thread(close_closes);
    }

    /// The × asks the owner to close the panel; the strip doesn't close
    /// it itself.
    fn close_closes() {
        let (owner, heard) = recorder::<TabStripOutput>();
        let (frame, strip, _model, code) = frame_with_two_panels_under(TabStrip::new(
            OwnTabs::new(),
            &owner,
            std::convert::identity,
        ));
        settle();
        let Some(close) = tab_boxes(&strip)
            .get(1)
            .and_then(gtk::Widget::last_child)
            .and_then(|child| child.downcast::<gtk::Button>().ok())
        else {
            panic!("the Code tab has a ×");
        };
        close.emit_clicked();
        assert!(heard.borrow().is_empty(), "only queued");
        settle();
        assert_eq!(*heard.borrow(), vec![TabStripOutput::Close(PanelId::Code)]);
        assert_eq!(frame.n_pages(), 2, "closing is the owner's");
        assert!(code.parent().is_some());
    }

    #[test]
    fn the_models_tab_has_no_close_button() {
        on_gtk_thread(model_tab_is_fixed);
    }

    /// The 3D view can't be closed: its tab has no × (and is marked fixed,
    /// still dragged); another panel's tab has one.
    fn model_tab_is_fixed() {
        panel::init();
        let frame = panel::Frame::new();
        let strip = strip();
        frame.set_header(Some(&strip));
        let model = panel::Widget::builder().title("Model").build();
        model.set_id(super::super::workbench::PanelId::View.word());
        let parts = panel::Widget::builder().title("Parts").build();
        frame.add(&model);
        frame.add(&parts);
        settle();
        let tabs = tab_boxes(&strip);
        assert_eq!(tabs.len(), 2);
        assert!(
            tabs[0]
                .last_child()
                .is_some_and(|child| !child.has_css_class("stave-tab-close"))
        );
        assert!(tabs[0].has_css_class("stave-tab-fixed"));
        assert!(
            tabs[1]
                .last_child()
                .is_some_and(|child| child.has_css_class("stave-tab-close"))
        );
        assert!(!tabs[1].has_css_class("stave-tab-fixed"));
    }

    #[test]
    fn tabs_stay_put_when_another_page_shows() {
        on_gtk_thread(tabs_stay_put);
    }

    /// Showing another page restyles the tabs in place (the focused tab
    /// survives).
    fn tabs_stay_put() {
        panel::init();
        let frame = panel::Frame::new();
        let strip = strip();
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
        settle();
        let before = tab_boxes(&strip);
        assert_eq!(before.len(), 3);
        assert_eq!(
            tab_list(&strip).map(|list| list.accessible_role()),
            Some(gtk::AccessibleRole::TabList)
        );
        let show = |place: usize| show_button(&strip, place);
        assert_eq!(show(0).accessible_role(), gtk::AccessibleRole::Tab);
        let focus = || gtk::prelude::GtkWindowExt::focus(&window);
        assert!(show(1).grab_focus());
        assert_eq!(focus().as_ref(), Some(show(1).upcast_ref()));
        frame.set_visible_child(&pages[1]);
        settle();
        assert_eq!(tab_boxes(&strip), before, "the tabs were made again");
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
        // The × is not a Tab stop.
        let Some(close) = before[0].last_child() else {
            panic!("the Parts tab has a ×");
        };
        assert!(!close.is_focusable());
        assert!(close.has_css_class("stave-tab-close"));
        // Every icon button (the ×s, the ▾ menu) has a name.
        assert_eq!(unnamed_buttons(&strip), Vec::<String>::new());
        window.destroy();
    }
}
