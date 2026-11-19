//! The title bar, one slim row (34 px): at the left a menu bar (Project,
//! Edit, View, Help, made from the main menu) and the project's name in
//! small grey words; the Design / Cut list switcher in the middle; at the
//! right a find button (the finder, Ctrl+K), the problems chip (the status
//! pill between previous and next, stepping through the problems) and the
//! build button. Back and forward live in the View menu and on Alt+← and
//! Alt+→. In a very narrow window the menu bar folds into one menu
//! button. It also makes the scene menu, which floats on the 3D view. It
//! only shows what the session says; the window wires its buttons to
//! session commands.
//!
//! It is a relm4 component, [`HeaderComponent`]: its model (what the
//! session last said) is the only state, only its update changes it, and
//! its widgets' handlers only send [`HeaderInput`] messages. [`Header`]
//! keeps the older API in front of it until the window moves to relm4
//! too. The window still reaches into several of the bar's widgets (the
//! breakpoint hides the title and swaps the menus, the dock holds the
//! scene menu, the switcher and the scene list are connected directly),
//! so [`Header`] makes them and hands them to the component at launch.

use adw::prelude::*;
use gtk::{gio, glib};
use relm4::{Component, ComponentParts, ComponentSender};
use stave_app::session::{Activity, Build, Pill, Tone, View};

use super::icons::{self, PathIcon};
use super::relm4_shim::Bridge;

/// `true` for a view that is made: one the switcher and its shortcut may
/// lead to. A view that isn't made stays out of reach rather than show a
/// page that does nothing.
pub fn ready(view: View) -> bool {
    match view {
        View::Design => true,
        View::CutList => false,
    }
}

/// What the problems chip shows: the pill, how many problems there are
/// to step through, and the status line for its tooltip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PillShown {
    /// The pill's colour, words and whether it spins.
    pub pill: Pill,
    /// How many problems there are.
    pub problems: usize,
    /// What the last check found, in words.
    pub line: String,
}

/// The scenes to choose from and the one showing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScenesShown {
    /// Each scene's title.
    pub titles: Vec<String>,
    /// The one showing, by its place in `titles`.
    pub current: usize,
}

/// The title bar's state: what the session last said. `None` keeps the
/// look the bar was made with ("Not checked yet", the folder's name).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeaderComponent {
    /// The project's name and its folder.
    title: Option<(String, String)>,
    /// The view showing.
    view: View,
    scenes: Option<ScenesShown>,
    pill: Option<PillShown>,
    /// The build button's state and tooltip.
    build: Option<(Build, String)>,
}

/// What the title bar is told.
#[derive(Debug)]
pub enum HeaderInput {
    /// Show the project's name, and its folder in the tooltip.
    ShowTitle {
        /// The project's name.
        title: String,
        /// Its folder, as shown.
        folder: String,
    },
    /// Show this view in the switcher, without telling anyone.
    ShowView(View),
    /// Fill the scene menu.
    ShowScenes(ScenesShown),
    /// Show the problems chip.
    ShowPill(PillShown),
    /// Show the build button building or ready, with this tooltip.
    ShowBuild {
        /// Building or ready.
        build: Build,
        /// What it does, or why there is nothing to build.
        tooltip: String,
    },
    /// People chose a view in the switcher.
    ViewChosen(View),
    /// People chose the scene at this place in the scene menu.
    SceneChosen(usize),
}

/// What the title bar tells its owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeaderOutput {
    /// People chose this view.
    View(View),
    /// People chose the scene at this place in the scene menu.
    Scene(usize),
}

/// The bar's widgets, made by [`Header`] (the window still reaches into
/// them) and handed to the component, which owns their state.
#[derive(Clone, Debug)]
pub struct HeaderParts {
    menu_bar: gtk::PopoverMenuBar,
    menu_button: gtk::MenuButton,
    title: gtk::Label,
    find: gtk::Button,
    switcher: adw::ToggleGroup,
    switcher_labels: Vec<gtk::Label>,
    scene: gtk::MenuButton,
    scene_label: gtk::Label,
    scenes: gtk::ListBox,
    chip: gtk::Box,
    pill: gtk::Button,
    previous: gtk::Button,
    next: gtk::Button,
    pill_icon: PathIcon,
    pill_spinner: adw::Spinner,
    pill_stack: gtk::Stack,
    pill_label: gtk::Label,
    build: gtk::Button,
    build_stack: gtk::Stack,
}

