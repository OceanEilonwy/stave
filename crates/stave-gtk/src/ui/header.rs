//! The title bar, one slim row (34 px): at the left a menu bar (Project,
//! Edit, View, Help, made from the main menu); in the middle the
//! project's name over its folder (an `adw::WindowTitle`, as libadwaita
//! centres a window's title), or the Design / Cut list switcher once
//! there is more than one view; at the right a find button (the finder,
//! Ctrl+K), the problems chip (the status pill between previous and next,
//! stepping through the problems) and the build button. Back and forward
//! live in the View menu and on Alt+← and Alt+→. In a very narrow window the menu bar folds into one menu
//! button. It also makes the scene menu, which floats on the 3D view. It
//! only shows what the session says; the window wires its buttons to
//! session commands.
//!
//! It is a relm4 component, [`HeaderComponent`]: its model (what the
//! session last said) is the only state, only its update changes it, and
//! its widgets' handlers only send [`HeaderInput`] messages. [`Header`]
//! owns its controller and keeps the setters as plain methods; people's
//! choices go to the owner as the message it chose when it made the bar.
//! The window still reaches into several of the bar's widgets (the
//! breakpoint swaps the menus, the dock holds the scene menu), so
//! [`Header`] makes them and hands them to the component at launch.

use adw::prelude::*;
use gtk::{gio, glib};
use relm4::{Component, ComponentController, ComponentParts, ComponentSender, Controller};
use stave_app::i18n::{fill_in, tr};
use stave_app::session::{Activity, Build, Pill, Tone, View};

use super::icons::{self, PathIcon};

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
    /// Show the project's name, and its folder under it and in the
    /// tooltip.
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
    title: adw::WindowTitle,
    /// The title and the switcher, the bar's title widget.
    middle: gtk::Box,
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

