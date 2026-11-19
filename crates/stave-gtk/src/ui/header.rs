//! The title bar, one slim row (34 px): at the left a menu bar (Project,
//! Edit, View, Help, made from the main menu); in the middle the
//! project's name over its folder (an `adw::WindowTitle`, as libadwaita
//! centres a window's title), or the Design / Cut list switcher once
//! there is more than one view; at the right a find button (the finder,
//! Ctrl+K), the problems chip (the status pill between previous and next,
//! stepping through the problems) and the build button. Back and forward live in the View menu and on Alt+← and
//! Alt+→. In a very narrow window the menu bar folds into one menu
//! button. It also makes the scene menu, which floats on the 3D view. It
//! only shows what the session says; the window wires its buttons to
//! session commands.

use adw::prelude::*;
use gtk::gio;
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

/// The title bar's widgets.
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
    pub title: adw::WindowTitle,
    /// Design / Cut list.
    pub switcher: adw::ToggleGroup,
    switcher_labels: Vec<gtk::Label>,
    /// The scene menu (shown in the dock).
    pub scene: gtk::MenuButton,
    scene_label: gtk::Label,
    /// The scenes, in the scene menu's popover.
    pub scenes: gtk::ListBox,
    /// The problems chip round the pill and its arrows.
    pub chip: gtk::Box,
    /// The status pill: selects the next problem (F8).
    pub pill: gtk::Button,
    /// Selects the previous problem (Shift+F8).
    pub previous: gtk::Button,
    /// Selects the next problem (F8).
    pub next: gtk::Button,
    pill_icon: PathIcon,
    pill_spinner: adw::Spinner,
    pill_stack: gtk::Stack,
    pill_label: gtk::Label,
    /// Builds the plan (Ctrl+B).
    pub build: gtk::Button,
    build_stack: gtk::Stack,
}