/// The component's widgets: the parts, and the switcher's handler so a
/// view the session shows doesn't come back as people's choice.
#[derive(Debug)]
pub struct HeaderWidgets {
    parts: HeaderParts,
    view_chosen: glib::SignalHandlerId,
}

impl HeaderParts {
    /// The bar's widgets for a project in `folder_label`, its menus made
    /// of `menus`.
    fn new(folder_label: &str, menus: &gio::Menu) -> HeaderParts {
        let menu_bar = gtk::PopoverMenuBar::from_model(Some(menus));
        menu_bar.add_css_class("stave-menu-bar");
        menu_bar.set_valign(gtk::Align::Center);
        let menu_button = gtk::MenuButton::builder()
            .child(&PathIcon::new(icons::MENU, 16))
            .menu_model(menus)
            .tooltip_text("Main menu")
            .css_classes(["flat", "stave-icon-button"])
            .valign(gtk::Align::Center)
            .primary(true)
            .visible(false)
            .build();

        let title = gtk::Label::builder()
            .label(folder_label)
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .max_width_chars(28)
            .valign(gtk::Align::Center)
            .css_classes(["stave-project-name"])
            .build();

        let find = gtk::Button::builder()
            .icon_name("system-search-symbolic")
            .tooltip_text("Find a part, file, line or problem (Ctrl K)")
            .action_name("win.find")
            .css_classes(["flat", "stave-icon-button", "stave-find"])
            .valign(gtk::Align::Center)
            .build();
        find.update_property(&[gtk::accessible::Property::Label(
            "Find a part, file, line or problem",
        )]);

        let switcher = adw::ToggleGroup::builder()
            .css_classes(["stave-switcher"])
            .homogeneous(true)
            .valign(gtk::Align::Center)
            .build();
        let mut switcher_labels = Vec::new();
        for view in View::ALL {
            let shape = match view {
                View::Design => icons::PARTS,
                View::CutList => icons::CUT_LIST,
            };
            let content = gtk::Box::builder()
                .halign(gtk::Align::Center)
                .valign(gtk::Align::Center)
                .build();
            content.append(&PathIcon::new(shape, 15));
            let label = gtk::Label::new(Some(view.title()));
            content.append(&label);
            switcher_labels.push(label);
            let toggle = adw::Toggle::builder()
                .name(view.word())
                .child(&content)
                .tooltip(view.title())
                .build();
            switcher.add(toggle);
        }
        switcher.set_active_name(Some(View::Design.word()));
        // The Cut list view isn't made yet, so the design is the only
        // view: the switcher stays hidden until there is a second one.
        switcher.set_visible(View::ALL.into_iter().filter(|view| ready(*view)).count() > 1);

        let scene_label = gtk::Label::new(Some("Rest"));
        let scene_content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let scene_icon = PathIcon::new(icons::SCENE, 15);
        scene_icon.add_css_class("dim");
        let chevron = PathIcon::new(icons::CHEVRON_DOWN, 10);
        chevron.add_css_class("dim");
        scene_content.append(&scene_icon);
        scene_content.append(&scene_label);
        scene_content.append(&chevron);
        let scenes = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["navigation-sidebar"])
            .build();
        let popover = gtk::Popover::builder().child(&scenes).build();
        let scene = gtk::MenuButton::builder()
            .child(&scene_content)
            .popover(&popover)
            .tooltip_text("Scene")
            .css_classes(["stave-scene"])
            .valign(gtk::Align::Center)
            .build();

