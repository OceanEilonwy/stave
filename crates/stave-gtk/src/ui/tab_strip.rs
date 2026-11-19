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
//! tabs in place, so the tab with the keyboard keeps it; one Tab stop per
//! tab (the × is for the pointer; the menu closes a panel); and
//! Ctrl+Page Up / Ctrl+Page Down show the previous and next tab while the
//! keyboard is anywhere in the frame.
//!
//! A panel can show its own tabs in place of its one tab, given that
//! window's panel widget, so each window's panel shows its own: the Code
//! panel's open files appear this way, one tab each (name, and a yellow ●
//! while unsaved), beside the Model's tab in the frame they share. Its
//! owner registers where a chosen tab goes once ([`set_page_tabs`], a
//! function that only sends a message) and hands over the tabs as data
//! whenever they change ([`show_page_tabs`]); nothing here asks the owner
//! for them. Clicking one shows the panel on that file.
//!
//! The strip is a GObject subclass, [`TabStrip`], because libpanel takes a
//! frame's header as a widget implementing `PanelFrameHeader`. Its tabs
//! are a relm4 component, [`TabStripComponent`] (docs/relm4-migration.md):
//! the component's model is the only state, the tabs' buttons and keys
//! only send [`TabStripInput`] messages, and showing one of a panel's own
//! tabs is a [`TabStripOutput`]. The subclass owns the component: it
//! gathers the tabs, from the frame's pages and the panels' own tabs
//! (data, as last handed over), when the main loop next runs, never
//! inside whatever changed them, and sends them as
//! [`TabStripInput::SetTabs`]; another page
//! shown is only [`TabStripInput::PageChanged`], which restyles the tabs
//! already made.

use stave_app::i18n::{fill_in, tr};
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

/// A panel's own tabs, as last handed over, and where a chosen one goes.
struct PageTabs {
    /// The tabs.
    tabs: Vec<PageTab>,
    /// Shows the tab at this place among them (the panel is brought to
    /// the front first): it only sends its owner a message.
    show: Rc<dyn Fn(usize)>,
}

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

/// Lets this panel show its own tabs (once [`show_page_tabs`] hands any
/// over) in place of its one tab, in whatever frame holds it; choosing
/// one calls `show` with its place among them. `show` should only send a
/// message: it is called after the strip's update, from the main loop.
pub fn set_page_tabs(panel: &panel::Widget, show: impl Fn(usize) + 'static) {
    PAGE_TABS.with(|all| {
        let mut all = all.borrow_mut();
        all.retain(|(widget, _)| widget.upgrade().is_some_and(|widget| widget != *panel));
        all.push((
            panel.downgrade(),
            PageTabs {
                tabs: Vec::new(),
                show: Rc::new(show),
            },
        ));
    });
    page_tabs_changed();
}

