//! The title bar, one slim row (34 px): at the left Stave's logo (the
//! dovetail cube, the application icon) and a menu bar (Project,
//! Edit, View, Help, made from the main menu); in the middle the
//! project's name over its folder (an `adw::WindowTitle`, as libadwaita
//! centres a window's title), or the Design / Cut list switcher once
//! there is more than one view; at the right a find button (the finder,
//! Ctrl+K), the problems chip (the status pill between previous and next,
//! stepping through the problems) and the build button. Back and forward
//! live in the View menu and on Alt+← and Alt+→. In a narrow window the
//! switcher drops its words, and in a very narrow one the menu bar folds
//! into one menu button. It only shows what the session says; the window
//! wires its buttons to session commands.
//!
//! It is a relm4 component, [`HeaderComponent`]: its model (what the
//! session last said) is the only state, only its update changes it, and
//! its widgets' handlers only send [`HeaderInput`] messages. [`Header`]
//! owns its controller and keeps the setters as plain methods; people's
//! choices go to the owner as the message it chose when it made the bar.
//! The window's breakpoints reach the bar's widgets only through it: the
//! bar adds its own setters to each ([`HeaderInput::Narrow`]).

use adw::prelude::*;
use gtk::{gio, glib};
use relm4::{Component, ComponentController, ComponentParts, ComponentSender, Controller};
use stave_app::i18n::{fill_in, tr, tr_noop};
use stave_app::session::{Activity, Build, Pill, Tone, View};
use std::path::PathBuf;

use super::icons::{self, PathIcon};
use super::logo::Logo;
use super::{app, workbench};

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

/// A window at most this wide (in sp) shows the switcher's icons without
/// their words.
const WORDLESS_AT: f64 = 760.0;

/// A window at most this wide (in sp) folds the menu bar into one menu
/// button and draws the switcher compact.
const FOLDED_AT: f64 = 600.0;

/// The title bar's state: what the session last said. `None` keeps the
/// look the bar was made with ("Not checked yet", the folder's name).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeaderComponent {
    /// The project's name and its folder.
    title: Option<(String, String)>,
    /// The view showing.
    view: View,
    pill: Option<PillShown>,
    /// The build button's state and tooltip.
    build: Option<(Build, String)>,
    /// The recent projects Open Recent lists, newest first.
    recent: Vec<PathBuf>,
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
    /// Narrow the bar while this breakpoint (a window at most `width` sp
    /// wide) applies: at most 760 sp the switcher drops its words, at
    /// most 600 sp the menu bar folds into one menu button and the
    /// switcher is drawn compact. The bar adds its own setters to it.
    Narrow {
        /// The breakpoint.
        breakpoint: adw::Breakpoint,
        /// The widest window it applies to, in sp.
        width: f64,
    },
    /// Show the problems chip.
    ShowPill(PillShown),
    /// Show the build button building or ready, with this tooltip.
    ShowBuild {
        /// Building or ready.
        build: Build,
        /// What it does, or why there is nothing to build.
        tooltip: String,
    },
    /// List these project folders, newest first, under Project → Open
    /// Recent (the application's recent projects, sent to every window
    /// whenever they change).
    ShowRecent(Vec<PathBuf>),
    /// People chose a view in the switcher.
    ViewChosen(View),
}

/// What the title bar tells its owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeaderOutput {
    /// People chose this view.
    View(View),
}