        let pill_icon = PathIcon::new(icons::PENDING, 16);
        let pill_spinner = adw::Spinner::builder()
            .width_request(14)
            .height_request(14)
            .valign(gtk::Align::Center)
            .build();
        let pill_stack = gtk::Stack::builder()
            .hhomogeneous(true)
            .vhomogeneous(true)
            .valign(gtk::Align::Center)
            .build();
        pill_stack.add_named(&pill_icon, Some("icon"));
        pill_stack.add_named(&pill_spinner, Some("spinner"));
        let pill_label = gtk::Label::new(Some("Not checked yet"));
        let pill_content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        pill_content.append(&pill_stack);
        pill_content.append(&pill_label);
        let pill = gtk::Button::builder()
            .child(&pill_content)
            .css_classes(["stave-pill"])
            .tooltip_text("Not checked yet")
            .action_name("win.next-problem")
            .valign(gtk::Align::Center)
            .build();
        let previous = gtk::Button::builder()
            .icon_name("go-previous-symbolic")
            .tooltip_text("Previous problem (Shift+F8)")
            .action_name("win.previous-problem")
            .css_classes(["stave-problem-step"])
            .valign(gtk::Align::Center)
            .visible(false)
            .build();
        let next = gtk::Button::builder()
            .icon_name("go-next-symbolic")
            .tooltip_text("Next problem (F8)")
            .action_name("win.next-problem")
            .css_classes(["stave-problem-step"])
            .valign(gtk::Align::Center)
            .visible(false)
            .build();
        let chip = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .css_classes(["stave-problems-chip", "unknown"])
            .valign(gtk::Align::Center)
            .build();
        chip.append(&previous);
        chip.append(&pill);
        chip.append(&next);

        let hammer = PathIcon::new(icons::HAMMER, 18);
        let building = gtk::Overlay::new();
        let small_hammer = PathIcon::new(icons::HAMMER, 12);
        small_hammer.set_opacity(0.9);
        building.set_child(Some(&small_hammer));
        let ring = adw::Spinner::builder()
            .width_request(24)
            .height_request(24)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .build();
        building.add_overlay(&ring);
        let build_stack = gtk::Stack::builder()
            .hhomogeneous(true)
            .vhomogeneous(true)
            .build();
        build_stack.add_named(&hammer, Some("ready"));
        build_stack.add_named(&building, Some("building"));
        let build = gtk::Button::builder()
            .child(&build_stack)
            .tooltip_text("Build plan (Ctrl+B)")
            .css_classes(["stave-build"])
            .valign(gtk::Align::Center)
            .build();
        build.update_property(&[gtk::accessible::Property::Label("Build plan")]);
        build.set_action_name(Some("win.build"));
        // The build plan isn't made yet: pressing the button would only
        // say so. It stays hidden (and `win.build` unregistered) until it
        // builds a plan.
        build.set_visible(false);

        HeaderParts {
            menu_bar,
            menu_button,
            title,
            find,
            switcher,
            switcher_labels,
            scene,
            scene_label,
            scenes,
            chip,
            pill,
            previous,
            next,
            pill_icon,
            pill_spinner,
            pill_stack,
            pill_label,
            build,
            build_stack,
        }
    }
}

impl HeaderComponent {
    /// Applies the model to the widgets. Only what the session has said
    /// is applied; the rest keeps the look the bar was made with.
    fn show(&self, widgets: &HeaderWidgets) {
        let parts = &widgets.parts;
        if let Some((title, folder)) = &self.title {
            parts.title.set_label(title);
            parts.title.set_tooltip_text(Some(folder));
        }
        // The session showing a view isn't people choosing one.
        if parts.switcher.active_name().as_deref() != Some(self.view.word()) {
            parts.switcher.block_signal(&widgets.view_chosen);
            parts.switcher.set_active_name(Some(self.view.word()));
            parts.switcher.unblock_signal(&widgets.view_chosen);
        }
        if let Some(scenes) = &self.scenes {
            parts.scene_label.set_label(
                scenes
                    .titles
                    .get(scenes.current)
                    .map_or("Rest", String::as_str),
            );
            parts.scene.set_sensitive(!scenes.titles.is_empty());
        }
        if let Some(shown) = &self.pill {
            show_pill(parts, shown);
        }
        if let Some((build, tooltip)) = &self.build {
            match build {
                Build::Building => {
                    parts.build.add_css_class("building");
                    parts.build_stack.set_visible_child_name("building");
                }
                Build::Idle => {
                    parts.build.remove_css_class("building");
                    parts.build_stack.set_visible_child_name("ready");
                }
            }
            parts.build.set_tooltip_text(Some(tooltip));
        }
    }
}