/// This panel's own tabs are now `tabs`: every strip shows them when the
/// main loop next runs. A panel never given to [`set_page_tabs`] is left
/// as it is.
pub fn show_page_tabs(panel: &panel::Widget, tabs: Vec<PageTab>) {
    let changed = PAGE_TABS.with(|all| {
        let mut all = all.borrow_mut();
        match all
            .iter_mut()
            .find(|(widget, _)| widget.upgrade().is_some_and(|widget| widget == *panel))
        {
            Some((_, own)) if own.tabs != tabs => {
                own.tabs = tabs;
                true
            }
            Some(_) | None => false,
        }
    });
    if changed {
        page_tabs_changed();
    }
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

/// A panel's own tabs, if it has any: a copy, so nothing is borrowed.
fn page_tabs(panel: &panel::Widget) -> Vec<PageTab> {
    PAGE_TABS.with(|all| {
        all.borrow()
            .iter()
            .find(|(widget, _)| widget.upgrade().is_some_and(|widget| widget == *panel))
            .map(|(_, own)| own.tabs.clone())
            .unwrap_or_default()
    })
}

/// Where a panel's chosen own tab goes, cloned out of the registry.
fn page_tab_route(panel: &panel::Widget) -> Option<Rc<dyn Fn(usize)>> {
    PAGE_TABS.with(|all| {
        all.borrow()
            .iter()
            .find(|(widget, _)| widget.upgrade().is_some_and(|widget| widget == *panel))
            .map(|(_, own)| Rc::clone(&own.show))
    })
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

/// Whether a panel's tab can close it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Closing {
    /// A × closes the panel.
    Closable,
    /// No ×: the 3D view can't be closed (its tab is still dragged).
    Fixed,
}

/// Which way Ctrl+Page Up and Ctrl+Page Down step through a frame's
/// pages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// The page before the shown one (the last after the first).
    Previous,
    /// The page after the shown one (the first after the last).
    Next,
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

/// The strip's tabs: the frame they follow and the tabs last gathered.
pub struct TabStripComponent {
    /// The frame whose pages are the tabs.
    frame: glib::WeakRef<panel::Frame>,
    /// Watching the frame's pages, to gather the tabs again.
    watch: Option<(gtk::SelectionModel, glib::SignalHandlerId)>,
    /// Ctrl+Page Up / Down on the frame, and the frame it is on.
    keys: Option<(glib::WeakRef<panel::Frame>, gtk::ShortcutController)>,
    /// The tabs, as shown.
    tabs: Vec<PageTabLabel>,
    /// The tabs' widgets, one per entry of `tabs`.
    made: Vec<MadeTab>,
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
    /// Show these tabs (pages came or went, or a panel's own tabs
    /// changed): the tabs are made again, and the one at the place the
    /// keyboard was takes it back.
    SetTabs(Vec<PageTabLabel>),
    /// The frame shows another page: the tabs already made are restyled
    /// in place, so the tab with the keyboard keeps it.
    PageChanged,
    /// People pressed Ctrl+Page Up or Ctrl+Page Down in the frame.
    Step(Step),
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
    /// Follows a frame's pages (or none), with Ctrl+Page Up / Down on
    /// it, and has the tabs gathered.
    fn follow(&mut self, frame: Option<&panel::Frame>, sender: &ComponentSender<Self>) {
        self.stop_watching();
        self.frame.set(frame);
        if let Some(frame) = frame {
            let pages = frame.pages();
            // Captures nothing: whichever strips are alive gather again.
            let handler = pages.connect_items_changed(|_, _, _, _| page_tabs_changed());
            self.watch = Some((pages, handler));
            let keys = step_keys(sender);
            frame.add_controller(keys.clone());
            self.keys = Some((frame.downgrade(), keys));
        }
        page_tabs_changed();
    }

    /// Stops watching the frame's pages and takes the keys off it.
    fn stop_watching(&mut self) {
        if let Some((pages, handler)) = self.watch.take() {
            pages.disconnect(handler);
        }
        if let Some((frame, keys)) = self.keys.take()
            && let Some(frame) = frame.upgrade()
        {
            frame.remove_controller(&keys);
        }
    }

    /// Makes the tabs again: one per page (or one per own tab), the shown
    /// one marked, and the shown panel's menu at the end. The tab at the
    /// place the keyboard was takes it back.
    fn show_tabs(&mut self, root: &gtk::Box, sender: &ComponentSender<Self>) {
        let focused = self.made.iter().position(|made| made.show.has_focus());
        while let Some(child) = root.first_child() {
            root.remove(&child);
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
                TabKind::Own(own) => own_tab(place, own, &page, sender),
            };
            root.append(&made.root);
            self.made.push(made);
        }
        if let Some(place) = focused
            && let Some(made) = self.made.get(place.min(self.made.len().saturating_sub(1)))
        {
            made.show.grab_focus();
        }
        self.mark_shown();
    }

    /// Marks the shown page's tab, in place (the tab the keyboard is on
    /// stays where it is), and puts the shown panel's menu at the end. A
    /// panel's own tab is marked only when it is its panel's current one.
    fn mark_shown(&mut self) {
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
                        TabKind::Own(own) => own.active,
                    }
            });
            made.mark(marked);
        }
        let menu = shown.as_ref().and_then(|page| page.menu_model());
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
        gtk::Box::builder()
            .accessible_role(gtk::AccessibleRole::TabList)
            .build() -> gtk::Box {
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
            keys: None,
            tabs: Vec::new(),
            made: Vec::new(),
            menu,
        };
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: TabStripInput, sender: ComponentSender<Self>, root: &Self::Root) {
        match message {
            TabStripInput::SetFrame(frame) => self.follow(frame.as_ref(), &sender),
            TabStripInput::SetTabs(tabs) => {
                self.tabs = tabs;
                self.show_tabs(root, &sender);
            }
            TabStripInput::PageChanged => self.mark_shown(),
            TabStripInput::Step(step) => {
                // The frame's page-changed only queues `PageChanged`.
                if let Some(frame) = self.frame.upgrade() {
                    show_step(&frame, step);
                }
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

/// Shows one of a panel's own tabs: its owner is told which, outside any
/// borrow of the registry.
fn show_own_tab(output: TabStripOutput) {
    match output {
        TabStripOutput::Show { page, index } => {
            let Some(page) = page.upgrade() else {
                return;
            };
            if let Some(show) = page_tab_route(&page) {
                show(index);
            }
        }
    }
}

/// Whether a panel's tab has a ×: the 3D view's has none.
fn closing_of(page: &panel::Widget) -> Closing {
    // Read as a property: `id()` asserts on a panel that has none.
    let id = page.property::<Option<String>>("id");
    if id.as_deref() == Some(super::workbench::PanelId::View.word()) {
        Closing::Fixed
    } else {
        Closing::Closable
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
                    closing: closing_of(&page),
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
            self.menu
                .set_tooltip_text(Some(&tr("Move or close this panel")));
            self.menu
                .update_property(&[gtk::accessible::Property::Label(&tr("Panel menu"))]);
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
            // Only restyled: the tab the keyboard is on stays where it is.
            if let Some(tabs) = self.tabs.get() {
                tabs.send(TabStripInput::PageChanged);
            }
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

/// Ctrl+Page Up and Ctrl+Page Down: the previous and the next tab of the
/// frame the keyboard is in. After the focused widget had its turn (the
/// text editor keeps its own Ctrl+Page keys), and passed on when the
/// frame has fewer than two pages.
fn step_keys(sender: &ComponentSender<TabStripComponent>) -> gtk::ShortcutController {
    let keys = gtk::ShortcutController::new();
    for (trigger, step) in [
        ("<Control>Page_Up", Step::Previous),
        ("<Control>Page_Down", Step::Next),
    ] {
        let input = sender.input_sender().clone();
        // Reads only the frame the key came to, and sends.
        let action = gtk::CallbackAction::new(move |widget, _| {
            match widget.downcast_ref::<panel::Frame>() {
                Some(frame) if frame.n_pages() >= 2 => {
                    input.emit(TabStripInput::Step(step));
                    glib::Propagation::Stop
                }
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

/// Shows the page a `step` from the shown one, going round. `false` when
/// there is only one page or none.
fn show_step(frame: &panel::Frame, step: Step) -> bool {
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
    let next = match step {
        Step::Previous => current.checked_sub(1).unwrap_or(count - 1),
        Step::Next => current.saturating_add(1) % count,
    };
    let Some(page) = frame.page(next) else {
        return false;
    };
    frame.set_visible_child(&page);
    true
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

    use super::*;
    use crate::ui::gtk_test::{on_gtk_thread, settle, unnamed_buttons};

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
        set_page_tabs(&code, move |index| {
            let first = log.borrow().is_empty();
            log.borrow_mut().push(index);
            if first && let Some(strip) = weak_strip.upgrade() {
                // GTK firing another tab's click from in here only queues
                // a message.
                show_button(&strip, 2).emit_clicked();
            }
        });
        show_page_tabs(
            &code,
            (0..2)
                .map(|index| PageTab {
                    title: format!("file{index}.stave"),
                    tooltip: String::new(),
                    unsaved: index == 1,
                    mark: None,
                    active: index == 0,
                })
                .collect(),
        );
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

    #[test]
    fn the_models_tab_has_no_close_button() {
        on_gtk_thread(model_tab_is_fixed);
    }

    /// The 3D view can't be closed: its tab has no × (and is marked fixed,
    /// still dragged); another panel's tab has one.
    fn model_tab_is_fixed() {
        panel::init();
        let frame = panel::Frame::new();
        let strip = TabStrip::new();
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
        settle();
        let before = tab_boxes(&strip);
        assert_eq!(before.len(), 3);
        let Some(tabs) = strip.imp().tabs.get() else {
            panic!("the strip has its tabs");
        };
        assert_eq!(
            tabs.widget().accessible_role(),
            gtk::AccessibleRole::TabList
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
        // Ctrl+Page Down, then round from the last to the first.
        assert!(show_step(&frame, Step::Next));
        assert_eq!(frame.visible_child().as_ref(), Some(&pages[2]));
        assert!(show_step(&frame, Step::Next));
        assert_eq!(frame.visible_child().as_ref(), Some(&pages[0]));
        assert!(show_step(&frame, Step::Previous));
        assert_eq!(frame.visible_child().as_ref(), Some(&pages[2]));
        // The keys reach the frame by message; nothing happens until it
        // is handled.
        tabs.send(TabStripInput::Step(Step::Next));
        settle();
        assert_eq!(frame.visible_child().as_ref(), Some(&pages[0]));
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

    #[test]
    fn ctrl_page_keys_pass_on_with_one_page() {
        on_gtk_thread(one_page_passes_keys_on);
    }

    /// With fewer than two pages there is nowhere to step: the frame does
    /// nothing, so the key goes on to whatever else wants it.
    fn one_page_passes_keys_on() {
        panel::init();
        let frame = panel::Frame::new();
        let only = panel::Widget::builder().title("Parts").build();
        frame.add(&only);
        assert!(!show_step(&frame, Step::Next));
        assert!(!show_step(&frame, Step::Previous));
        assert_eq!(frame.visible_child().as_ref(), Some(&only));
    }
}