impl Header {
    /// A title bar for a project in `folder_label` (shown as its name
    /// until the design is loaded), its menu bar made of `menus`: one
    /// submenu each for Project, Edit, View and Help.
    pub fn new(folder_label: &str, menus: &gio::Menu) -> Header {
        let menu_bar = gtk::PopoverMenuBar::from_model(Some(menus));
        menu_bar.add_css_class("stave-menu-bar");
        menu_bar.set_valign(gtk::Align::Center);
        let menu_button = gtk::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .menu_model(menus)
            .tooltip_text("Main Menu")
            .css_classes(["flat", "stave-icon-button"])
            .valign(gtk::Align::Center)
            .primary(true)
            .visible(false)
            .build();
        menu_button.update_property(&[gtk::accessible::Property::Label("Main Menu")]);

        let title = adw::WindowTitle::builder()
            .title(folder_label)
            .css_classes(["stave-title"])
            .valign(gtk::Align::Center)
            .build();

        let find = gtk::Button::builder()
            .icon_name("system-search-symbolic")
            .tooltip_text("Find a part, file, line or problem (Ctrl+K)")
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
        // view: the switcher stays hidden until there is a second one,
        // and the project's name takes the middle of the bar.
        let switching = View::ALL.into_iter().filter(|view| ready(*view)).count() > 1;
        switcher.set_visible(switching);
        title.set_visible(!switching);
        let middle = gtk::Box::builder().valign(gtk::Align::Center).build();
        middle.append(&title);
        middle.append(&switcher);

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
        previous.update_property(&[gtk::accessible::Property::Label("Previous problem")]);
        let next = gtk::Button::builder()
            .icon_name("go-next-symbolic")
            .tooltip_text("Next problem (F8)")
            .action_name("win.next-problem")
            .css_classes(["stave-problem-step"])
            .valign(gtk::Align::Center)
            .visible(false)
            .build();
        next.update_property(&[gtk::accessible::Property::Label("Next problem")]);
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

        let bar = adw::HeaderBar::builder()
            .title_widget(&middle)
            .css_classes(["stave-header"])
            .build();
        bar.pack_start(&menu_button);
        bar.pack_start(&menu_bar);
        bar.pack_end(&build);
        bar.pack_end(&chip);
        bar.pack_end(&find);

        Header {
            bar,
            menu_bar,
            menu_button,
            title,
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

    /// The project's name, and its folder under it and in the tooltip.
    pub fn show_title(&self, title: &str, folder: &str) {
        self.title.set_title(title);
        self.title.set_subtitle(folder);
        self.title.set_tooltip_text(Some(folder));
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

    /// The problems chip: the pill's colour and words, the arrows when
    /// there are problems to step through, and the status line (what the
    /// last check found) in its tooltip.
    pub fn show_pill(&self, pill: &Pill, problems: usize, line: &str) {
        for class in ["clean", "warnings", "errors", "unknown"] {
            self.chip.remove_css_class(class);
        }
        let (class, shape) = match pill.tone {
            Tone::Clean => ("clean", icons::CHECK),
            Tone::Warnings => ("warnings", icons::WARNING),
            Tone::Errors => ("errors", icons::ALERT),
            Tone::Unknown => ("unknown", icons::PENDING),
        };
        self.chip.add_css_class(class);
        self.previous.set_visible(problems > 1);
        self.next.set_visible(problems > 1);
        self.pill.set_sensitive(problems > 0);
        self.pill_icon.set_shape(shape);
        self.pill_label.set_label(&pill.label);
        let checking = pill.activity == Activity::Checking;
        self.pill_stack
            .set_visible_child_name(if checking { "spinner" } else { "icon" });
        self.pill_spinner.set_visible(checking);
        let tooltip = if problems > 0 {
            format!("{line}. Show the next problem (F8)")
        } else {
            line.to_owned()
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

#[cfg(test)]
mod tests {
    use gtk::subclass::prelude::ObjectSubclassIsExt;

    use super::*;

    #[test]
    fn every_title_bar_button_has_a_name() {
        crate::ui::gtk_test::on_gtk_thread(title_bar_buttons_are_named);
    }

    /// Icon-only buttons (previous, next, the main menu, ...) have an
    /// accessible name, not only a tooltip.
    #[test]
    fn errors_and_warnings_have_different_icons() {
        crate::ui::gtk_test::on_gtk_thread(pill_icons_differ);
    }

    /// The pill's icon tells errors (a circle) from warnings (a
    /// triangle) without their colour.
    fn pill_icons_differ() {
        let header = Header::new("shelf", &gtk::gio::Menu::new());
        let shape = |tone| {
            let pill = Pill {
                tone,
                label: String::new(),
                activity: Activity::Idle,
            };
            header.show_pill(&pill, 1, "");
            header.pill_icon.imp().shape.get()
        };
        assert_eq!(shape(Tone::Errors), icons::ALERT);
        assert_eq!(shape(Tone::Warnings), icons::WARNING);
        assert_ne!(icons::ALERT, icons::WARNING);
    }

    /// The main menu uses the theme's symbolic icon, not a drawn one.
    #[test]
    fn the_main_menu_uses_the_theme_icon() {
        crate::ui::gtk_test::on_gtk_thread(|| {
            let header = Header::new("shelf", &gtk::gio::Menu::new());
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
        crate::ui::gtk_test::on_gtk_thread(|| {
            let header = Header::new("shelf", &gtk::gio::Menu::new());
            header.show_title("Shelf", "~/shelf");
            let middle = header.bar.title_widget().expect("a title widget");
            assert!(header.title.is_ancestor(&middle));
            assert!(header.title.is_visible());
            assert!(!header.switcher.is_visible());
            assert_eq!(header.title.subtitle(), "~/shelf");
        });
    }

    fn title_bar_buttons_are_named() {
        let header = Header::new("shelf", &gtk::gio::Menu::new());
        assert_eq!(
            crate::ui::gtk_test::unnamed_buttons(&header.bar),
            Vec::<String>::new()
        );
    }
}