/// The problems chip: the pill's colour and words, the arrows when there
/// are problems to step through, and the status line (what the last check
/// found) in its tooltip.
fn show_pill(parts: &HeaderParts, shown: &PillShown) {
    let PillShown {
        pill,
        problems,
        line,
    } = shown;
    let (class, shape) = match pill.tone {
        Tone::Clean => ("clean", icons::CHECK),
        Tone::Warnings => ("warnings", icons::ALERT),
        Tone::Errors => ("errors", icons::ALERT),
        Tone::Unknown => ("unknown", icons::PENDING),
    };
    for other in ["clean", "warnings", "errors", "unknown"] {
        if other != class {
            parts.chip.remove_css_class(other);
        }
    }
    parts.chip.add_css_class(class);
    parts.previous.set_visible(*problems > 1);
    parts.next.set_visible(*problems > 1);
    parts.pill.set_sensitive(*problems > 0);
    parts.pill_icon.set_shape(shape);
    parts.pill_label.set_label(&pill.label);
    let checking = pill.activity == Activity::Checking;
    parts
        .pill_stack
        .set_visible_child_name(if checking { "spinner" } else { "icon" });
    parts.pill_spinner.set_visible(checking);
    let tooltip = if *problems > 0 {
        format!("{line}. Show the next problem (F8)")
    } else {
        line.clone()
    };
    parts.pill.set_tooltip_text(Some(&tooltip));
    parts
        .pill
        .update_property(&[gtk::accessible::Property::Label(&tooltip)]);
}

/// Fills the scene menu: a row per title, the current one in bold.
fn fill_scenes(scenes: &gtk::ListBox, shown: &ScenesShown) {
    while let Some(row) = scenes.row_at_index(0) {
        scenes.remove(&row);
    }
    for (index, title) in shown.titles.iter().enumerate() {
        let label = gtk::Label::builder()
            .label(title)
            .xalign(0.0)
            .margin_start(6)
            .margin_end(6)
            .build();
        let row = gtk::ListBoxRow::builder().child(&label).build();
        if index == shown.current {
            label.add_css_class("heading");
        }
        scenes.append(&row);
    }
}

impl Component for HeaderComponent {
    type Init = HeaderParts;
    type Input = HeaderInput;
    type Output = HeaderOutput;
    type CommandOutput = ();
    type Root = adw::HeaderBar;
    type Widgets = HeaderWidgets;

    fn init_root() -> adw::HeaderBar {
        adw::HeaderBar::builder()
            .css_classes(["stave-header"])
            .build()
    }

    fn init(
        parts: HeaderParts,
        root: adw::HeaderBar,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        root.set_title_widget(Some(&parts.switcher));
        root.pack_start(&parts.menu_button);
        root.pack_start(&parts.menu_bar);
        root.pack_start(&parts.title);
        root.pack_end(&parts.build);
        root.pack_end(&parts.chip);
        root.pack_end(&parts.find);

        let view_chosen = {
            let sender = sender.clone();
            parts.switcher.connect_active_name_notify(move |group| {
                if let Some(view) = group.active_name().and_then(|name| View::from_word(&name)) {
                    sender.input_sender().emit(HeaderInput::ViewChosen(view));
                }
            })
        };
        parts.scenes.connect_row_activated(move |_, row| {
            if let Ok(index) = usize::try_from(row.index()) {
                sender.input_sender().emit(HeaderInput::SceneChosen(index));
            }
        });

        let model = HeaderComponent {
            title: None,
            view: View::Design,
            scenes: None,
            pill: None,
            build: None,
        };
        let widgets = HeaderWidgets { parts, view_chosen };
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: HeaderInput, sender: ComponentSender<Self>, _root: &Self::Root) {
        let output = match message {
            HeaderInput::ShowTitle { title, folder } => {
                self.title = Some((title, folder));
                None
            }
            HeaderInput::ShowView(view) => {
                self.view = view;
                None
            }
            HeaderInput::ShowScenes(scenes) => {
                self.scenes = Some(scenes);
                None
            }
            HeaderInput::ShowPill(pill) => {
                self.pill = Some(pill);
                None
            }
            HeaderInput::ShowBuild { build, tooltip } => {
                self.build = Some((build, tooltip));
                None
            }
            HeaderInput::ViewChosen(view) => {
                self.view = view;
                Some(HeaderOutput::View(view))
            }
            HeaderInput::SceneChosen(index) => Some(HeaderOutput::Scene(index)),
        };
        if let Some(output) = output {
            // Nobody listening is not a mistake: the owner may not want
            // to hear.
            let _ = sender.output(output);
        }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut HeaderWidgets,
        message: HeaderInput,
        sender: ComponentSender<Self>,
        root: &Self::Root,
    ) {
        let refill = matches!(message, HeaderInput::ShowScenes(_));
        self.update(message, sender.clone(), root);
        if refill && let Some(scenes) = &self.scenes {
            fill_scenes(&widgets.parts.scenes, scenes);
        }
        self.update_view(widgets, sender);
    }