/// The bar's widgets, made by [`Header`] (tests read some of them) and
/// handed to the component, which owns their state.
#[derive(Clone, Debug)]
pub struct HeaderParts {
    /// The Open Recent submenu of the bar's own menus.
    recent: gio::Menu,
    /// Stave's logo, at the bar's start.
    logo: Logo,
    menu_bar: gtk::PopoverMenuBar,
    menu_button: gtk::MenuButton,
    title: adw::WindowTitle,
    /// The title and the switcher, the bar's title widget.
    middle: gtk::Box,
    find: gtk::Button,
    switcher: adw::ToggleGroup,
    switcher_labels: Vec<gtk::Label>,
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

/// Stave's logo for the bar's start: the dovetail cube, 20 px, drawn
/// from its paths (see [`Logo`]).
fn logo() -> Logo {
    let logo = Logo::new(20);
    logo.set_margin_start(4);
    logo.set_margin_end(2);
    logo
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
    /// The bar's widgets for a project in `folder_label`, with its own
    /// menus ([`main_menu`]) and their Open Recent submenu.
    fn new(folder_label: &str) -> HeaderParts {
        let recent = gio::Menu::new();
        let menus = &main_menu(&recent);
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
            recent,
            logo: logo(),
            menu_bar,
            menu_button,
            title,
            middle,
            find,
            switcher,
            switcher_labels,
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

/// The title bar's menus: Project, Edit, View and Help, each a submenu
/// (the menu bar shows them side by side; a narrow window's one menu
/// button lists them).
fn main_menu(recent: &gio::Menu) -> gio::Menu {
    /// A section of `(label, action)` items.
    fn section(items: &[(&str, &str)]) -> gio::Menu {
        let section = gio::Menu::new();
        for (label, action) in items {
            section.append(Some(&tr(label)), Some(action));
        }
        section
    }
    let project = gio::Menu::new();
    let opening = section(&[
        (tr_noop("New Project…"), "win.new-project"),
        (tr_noop("Open Project…"), "win.open"),
    ]);
    opening.append_submenu(Some(&tr("Open Recent")), recent);
    project.append_section(None, &opening);
    project.append_section(
        None,
        &section(&[
            (tr_noop("Save"), "win.save"),
            (tr_noop("Check Again"), "win.check"),
            (tr_noop("Build Plan"), "win.build"),
        ]),
    );
    project.append_section(
        None,
        &section(&[(tr_noop("Export 3D Model…"), "win.export-view")]),
    );
    let edit = gio::Menu::new();
    edit.append_section(
        None,
        &section(&[
            (tr_noop("Find…"), "win.find"),
            (tr_noop("Find in the Text"), "win.search"),
            (tr_noop("Format the Text"), "win.format"),
        ]),
    );
    edit.append_section(
        None,
        &section(&[
            (tr_noop("Next Problem"), "win.next-problem"),
            (tr_noop("Previous Problem"), "win.previous-problem"),
        ]),
    );
    let view = gio::Menu::new();
    view.append_section(
        None,
        &section(&[
            (tr_noop("Design"), "win.view-design"),
            (tr_noop("Cut List"), "win.view-cut-list"),
        ]),
    );
    view.append_section(
        None,
        &section(&[
            (tr_noop("Back"), "win.back"),
            (tr_noop("Forward"), "win.forward"),
            (tr_noop("Frame Everything"), "win.frame"),
        ]),
    );
    // Any panel but the 3D view can be closed, and a first launch shows
    // only the 3D view, Parts and Problems; these open the others, a
    // closed one where it was.
    let panels = gio::Menu::new();
    for id in workbench::PanelId::ALL {
        panels.append(
            Some(&id.title()),
            Some(&format!("win.show-panel::{}", id.word())),
        );
    }
    panels.append(Some(&tr("Reset Layout")), Some("win.reset-layout"));
    view.append_section(Some(&tr("Panels")), &panels);
    view.append_section(Some(&tr("Style")), &app::style_section());
    let menu = gio::Menu::new();
    menu.append_submenu(Some(&tr("Project")), &project);
    menu.append_submenu(Some(&tr("Edit")), &edit);
    menu.append_submenu(Some(&tr("View")), &view);
    menu.append_submenu(Some(&tr("Help")), &app::help_section());
    menu
}

/// Fills the Open Recent submenu again with `folders`, newest first.
fn fill_recent(menu: &gio::Menu, folders: &[PathBuf]) {
    menu.remove_all();
    for folder in folders {
        let item = gio::MenuItem::new(Some(&app::menu_label(&app::folder_name(folder))), None);
        item.set_action_and_target_value(
            Some("win.open-recent"),
            Some(&folder.display().to_string().to_variant()),
        );
        menu.append_item(&item);
    }
}

/// Adds the bar's own setters to a breakpoint for a window at most
/// `width` sp wide: the switcher's words go at most 760 sp; at most 600
/// sp the menu bar folds into one menu button and the switcher is drawn
/// compact. Only the last breakpoint that matches applies, so a narrower
/// one repeats the wider ones' setters.
fn narrow(parts: &HeaderParts, breakpoint: &adw::Breakpoint, width: f64) {
    if width <= WORDLESS_AT {
        for label in &parts.switcher_labels {
            breakpoint.add_setter(label, "visible", Some(&false.to_value()));
        }
    }
    if width <= FOLDED_AT {
        breakpoint.add_setter(&parts.menu_bar, "visible", Some(&false.to_value()));
        breakpoint.add_setter(&parts.menu_button, "visible", Some(&true.to_value()));
        breakpoint.add_setter(
            &parts.switcher,
            "css-classes",
            Some(&["stave-switcher", "compact"].to_value()),
        );
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
        root.pack_start(&parts.logo);
        root.pack_start(&parts.menu_button);
        root.pack_start(&parts.menu_bar);
        root.pack_end(&parts.build);
        root.pack_end(&parts.chip);
        root.pack_end(&parts.find);

        let view_chosen = parts.switcher.connect_active_name_notify(move |group| {
            if let Some(view) = group.active_name().and_then(|name| View::from_word(&name)) {
                sender.input_sender().emit(HeaderInput::ViewChosen(view));
            }
        });

        let model = HeaderComponent {
            title: None,
            view: View::Design,
            pill: None,
            build: None,
            recent: Vec::new(),
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
            // The bar's own widgets: `update_with_view` adds the setters.
            HeaderInput::Narrow { .. } => None,
            HeaderInput::ShowRecent(recent) => {
                self.recent = recent;
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
        let recent = matches!(message, HeaderInput::ShowRecent(_));
        if let HeaderInput::Narrow { breakpoint, width } = &message {
            narrow(&widgets.parts, breakpoint, *width);
        }
        self.update(message, sender.clone(), root);
        if recent {
            fill_recent(&widgets.parts.recent, &self.recent);
        }
        self.update_view(widgets, sender);
    }

    fn update_view(&self, widgets: &mut HeaderWidgets, _sender: ComponentSender<Self>) {
        self.show(widgets);
    }
}

/// The title bar: it owns the component (dropping it shuts the component
/// down) and passes its setters on as [`HeaderInput`] messages. The
/// window places its root ([`Header::bar`]); every other widget of the
/// bar is the bar's own.
///
/// Setters are queued: they take effect when the main loop next runs,
/// after whatever called them has returned.
pub struct Header {
    /// The bar itself.
    pub bar: adw::HeaderBar,
    /// The bar's widgets, for tests to read.
    #[cfg(test)]
    parts: HeaderParts,
    controller: Controller<HeaderComponent>,
}

impl Header {
    /// A title bar for a project in `folder_label` (shown as its name
    /// until the design is loaded), its menu bar its own: one submenu
    /// each for Project, Edit, View and Help ([`main_menu`]), Open Recent
    /// empty until [`Header::show_recent`]. People's choices go to
    /// `owner` as `message(output)`.
    pub fn new<M: 'static>(
        folder_label: &str,
        owner: &relm4::Sender<M>,
        message: impl Fn(HeaderOutput) -> M + 'static,
    ) -> Header {
        let parts = HeaderParts::new(folder_label);
        #[cfg(test)]
        let kept = parts.clone();
        let controller = HeaderComponent::builder()
            .launch(parts)
            .forward(owner, message);
        Header {
            bar: controller.widget().clone(),
            #[cfg(test)]
            parts: kept,
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

    /// Lists these project folders, newest first, under Open Recent.
    pub fn show_recent(&self, folders: Vec<PathBuf>) {
        self.send(HeaderInput::ShowRecent(folders));
    }

    /// The view showing.
    pub fn show_view(&self, view: View) {
        self.send(HeaderInput::ShowView(view));
    }

    /// Narrows the bar while `breakpoint` (a window at most `width` sp
    /// wide) applies: at most 760 sp the switcher drops its words; at most
    /// 600 sp the menu bar folds into one menu button and the switcher is
    /// drawn compact. The bar adds its own setters, when the main loop
    /// next runs.
    pub fn narrow(&self, breakpoint: &adw::Breakpoint, width: f64) {
        self.send(HeaderInput::Narrow {
            breakpoint: breakpoint.clone(),
            width,
        });
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
        Header::new("hall_table", owner, std::convert::identity)
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
        let header = Header::new("hall_table", &owner, move |output| {
            let _ = &held;
            output
        });
        let callback = Rc::downgrade(&token);
        drop(token);
        let bar = header.bar.downgrade();
        let switcher = header.parts.switcher.downgrade();
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
    fn each_bar_lists_the_recent_projects_in_a_menu_of_its_own() {
        on_gtk_thread(recent_projects_are_listed);
    }

    /// Open Recent is the bar's own submenu, filled from `ShowRecent`
    /// when the main loop runs; another bar's stays as it was.
    fn recent_projects_are_listed() {
        let (one, two) = (header(), header());
        assert_ne!(one.parts.recent, two.parts.recent, "no menu is shared");
        one.show_recent(vec![
            PathBuf::from("/home/me/hall_table"),
            PathBuf::from("/home/me/chest"),
        ]);
        assert_eq!(one.parts.recent.n_items(), 0, "only queued");
        settle();
        assert_eq!(one.parts.recent.n_items(), 2);
        let label = one
            .parts
            .recent
            .item_attribute_value(0, "label", None)
            .and_then(|label| label.get::<String>());
        assert_eq!(label.as_deref(), Some("hall__table"), "underscores doubled");
        let target = one
            .parts
            .recent
            .item_attribute_value(1, "target", None)
            .and_then(|target| target.get::<String>());
        assert_eq!(target.as_deref(), Some("/home/me/chest"));
        assert_eq!(two.parts.recent.n_items(), 0);
        one.show_recent(Vec::new());
        settle();
        assert_eq!(one.parts.recent.n_items(), 0, "filled again, not added to");
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
        assert_eq!(
            header.parts.title.title(),
            "hall_table",
            "only queued so far"
        );
        settle();
        assert_eq!(header.parts.title.title(), "Hall table");
        assert_eq!(header.parts.title.subtitle(), "~/Projects/hall_table");
        assert_eq!(
            header.parts.title.tooltip_text().as_deref(),
            Some("~/Projects/hall_table")
        );
        assert_eq!(
            header.parts.switcher.active_name().as_deref(),
            Some(View::CutList.word())
        );
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
                header
                    .parts
                    .switcher
                    .set_active_name(Some(View::Design.word()));
            }
            log.borrow_mut().push((output, header.model().view));
        });

        header
            .parts
            .switcher
            .set_active_name(Some(View::CutList.word()));
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
            header.parts.switcher.active_name().as_deref(),
            Some(View::CutList.word())
        );
    }

    #[test]
    fn a_narrow_window_drops_the_switchers_words_then_folds_the_menus() {
        on_gtk_thread(narrow_window);
    }

    /// The bar adds its own setters to the window's breakpoints: at most
    /// 760 sp the switcher's words go; at most 600 sp the menu bar folds
    /// into the menu button and the switcher is drawn compact.
    fn narrow_window() {
        let header = header();
        let breakpoint = |width| {
            adw::Breakpoint::new(adw::BreakpointCondition::new_length(
                adw::BreakpointConditionLengthType::MaxWidth,
                width,
                adw::LengthUnit::Sp,
            ))
        };
        let (wordless, folded) = (breakpoint(760.0), breakpoint(600.0));
        header.narrow(&wordless, 760.0);
        header.narrow(&folded, 600.0);
        // The window's breakpoints, on a bin the test lays out at each
        // width.
        let bin = adw::BreakpointBin::builder()
            .width_request(300)
            .height_request(60)
            .child(&header.bar)
            .build();
        bin.add_breakpoint(wordless);
        bin.add_breakpoint(folded);
        let window = gtk::Window::builder().child(&bin).build();
        settle();
        window.present();
        settle();
        // A widget's own `visible`, whether its parents show or not (the
        // switcher is hidden while only one view is made).
        let visible = |widget: &gtk::Widget| widget.property::<bool>("visible");
        // What the bar shows `width` px wide: all the switcher's words,
        // the menu bar, the menu button, a compact switcher.
        let shown_at = |width: i32| {
            for _ in 0..3 {
                bin.allocate(width, 60, -1, None);
                settle();
            }
            let parts = &header.parts;
            (
                parts
                    .switcher_labels
                    .iter()
                    .all(|label| visible(label.upcast_ref())),
                visible(parts.menu_bar.upcast_ref()),
                visible(parts.menu_button.upcast_ref()),
                parts.switcher.has_css_class("compact"),
            )
        };
        assert_eq!(
            shown_at(1000),
            (true, true, false, false),
            "wide: all of it"
        );
        assert_eq!(shown_at(700), (false, true, false, false), "no words");
        assert_eq!(shown_at(500), (false, false, true, true), "folded");
        assert_eq!(shown_at(1000), (true, true, false, false), "all back");
        window.destroy();
        settle();
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
    fn the_logo_is_the_built_in_application_icon() {
        on_gtk_thread(logo_shows);
    }

    /// The bar starts with the logo, drawn from its own paths (no image
    /// to load, no icon theme to look it up in), before the menus.
    fn logo_shows() {
        let header = header();
        settle();
        assert_eq!(header.parts.logo.size(), 20);
        assert!(
            header.parts.logo.is_ancestor(&header.bar),
            "the logo is in the bar"
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
            header.parts.pill_icon.shape()
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
                header.parts.menu_button.icon_name().as_deref(),
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
            assert!(header.parts.title.is_ancestor(&middle));
            assert!(header.parts.title.is_visible());
            assert!(!header.parts.switcher.is_visible());
            assert_eq!(header.parts.title.subtitle(), "~/shelf");
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
            let mut button = header.parts.switcher.first_child();
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
