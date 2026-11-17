//! The title bar: the sidebar toggle, back and forward, the project's
//! title, the Parts / Cut list / Files switcher, the scene menu, the status
//! pill and the build button. It only shows what the session says; the
//! window wires its buttons to session commands.

use adw::prelude::*;
use stave_app::session::{Activity, Build, Pill, Tone, View};

use super::icons::{self, PathIcon};

/// The title bar's widgets.
pub struct Header {
    /// The bar itself.
    pub bar: adw::HeaderBar,
    /// Shows or hides the left sidebar.
    pub sidebar: gtk::ToggleButton,
    /// Back (Alt+←).
    pub back: gtk::Button,
    /// Forward (Alt+→).
    pub forward: gtk::Button,
    /// Opens another project folder.
    pub open: gtk::Button,
    /// The project's title and folder.
    pub title_box: gtk::Box,
    title: gtk::Label,
    subtitle: gtk::Label,
    /// Parts / Cut list / Files.
    pub switcher: adw::ToggleGroup,
    switcher_labels: Vec<gtk::Label>,
    /// The scene menu.
    pub scene: gtk::MenuButton,
    scene_label: gtk::Label,
    /// The scenes, in the scene menu's popover.
    pub scenes: gtk::ListBox,
    /// The status pill: opens the problems list.
    pub pill: gtk::Button,
    pill_icon: PathIcon,
    pill_spinner: adw::Spinner,
    pill_stack: gtk::Stack,
    pill_label: gtk::Label,
    /// Builds the plan (Ctrl+B).
    pub build: gtk::Button,
    build_stack: gtk::Stack,
}

fn icon_button(shape: icons::Shape, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::builder()
        .child(&PathIcon::new(shape, 16))
        .tooltip_text(tooltip)
        .css_classes(["flat", "stave-icon-button"])
        .valign(gtk::Align::Center)
        .build();
    button.update_property(&[gtk::accessible::Property::Label(tooltip)]);
    button
}