    fn update_view(&self, widgets: &mut HeaderWidgets, _sender: ComponentSender<Self>) {
        self.show(widgets);
    }
}

/// The title bar, behind the API the window still uses: it owns the
/// component (dropping it shuts the component down), passes its setters
/// on as [`HeaderInput`] messages and keeps the widgets the window
/// reaches into.
///
/// Setters are queued: they take effect when the main loop next runs,
/// after whatever called them has returned.
pub struct Header {
    /// The bar itself.
    pub bar: adw::HeaderBar,
    /// The menu bar: Project, Edit, View, Help.
    pub menu_bar: gtk::PopoverMenuBar,
    /// The same menus in one button, shown instead of the menu bar when
    /// the window is very narrow.
    pub menu_button: gtk::MenuButton,
    /// The project's name, small and grey; its folder in the tooltip.
    pub title: gtk::Label,
    /// Design / Cut list.
    pub switcher: adw::ToggleGroup,
    switcher_labels: Vec<gtk::Label>,
    /// The scene menu (shown in the dock).
    pub scene: gtk::MenuButton,
    /// The scenes, in the scene menu's popover.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the window hears the scene chosen as an output; tests read the rows"
        )
    )]
    pub scenes: gtk::ListBox,
    bridge: Bridge<HeaderComponent>,
}

impl Header {
    /// A title bar for a project in `folder_label` (shown as its name
    /// until the design is loaded), its menu bar made of `menus`: one
    /// submenu each for Project, Edit, View and Help.
    pub fn new(folder_label: &str, menus: &gio::Menu) -> Header {
        let parts = HeaderParts::new(folder_label, menus);
        let menu_bar = parts.menu_bar.clone();
        let menu_button = parts.menu_button.clone();
        let title = parts.title.clone();
        let switcher = parts.switcher.clone();
        let switcher_labels = parts.switcher_labels.clone();
        let scene = parts.scene.clone();
        let scenes = parts.scenes.clone();
        let bridge = Bridge::<HeaderComponent>::launch(parts);
        Header {
            bar: bridge.widget().clone(),
            menu_bar,
            menu_button,
            title,
            switcher,
            switcher_labels,
            scene,
            scenes,
            bridge,
        }
    }

    /// Calls `callback` with each of the header's outputs.
    pub fn connect(&self, callback: impl Fn(HeaderOutput) + 'static) {
        self.bridge.connect(callback);
    }

    /// The project's name, and its folder in the tooltip.
    pub fn show_title(&self, title: &str, folder: &str) {
        self.bridge.send(HeaderInput::ShowTitle {
            title: title.to_owned(),
            folder: folder.to_owned(),
        });
    }

    /// The view showing.
    pub fn show_view(&self, view: View) {
        self.bridge.send(HeaderInput::ShowView(view));
    }

    /// The switcher's words, hidden when the window is narrow.
    pub fn switcher_labels(&self) -> &[gtk::Label] {
        &self.switcher_labels
    }

    /// The scenes to choose from (titles) and the one showing.
    pub fn show_scenes(&self, titles: &[String], current: usize) {
        self.bridge.send(HeaderInput::ShowScenes(ScenesShown {
            titles: titles.to_vec(),
            current,
        }));
    }

    /// The problems chip: the pill's colour and words, the arrows when
    /// there are problems to step through, and the status line (what the
    /// last check found) in its tooltip.
    pub fn show_pill(&self, pill: &Pill, problems: usize, line: &str) {
        self.bridge.send(HeaderInput::ShowPill(PillShown {
            pill: pill.clone(),
            problems,
            line: line.to_owned(),
        }));
    }

    /// The build button: building (spinning) or ready, with the tooltip
    /// the session gives (what it does, or why there is nothing to build).
    pub fn show_build(&self, build: Build, tooltip: &str) {
        self.bridge.send(HeaderInput::ShowBuild {
            build,
            tooltip: tooltip.to_owned(),
        });
    }

    /// The component's model, as of the last message it handled.
    #[cfg(test)]
    fn model(&self) -> std::cell::Ref<'_, HeaderComponent> {
        self.bridge.model()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;
    use crate::ui::gtk_test::{on_gtk_thread, settle};