/// Frees the children of `group`'s `toggles` when the group is destroyed.
///
/// libadwaita (1.10, and its main branch) leaks an `adw::Toggle`'s child:
/// `adw_toggle_set_child` takes a reference to it and nothing drops that
/// reference, as `AdwToggle` has no dispose and its finalize leaves the
/// child alone. So the switcher's icons and labels outlived every closed
/// window. Holding the toggles until the group's `destroy` (which comes
/// after the group has let go of them) and unsetting their children then
/// frees them; the toggles go with the handler. See
/// `docs/reviews/upstream-adw-toggle-leak.md`.
fn free_children_on_destroy(group: &adw::ToggleGroup, toggles: Vec<adw::Toggle>) {
    group.connect_destroy(move |_| {
        for toggle in &toggles {
            toggle.set_child(None::<&gtk::Widget>);
        }
    });
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
            .icon_name("open-menu-symbolic")
            .menu_model(menus)
            .tooltip_text(tr("Main Menu"))
            .css_classes(["flat", "stave-icon-button"])
            .valign(gtk::Align::Center)
            .primary(true)
            .visible(false)
            .build();
        menu_button.update_property(&[gtk::accessible::Property::Label(&tr("Main Menu"))]);

        let title = adw::WindowTitle::builder()
            .title(folder_label)
            .css_classes(["stave-title"])
            .valign(gtk::Align::Center)
            .build();

        let find = gtk::Button::builder()
            .icon_name("system-search-symbolic")
            .tooltip_text(tr("Find a part, file, line or problem (Ctrl+K)"))
            .action_name("win.find")
            .css_classes(["flat", "stave-icon-button", "stave-find"])
            .valign(gtk::Align::Center)
            .build();
        find.update_property(&[gtk::accessible::Property::Label(&tr(
            "Find a part, file, line or problem",
        ))]);

        let switcher = adw::ToggleGroup::builder()
            .css_classes(["stave-switcher"])
            .homogeneous(true)
            .valign(gtk::Align::Center)
            .build();
        let mut switcher_labels = Vec::new();
        let mut toggles = Vec::new();
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
            let label = gtk::Label::new(Some(&view.title()));
            content.append(&label);
            switcher_labels.push(label);
            let toggle = adw::Toggle::builder()
                .name(view.word())
                .child(&content)
                .tooltip(view.title())
                .build();
            switcher.add(toggle.clone());
            toggles.push(toggle);
        }
        free_children_on_destroy(&switcher, toggles);
        switcher.set_active_name(Some(View::Design.word()));
        // The Cut list view isn't made yet, so the design is the only
        // view: the switcher stays hidden until there is a second one,
        // and the project's name takes the middle of the bar.
        let switching = View::ALL.into_iter().filter(|view| ready(*view)).count() > 1;
        switcher.set_visible(switching);
        title.set_visible(!switching);
        let middle = gtk::Box::builder().valign(gtk::Align::Center).build();
        middle.append(&title);
        middle.append(&switcher);

        let scene_label = gtk::Label::new(Some(&tr("Rest")));
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
            .tooltip_text(tr("Scene"))
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
        let pill_label = gtk::Label::new(Some(&tr("Not checked yet")));
        let pill_content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        pill_content.append(&pill_stack);
        pill_content.append(&pill_label);
        let pill = gtk::Button::builder()
            .child(&pill_content)
            .css_classes(["stave-pill"])
            .tooltip_text(tr("Not checked yet"))
            .action_name("win.next-problem")
            .valign(gtk::Align::Center)
            .build();
        let previous = gtk::Button::builder()
            .icon_name("go-previous-symbolic")
            .tooltip_text(tr("Previous problem (Shift+F8)"))
            .action_name("win.previous-problem")
            .css_classes(["stave-problem-step"])
            .valign(gtk::Align::Center)
            .visible(false)
            .build();
        previous.update_property(&[gtk::accessible::Property::Label(&tr("Previous problem"))]);
        let next = gtk::Button::builder()
            .icon_name("go-next-symbolic")
            .tooltip_text(tr("Next problem (F8)"))
            .action_name("win.next-problem")
            .css_classes(["stave-problem-step"])
            .valign(gtk::Align::Center)
            .visible(false)
            .build();
        next.update_property(&[gtk::accessible::Property::Label(&tr("Next problem"))]);
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
            .tooltip_text(tr("Build plan (Ctrl+B)"))
            .css_classes(["stave-build"])
            .valign(gtk::Align::Center)
            .build();
        build.update_property(&[gtk::accessible::Property::Label(&tr("Build plan"))]);
        build.set_action_name(Some("win.build"));
        // The build plan isn't made yet: pressing the button would only
        // say so. It stays hidden (and `win.build` unregistered) until it
        // builds a plan.
        build.set_visible(false);

        HeaderParts {
            menu_bar,
            menu_button,
            title,
            middle,
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
            parts.title.set_title(title);
            parts.title.set_subtitle(folder);
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
                &scenes
                    .titles
                    .get(scenes.current)
                    .map_or_else(|| tr("Rest"), Clone::clone),
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
        Tone::Warnings => ("warnings", icons::WARNING),
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
        fill_in(&tr("{line}. Show the next problem (F8)"), &[("line", line)])
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
        root.set_title_widget(Some(&parts.middle));
        root.pack_start(&parts.menu_button);
        root.pack_start(&parts.menu_bar);
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
    /// The project's name, its folder under it; in the middle of the bar
    /// while the switcher is hidden.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "it shortens itself in a narrow window; tests read it"
        )
    )]
    pub title: adw::WindowTitle,
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
    /// The pill's icon, for tests to read its shape.
    #[cfg(test)]
    pill_icon: PathIcon,
    controller: Controller<HeaderComponent>,
}