impl Header {
    /// A title bar for a project in `folder` (shown as the subtitle until
    /// the design is loaded).
    pub fn new(folder_label: &str, menu: &gtk::gio::Menu) -> Header {
        let sidebar = gtk::ToggleButton::builder()
            .child(&PathIcon::new(icons::SIDEBAR, 16))
            .tooltip_text("Show the parts (F9)")
            .css_classes(["flat", "stave-icon-button", "stave-sidebar-toggle"])
            .valign(gtk::Align::Center)
            .active(true)
            .build();
        let back = icon_button(icons::BACK, "Back (Alt+←)");
        let forward = icon_button(icons::FORWARD, "Forward (Alt+→)");
        back.set_sensitive(false);
        forward.set_sensitive(false);
        let open = icon_button(icons::FOLDER, "Open a project folder (Ctrl+O)");

        let title = gtk::Label::builder()
            .label(folder_label)
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["title"])
            .build();
        let subtitle = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["subtitle"])
            .build();
        let title_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .valign(gtk::Align::Center)
            .css_classes(["stave-title"])
            .build();
        title_box.append(&title);
        title_box.append(&subtitle);

        let switcher = adw::ToggleGroup::builder()
            .css_classes(["stave-switcher"])
            .homogeneous(true)
            .valign(gtk::Align::Center)
            .build();
        let mut switcher_labels = Vec::new();
        for view in View::ALL {
            let shape = match view {
                View::Parts => icons::PARTS,
                View::CutList => icons::CUT_LIST,
                View::Files => icons::FILES,
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
        switcher.set_active_name(Some(View::Parts.word()));

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
            .css_classes(["stave-pill", "unknown"])
            .tooltip_text("Show the problems")
            .valign(gtk::Align::Center)
            .build();

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

        let menu = gtk::MenuButton::builder()
            .child(&PathIcon::new(icons::MENU, 16))
            .menu_model(menu)
            .tooltip_text("Main menu")
            .css_classes(["flat", "stave-icon-button"])
            .valign(gtk::Align::Center)
            .primary(true)
            .build();

        let bar = adw::HeaderBar::builder()
            .title_widget(&switcher)
            .css_classes(["stave-header"])
            .build();
        bar.pack_start(&sidebar);
        bar.pack_start(&back);
        bar.pack_start(&forward);
        bar.pack_start(&open);
        bar.pack_start(&title_box);
        bar.pack_end(&menu);
        bar.pack_end(&build);
        bar.pack_end(&pill);
        bar.pack_end(&scene);

        Header {
            bar,
            sidebar,
            back,
            forward,
            open,
            title_box,
            title,
            subtitle,
            switcher,
            switcher_labels,
            scene,
            scene_label,
            scenes,
            pill,
            pill_icon,
            pill_spinner,
            pill_stack,
            pill_label,
            build,
            build_stack,
        }
    }

    /// The project's title and folder.
    pub fn show_title(&self, title: &str, subtitle: &str) {
        self.title.set_label(title);
        self.subtitle.set_label(subtitle);
    }

    /// Back and Forward's state.
    pub fn show_history(&self, back: bool, forward: bool) {
        self.back.set_sensitive(back);
        self.forward.set_sensitive(forward);
    }

    /// The view showing.
    pub fn show_view(&self, view: View) {
        if self.switcher.active_name().as_deref() != Some(view.word()) {
            self.switcher.set_active_name(Some(view.word()));
        }
    }

    /// The switcher's words, hidden when the window is narrow.
    pub fn switcher_labels(&self) -> &[gtk::Label] {
        &self.switcher_labels
    }

    /// The project's folder under its title, hidden when narrow.
    pub fn subtitle(&self) -> &gtk::Label {
        &self.subtitle
    }

    /// The scene menu's words.
    pub fn scene_label(&self) -> &gtk::Label {
        &self.scene_label
    }

    /// The scenes to choose from (titles) and the one showing.
    pub fn show_scenes(&self, titles: &[String], current: usize) {
        while let Some(row) = self.scenes.row_at_index(0) {
            self.scenes.remove(&row);
        }
        for (index, title) in titles.iter().enumerate() {
            let label = gtk::Label::builder()
                .label(title)
                .xalign(0.0)
                .margin_start(6)
                .margin_end(6)
                .build();
            let row = gtk::ListBoxRow::builder().child(&label).build();
            if index == current {
                label.add_css_class("heading");
            }
            self.scenes.append(&row);
        }
        self.scene_label
            .set_label(titles.get(current).map_or("Rest", |title| title.as_str()));
        self.scene.set_sensitive(!titles.is_empty());
    }

    /// The status pill. `open`: the problems list is showing.
    pub fn show_pill(&self, pill: &Pill, open: bool) {
        for class in ["clean", "warnings", "errors", "unknown"] {
            self.pill.remove_css_class(class);
        }
        let (class, shape) = match pill.tone {
            Tone::Clean => ("clean", icons::CHECK),
            Tone::Warnings => ("warnings", icons::ALERT),
            Tone::Errors => ("errors", icons::ALERT),
            Tone::Unknown => ("unknown", icons::PENDING),
        };
        self.pill.add_css_class(class);
        if open {
            self.pill.add_css_class("open");
        } else {
            self.pill.remove_css_class("open");
        }
        self.pill_icon.set_shape(shape);
        self.pill_label.set_label(&pill.label);
        let checking = pill.activity == Activity::Checking;
        self.pill_stack
            .set_visible_child_name(if checking { "spinner" } else { "icon" });
        self.pill_spinner.set_visible(checking);
        let tooltip = if checking {
            format!("{}, checking again. Show the problems", pill.label)
        } else {
            format!("{}. Show the problems", pill.label)
        };
        self.pill.set_tooltip_text(Some(&tooltip));
        self.pill
            .update_property(&[gtk::accessible::Property::Label(&tooltip)]);
    }

    /// The build button: building (spinning) or ready, with the tooltip
    /// the session gives (what it does, or why there is nothing to build).
    pub fn show_build(&self, build: Build, tooltip: &str) {
        match build {
            Build::Building => {
                self.build.add_css_class("building");
                self.build_stack.set_visible_child_name("building");
            }
            Build::Idle => {
                self.build.remove_css_class("building");
                self.build_stack.set_visible_child_name("ready");
            }
        }
        self.build.set_tooltip_text(Some(tooltip));
    }
}
