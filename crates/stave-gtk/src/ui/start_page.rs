//! The start page: what Stave shows when it is started with no folder,
//! from the app grid or the dock.
//!
//! ```text
//! ┌──────────────────────────── Stave ─────────────────────── ☰ ✕ ┐
//! │                          (the icon)                          │
//! │                            Stave                             │
//! │      Open a project folder to see its design, or start one.  │
//! │            [ Open Folder… ]   [ New Project… ]               │
//! │  RECENT PROJECTS                                             │
//! │  │ hall_table                    ~/Projects/hall_table   › │ │
//! │  │ clothing_cabinet          ~/stave/examples/…          › │ │
//! └──────────────────────────────────────────────────────────────┘
//! ```
//!
//! A folder dropped on it from Files opens too. Opening a project opens
//! its window and closes this one.

use stave_app::i18n::tr;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::gio;

use super::app::{self, APP_ID, Shell};
use super::{new_project, shown_path};

/// The start page's window.
pub(super) struct StartPage {
    /// Its window.
    pub(super) window: adw::ApplicationWindow,
    /// Where its short lines show.
    pub(super) toasts: adw::ToastOverlay,
    shell: std::rc::Weak<Shell>,
}

impl StartPage {
    /// The start page, listing the recent projects.
    pub(super) fn new(shell: &Rc<Shell>) -> Rc<StartPage> {
        let menu = gio::Menu::new();
        let project = gio::Menu::new();
        project.append(Some(&tr("New Project…")), Some("win.new-project"));
        project.append(Some(&tr("Open Project…")), Some("win.open"));
        menu.append_section(None, &project);
        menu.append_section(Some(&tr("Style")), &app::style_section());
        menu.append_section(None, &app::help_section());
        let header = adw::HeaderBar::new();
        let menu_button = gtk::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .menu_model(&menu)
            .primary(true)
            .tooltip_text(tr("Main Menu"))
            .build();
        menu_button.update_property(&[gtk::accessible::Property::Label(&tr("Main Menu"))]);
        header.pack_end(&menu_button);

        let open = gtk::Button::builder()
            .label(tr("_Open Folder…"))
            .use_underline(true)
            .action_name("win.open")
            .css_classes(["pill", "suggested-action"])
            .build();
        let create = gtk::Button::builder()
            .label(tr("_New Project…"))
            .use_underline(true)
            .action_name("win.new-project")
            .css_classes(["pill"])
            .build();
        let buttons = gtk::Box::builder()
            .spacing(12)
            .halign(gtk::Align::Center)
            .build();
        buttons.append(&open);
        buttons.append(&create);
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(36)
            .build();
        content.append(&buttons);
        let recent = shell.recent_projects();
        if !recent.is_empty() {
            content.append(&recent_list(shell, &recent));
        }
        let clamp = adw::Clamp::builder()
            .maximum_size(520)
            .child(&content)
            .build();
        let has_icon = gtk::gdk::Display::default()
            .is_some_and(|display| gtk::IconTheme::for_display(&display).has_icon(APP_ID));
        let status = adw::StatusPage::builder()
            .icon_name(if has_icon {
                APP_ID
            } else {
                "folder-open-symbolic"
            })
            .title("Stave")
            .description(tr(
                "Open a project folder to see its design, or start a new project. \
                 You can also drop a folder here.",
            ))
            .child(&clamp)
            .vexpand(true)
            .build();
        let scrolled = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&status)
            .build();
        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&scrolled));
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&toasts));
        let window = adw::ApplicationWindow::builder()
            .application(shell.application())
            .title("Stave")
            .default_width(720)
            .default_height(640)
            .width_request(360)
            .height_request(400)
            .content(&toolbar)
            .build();
        let page = Rc::new(StartPage {
            window,
            toasts,
            shell: Rc::downgrade(shell),
        });
        page.install_actions();
        page
    }

    fn install_actions(self: &Rc<Self>) {
        let window = &self.window;
        let open = gio::SimpleAction::new("open", None);
        let page = Rc::downgrade(self);
        open.connect_activate(move |_, _| {
            if let Some(page) = page.upgrade() {
                page.choose_folder();
            }
        });
        window.add_action(&open);
        let create = gio::SimpleAction::new("new-project", None);
        let page = Rc::downgrade(self);
        create.connect_activate(move |_, _| {
            if let Some(page) = page.upgrade() {
                page.ask_new_project();
            }
        });
        window.add_action(&create);
        let shell = self.shell.clone();
        window.add_controller(app::folder_drop(move |path| {
            if let Some(shell) = shell.upgrade() {
                shell.open_folder(&path);
            }
        }));
        let shell = self.shell.clone();
        window.connect_close_request(move |_| {
            if let Some(shell) = shell.upgrade() {
                shell.start_closed();
            }
            gtk::glib::Propagation::Proceed
        });
    }

    /// Asks for a project folder and opens it.
    fn choose_folder(&self) {
        let dialog = gtk::FileDialog::builder()
            .title(tr("Open a project folder"))
            .modal(true)
            .build();
        let shell = self.shell.clone();
        dialog.select_folder(Some(&self.window), gio::Cancellable::NONE, move |result| {
            if let (Ok(folder), Some(shell)) = (result, shell.upgrade())
                && let Some(path) = folder.path()
            {
                shell.open_folder(&path);
            }
        });
    }

    /// The New Project dialog; the project it makes opens in its window.
    pub(super) fn ask_new_project(&self) {
        let shell = self.shell.clone();
        new_project::ask(self.window.upcast_ref(), move |folder: PathBuf| {
            if let Some(window) = shell.upgrade().and_then(|shell| shell.open_folder(&folder)) {
                window.welcome_new_project();
            }
        });
    }
}

/// The recent projects, one row each.
fn recent_list(shell: &Rc<Shell>, recent: &[PathBuf]) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder()
        .title(tr("Recent Projects"))
        .build();
    for folder in recent {
        let row = adw::ActionRow::builder()
            .title(app::folder_name(folder))
            .subtitle(shown_path(folder))
            .use_markup(false)
            .activatable(true)
            .build();
        row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
        let shell = Rc::downgrade(shell);
        let folder = folder.clone();
        row.connect_activated(move |_| {
            if let Some(shell) = shell.upgrade() {
                shell.open_folder(&folder);
            }
        });
        group.add(&row);
    }
    group
}