impl Header {
    /// A title bar for a project in `folder_label` (shown as its name
    /// until the design is loaded), its menu bar made of `menus`: one
    /// submenu each for Project, Edit, View and Help. People's choices go
    /// to `owner` as `message(output)`.
    pub fn new<M: 'static>(
        folder_label: &str,
        menus: &gio::Menu,
        owner: &relm4::Sender<M>,
        message: impl Fn(HeaderOutput) -> M + 'static,
    ) -> Header {
        let parts = HeaderParts::new(folder_label, menus);
        let menu_bar = parts.menu_bar.clone();
        let menu_button = parts.menu_button.clone();
        let title = parts.title.clone();
        let switcher = parts.switcher.clone();
        let switcher_labels = parts.switcher_labels.clone();
        let scene = parts.scene.clone();
        let scenes = parts.scenes.clone();
        #[cfg(test)]
        let pill_icon = parts.pill_icon.clone();
        let controller = HeaderComponent::builder()
            .launch(parts)
            .forward(owner, message);
        Header {
            bar: controller.widget().clone(),
            menu_bar,
            menu_button,
            title,
            switcher,
            switcher_labels,
            scene,
            scenes,
            #[cfg(test)]
            pill_icon,
            controller,
        }
    }

    /// Queues `input`; once the bar has shut down there is nobody to tell.
    fn send(&self, input: HeaderInput) {
        let _ = self.controller.sender().send(input);
    }

    /// The project's name, and its folder under it and in the tooltip.
    pub fn show_title(&self, title: &str, folder: &str) {
        self.send(HeaderInput::ShowTitle {
            title: title.to_owned(),
            folder: folder.to_owned(),
        });
    }

    /// The view showing.
    pub fn show_view(&self, view: View) {
        self.send(HeaderInput::ShowView(view));
    }

    /// The switcher's words, hidden when the window is narrow.
    pub fn switcher_labels(&self) -> &[gtk::Label] {
        &self.switcher_labels
    }

    /// The scenes to choose from (titles) and the one showing.
    pub fn show_scenes(&self, titles: &[String], current: usize) {
        self.send(HeaderInput::ShowScenes(ScenesShown {
            titles: titles.to_vec(),
            current,
        }));
    }

    /// The problems chip: the pill's colour and words, the arrows when
    /// there are problems to step through, and the status line (what the
    /// last check found) in its tooltip.
    pub fn show_pill(&self, pill: &Pill, problems: usize, line: &str) {
        self.send(HeaderInput::ShowPill(PillShown {
            pill: pill.clone(),
            problems,
            line: line.to_owned(),
        }));
    }

    /// The build button: building (spinning) or ready, with the tooltip
    /// the session gives (what it does, or why there is nothing to build).
    pub fn show_build(&self, build: Build, tooltip: &str) {
        self.send(HeaderInput::ShowBuild {
            build,
            tooltip: tooltip.to_owned(),
        });
    }

    /// The component's model, as of the last message it handled.
    #[cfg(test)]
    fn model(&self) -> std::cell::Ref<'_, HeaderComponent> {
        self.controller.model()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;
    use crate::ui::gtk_test::{handle_each, on_gtk_thread, recorder, settle};

    /// A bar whose outputs go to `owner`.
    fn header_for(owner: &relm4::Sender<HeaderOutput>) -> Header {
        Header::new(
            "hall_table",
            &gio::Menu::new(),
            owner,
            std::convert::identity,
        )
    }

    /// A bar whose outputs nobody hears.
    fn header() -> Header {
        let (owner, _) = recorder();
        header_for(&owner)
    }

    #[test]
    fn a_dropped_header_is_freed() {
        on_gtk_thread(header_is_freed);
    }

    /// Dropping the header shuts its component down when the main loop
    /// next runs: the bar, the model (and its runtime) and the message
    /// maker are all freed.
    fn header_is_freed() {
        let token = Rc::new(());
        let held = Rc::clone(&token);
        let (owner, _) = recorder();
        let header = Header::new("hall_table", &gio::Menu::new(), &owner, move |output| {
            let _ = &held;
            output
        });
        let callback = Rc::downgrade(&token);
        drop(token);
        let bar = header.bar.downgrade();
        let switcher = header.switcher.downgrade();
        let input = header.controller.sender().clone();
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
        assert!(callback.upgrade().is_none(), "the message maker is freed");
    }

    #[test]
    fn setters_show_what_the_session_says_without_calling_back() {
        on_gtk_thread(setters_do_not_call_back);
    }

    fn setters_do_not_call_back() {
        let (owner, heard) = recorder();
        let header = header_for(&owner);
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
        assert_eq!(header.title.title(), "hall_table", "only queued so far");
        settle();
        assert_eq!(header.title.title(), "Hall table");
        assert_eq!(header.title.subtitle(), "~/Projects/hall_table");
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
    /// the owner after the update has returned (reading the model there
    /// would panic if it had not), and a choice made while the owner
    /// handles it is queued too.
    fn choices_are_queued() {
        let (owner, outputs) = relm4::channel();
        let header = Rc::new(header_for(&owner));
        let heard: Rc<RefCell<Vec<(HeaderOutput, View)>>> = Rc::default();
        let log = Rc::clone(&heard);
        let weak = Rc::downgrade(&header);
        handle_each(outputs, move |output| {
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

    #[test]
    fn every_title_bar_button_has_a_name() {
        on_gtk_thread(title_bar_buttons_are_named);
    }

    /// Icon-only buttons (previous, next, the main menu, ...) have an
    /// accessible name, not only a tooltip.
    fn title_bar_buttons_are_named() {
        let header = header();
        settle();
        assert_eq!(
            crate::ui::gtk_test::unnamed_buttons(&header.bar),
            Vec::<String>::new()
        );
    }

    #[test]
    fn errors_and_warnings_have_different_icons() {
        on_gtk_thread(pill_icons_differ);
    }

    /// The pill's icon tells errors (a circle) from warnings (a
    /// triangle) without their colour.
    fn pill_icons_differ() {
        let header = header();
        let shape = |tone| {
            let pill = Pill {
                tone,
                label: String::new(),
                activity: Activity::Idle,
            };
            header.show_pill(&pill, 1, "");
            settle();
            header.pill_icon.shape()
        };
        assert_eq!(shape(Tone::Errors), icons::ALERT);
        assert_eq!(shape(Tone::Warnings), icons::WARNING);
        assert_ne!(icons::ALERT, icons::WARNING);
    }

    /// The main menu uses the theme's symbolic icon, not a drawn one.
    #[test]
    fn the_main_menu_uses_the_theme_icon() {
        on_gtk_thread(|| {
            let header = header();
            assert_eq!(
                header.menu_button.icon_name().as_deref(),
                Some("open-menu-symbolic")
            );
        });
    }

    /// With one view the project's name, not the hidden switcher, is the
    /// bar's centred title, with its folder under it.
    #[test]
    fn the_title_is_in_the_middle() {
        on_gtk_thread(|| {
            let header = header();
            header.show_title("Shelf", "~/shelf");
            settle();
            let middle = header.bar.title_widget().expect("a title widget");
            assert!(header.title.is_ancestor(&middle));
            assert!(header.title.is_visible());
            assert!(!header.switcher.is_visible());
            assert_eq!(header.title.subtitle(), "~/shelf");
        });
    }

    /// A toggle group with one toggle whose own child is an icon and a
    /// label, as the switcher's are, and weak references to the child's
    /// three widgets.
    fn group_with_a_toggle_child() -> (
        adw::ToggleGroup,
        adw::Toggle,
        Vec<glib::WeakRef<gtk::Widget>>,
    ) {
        let group = adw::ToggleGroup::new();
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let icon = gtk::Image::from_icon_name("go-home-symbolic");
        let label = gtk::Label::new(Some("Design"));
        content.append(&icon);
        content.append(&label);
        let toggle = adw::Toggle::builder()
            .name("design")
            .child(&content)
            .build();
        group.add(toggle.clone());
        let weak = vec![
            content.upcast_ref::<gtk::Widget>().downgrade(),
            icon.upcast_ref::<gtk::Widget>().downgrade(),
            label.upcast_ref::<gtk::Widget>().downgrade(),
        ];
        (group, toggle, weak)
    }

    /// libadwaita's own leak, with nothing of Stave's in it: a plain
    /// `adw::ToggleGroup`, dropped, leaves its toggle's child alive. If
    /// this fails, libadwaita has fixed it, and `free_children_on_destroy`
    /// can go.
    #[test]
    fn libadwaita_leaks_a_toggles_child() {
        on_gtk_thread(|| {
            let (group, toggle, weak) = group_with_a_toggle_child();
            drop(toggle);
            let freed = group.downgrade();
            drop(group);
            settle();
            assert!(freed.upgrade().is_none(), "the group is freed");
            assert!(
                weak.iter().all(|widget| widget.upgrade().is_some()),
                "libadwaita still keeps a toggle's child alive"
            );
        });
    }

    /// With `free_children_on_destroy`, the same group's toggle child is
    /// freed with it.
    #[test]
    fn a_destroyed_groups_toggle_children_are_freed() {
        on_gtk_thread(|| {
            let (group, toggle, weak) = group_with_a_toggle_child();
            free_children_on_destroy(&group, vec![toggle]);
            let freed = group.downgrade();
            drop(group);
            settle();
            assert!(freed.upgrade().is_none(), "the group is freed");
            assert!(
                weak.iter().all(|widget| widget.upgrade().is_none()),
                "the toggle's child is freed with its group"
            );
        });
    }

    /// The bar's switcher, dropped with the header, leaves none of its
    /// toggles' icons and labels behind.
    #[test]
    fn the_switchers_toggle_children_are_freed() {
        on_gtk_thread(|| {
            let header = header();
            let mut children = Vec::new();
            let mut button = header.switcher.first_child();
            while let Some(current) = button {
                if let Some(child) = current
                    .downcast_ref::<gtk::ToggleButton>()
                    .and_then(gtk::prelude::ButtonExt::child)
                {
                    children.push(child.downgrade());
                }
                button = current.next_sibling();
            }
            assert_eq!(children.len(), View::ALL.len(), "one child per view");
            drop(header);
            settle();
            assert!(
                children.iter().all(|child| child.upgrade().is_none()),
                "the switcher's toggle children are freed"
            );
        });
    }
}