    fn header() -> Header {
        Header::new("hall_table", &gio::Menu::new())
    }

    #[test]
    fn a_dropped_header_is_freed() {
        on_gtk_thread(header_is_freed);
    }

    /// Dropping the header shuts its component down when the main loop
    /// next runs: the bar, the model (and its runtime) and the callback
    /// are all freed.
    fn header_is_freed() {
        let header = header();
        let token = Rc::new(());
        let held = Rc::clone(&token);
        header.connect(move |_| {
            let _ = &held;
        });
        let callback = Rc::downgrade(&token);
        drop(token);
        let bar = header.bar.downgrade();
        let switcher = header.switcher.downgrade();
        let input = header.bridge.sender().clone();
        settle();
        assert!(bar.upgrade().is_some(), "the bar lives while held");
        drop(header);
        settle();
        assert!(bar.upgrade().is_none(), "the bar is freed");
        assert!(switcher.upgrade().is_none(), "its widgets are freed");
        assert!(
            input.send(HeaderInput::ShowView(View::Design)).is_err(),
            "the component's runtime, and with it the model, is gone"
        );
        assert!(callback.upgrade().is_none(), "the callback is freed");
    }

    #[test]
    fn setters_show_what_the_session_says_without_calling_back() {
        on_gtk_thread(setters_do_not_call_back);
    }

    fn setters_do_not_call_back() {
        let header = header();
        let heard: Rc<RefCell<Vec<HeaderOutput>>> = Rc::default();
        let log = Rc::clone(&heard);
        header.connect(move |output| log.borrow_mut().push(output));
        header.show_title("Hall table", "~/Projects/hall_table");
        header.show_view(View::CutList);
        header.show_scenes(&["Rest".to_owned(), "Open".to_owned()], 1);
        header.show_pill(
            &Pill {
                tone: Tone::Errors,
                label: "2 problems".to_owned(),
                activity: Activity::Idle,
            },
            2,
            "2 problems",
        );
        header.show_build(Build::Building, "Building the plan");
        assert_eq!(header.title.label(), "hall_table", "only queued so far");
        settle();
        assert_eq!(header.title.label(), "Hall table");
        assert_eq!(
            header.title.tooltip_text().as_deref(),
            Some("~/Projects/hall_table")
        );
        assert_eq!(
            header.switcher.active_name().as_deref(),
            Some(View::CutList.word())
        );
        assert!(header.scenes.row_at_index(1).is_some());
        assert!(header.scenes.row_at_index(2).is_none());
        assert_eq!(header.model().view, View::CutList);
        assert!(heard.borrow().is_empty(), "setters never call back");
    }

    #[test]
    fn a_view_chosen_while_handling_an_output_is_queued_not_re_entered() {
        on_gtk_thread(choices_are_queued);
    }

    /// People choosing a view only queues a message; its output reaches
    /// the callback after the update has returned (reading the model
    /// there would panic if it had not), and a choice made from inside
    /// the callback is queued too.
    fn choices_are_queued() {
        let header = Rc::new(header());
        let heard: Rc<RefCell<Vec<(HeaderOutput, View)>>> = Rc::default();
        let log = Rc::clone(&heard);
        let weak = Rc::downgrade(&header);
        header.connect(move |output| {
            let Some(header) = weak.upgrade() else {
                return;
            };
            let first = log.borrow().is_empty();
            if first {
                // GTK firing the switcher's signal from in here only
                // queues a message: the model doesn't change under us.
                header.switcher.set_active_name(Some(View::Design.word()));
            }
            log.borrow_mut().push((output, header.model().view));
        });

        header.switcher.set_active_name(Some(View::CutList.word()));
        assert!(heard.borrow().is_empty(), "nothing has run yet");
        assert_eq!(
            header.model().view,
            View::Design,
            "the choice is only queued"
        );
        settle();
        assert_eq!(
            *heard.borrow(),
            vec![
                (HeaderOutput::View(View::CutList), View::CutList),
                (HeaderOutput::View(View::Design), View::Design),
            ],
            "each output after its update, the inner choice after the first"
        );

        // The session showing a view isn't people choosing it.
        header.show_view(View::CutList);
        settle();
        assert_eq!(heard.borrow().len(), 2, "showing a view never calls back");
        assert_eq!(
            header.switcher.active_name().as_deref(),
            Some(View::CutList.word())
        );
    }
}
