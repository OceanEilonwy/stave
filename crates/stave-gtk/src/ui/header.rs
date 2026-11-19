//! The title bar (V2C_Home.dc.html): back and forward, the project's
//! title, the finder's field (Ctrl+K), the Design / Cut list switcher, the
//! problems chip (the status pill between previous and next, stepping
//! through the problems) and the build button. It also makes the scene
//! menu, which the window puts in the dock. It only shows what the
//! session says; the window wires its buttons to session commands.

use adw::prelude::*;
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
    /// Opens the finder (Ctrl+K).
    pub find: gtk::Button,
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

        // The finder's field: a button that looks like a search entry.
        let find_content = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(8)
            .build();
        find_content.append(&gtk::Image::from_icon_name("system-search-symbolic"));
        let find_words = gtk::Label::builder()
            .label("Find a part, file, line or problem")
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["dim-label"])
            .build();
        find_content.append(&find_words);
        find_content.append(
            &gtk::Label::builder()
                .label("Ctrl K")
                .css_classes(["stave-key", "dim-label"])
                .build(),
        );
        let find = gtk::Button::builder()
            .child(&find_content)
            .tooltip_text("Find a part, file, line or problem (Ctrl+K)")
            .action_name("win.find")
            .css_classes(["stave-find"])
            .valign(gtk::Align::Center)
            .hexpand(true)
            .width_request(260)
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

        let menu = gtk::MenuButton::builder()
            .child(&PathIcon::new(icons::MENU, 16))
            .menu_model(menu)
            .tooltip_text("Main menu")
            .css_classes(["flat", "stave-icon-button"])
            .valign(gtk::Align::Center)
            .primary(true)
            .build();
        menu.update_property(&[gtk::accessible::Property::Label("Main menu")]);

        let find_box = gtk::Box::builder()
            .halign(gtk::Align::Center)
            .hexpand(true)
            .build();
        find_box.append(&find);
        find.set_width_request(420);
        let bar = adw::HeaderBar::builder()
            .title_widget(&find_box)
            .css_classes(["stave-header"])
            .build();
        bar.pack_start(&back);
        bar.pack_start(&forward);
        bar.pack_start(&open);
        bar.pack_start(&title_box);
        bar.pack_end(&menu);
        bar.pack_end(&build);
        bar.pack_end(&chip);
        bar.pack_end(&switcher);

        Header {
            bar,
            back,
            forward,
            open,
            title_box,
            title,
            subtitle,
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

    /// The finder's field, hidden when the window is narrow.
    pub fn find_box(&self) -> &gtk::Button {
        &self.find
    }

    /// The switcher's words, hidden when the window is narrow.
    pub fn switcher_labels(&self) -> &[gtk::Label] {
        &self.switcher_labels
    }

    /// The project's folder under its title, hidden when narrow.
    pub fn subtitle(&self) -> &gtk::Label {
        &self.subtitle
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
            Tone::Warnings => ("warnings", icons::ALERT),
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
    use super::*;

    #[test]
    fn every_title_bar_button_has_a_name() {
        crate::ui::gtk_test::on_gtk_thread(title_bar_buttons_are_named);
    }

    /// Icon-only buttons (previous, next, the main menu, ...) have an
    /// accessible name, not only a tooltip.
    fn title_bar_buttons_are_named() {
        let header = Header::new("shelf", &gtk::gio::Menu::new());
        assert_eq!(
            crate::ui::gtk_test::unnamed_buttons(&header.bar),
            Vec::<String>::new()
        );
    }
}
