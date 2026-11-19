//! The application round the windows: one process for every project
//! window. Launching Stave again, or opening a folder with it from Files,
//! hands the folder to the running Stave, which opens a window for it
//! (or brings forward the one already showing it). Started with no
//! folder, it shows the start page (`start_page`).
//!
//! Here too: what belongs to the application rather than one window
//! (Quit, About, the keyboard shortcuts, the style, recent projects) and
//! what it remembers between runs (`remember`).

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};

use adw::prelude::*;
use anyhow::{Context as _, Result, bail};
use gtk::{gdk, gio, glib};

use super::remember::Remembered;
use super::start_page::StartPage;
use super::{Options, Theme, Window, style};

/// The application's id, and its desktop file's, metainfo's and icon's
/// name. Under the domain of the project's own repository.
pub const APP_ID: &str = "io.github.OceanEilonwy.Stave";

/// The project's home page.
const WEBSITE: &str = "https://github.com/OceanEilonwy/stave";

/// Where problems with Stave itself are reported.
const ISSUES: &str = "https://github.com/OceanEilonwy/stave/issues";

/// The name the recent-files list knows Stave by.
const RECENT_NAME: &str = "Stave";

/// How many recent projects are offered.
const RECENT_LIMIT: usize = 8;

/// The window size used for screenshots when none is asked for.
const SCREENSHOT_SIZE: (u32, u32) = (1440, 900);

/// Where the shortcuts dialog lists a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum KeySection {
    /// Projects, windows and moving about.
    General,
    /// The 3D view.
    View,
    /// The text editor.
    Editor,
}

impl KeySection {
    /// Every section, in the dialog's order.
    const ALL: [KeySection; 3] = [KeySection::General, KeySection::View, KeySection::Editor];

    fn title(self) -> &'static str {
        match self {
            KeySection::General => "General",
            KeySection::View => "3D View",
            KeySection::Editor => "Editor",
        }
    }
}

/// An action with keys: what `set_accels_for_action` binds, and what the
/// shortcuts dialog lists.
pub(super) struct Accelerator {
    /// The action, `win.check`.
    pub(super) action: &'static str,
    /// Its keys, as GTK writes them.
    pub(super) keys: &'static [&'static str],
    /// What the shortcuts dialog calls it.
    pub(super) title: &'static str,
    /// Where it lists it.
    pub(super) section: KeySection,
}

/// Every action that has keys.
pub(super) const ACCELERATORS: &[Accelerator] = &[
    Accelerator {
        action: "win.new-project",
        keys: &["<Control>n"],
        title: "New project",
        section: KeySection::General,
    },
    Accelerator {
        action: "win.open",
        keys: &["<Control>o"],
        title: "Open a project folder",
        section: KeySection::General,
    },
    Accelerator {
        action: "win.check",
        keys: &["<Control>r"],
        title: "Check the project again",
        section: KeySection::General,
    },
    Accelerator {
        action: "win.find",
        keys: &["<Control>k"],
        title: "Find a part, a problem or a name",
        section: KeySection::General,
    },
    Accelerator {
        action: "win.next-problem",
        keys: &["F8"],
        title: "Next problem",
        section: KeySection::General,
    },
    Accelerator {
        action: "win.previous-problem",
        keys: &["<Shift>F8"],
        title: "Previous problem",
        section: KeySection::General,
    },
    Accelerator {
        action: "win.back",
        keys: &["<Alt>Left"],
        title: "Go back",
        section: KeySection::General,
    },
    Accelerator {
        action: "win.forward",
        keys: &["<Alt>Right"],
        title: "Go forward",
        section: KeySection::General,
    },
    Accelerator {
        action: "win.model",
        keys: &["F9"],
        title: "Show the parts",
        section: KeySection::General,
    },
    Accelerator {
        action: "app.shortcuts",
        keys: &["<Control>question"],
        title: "Keyboard shortcuts",
        section: KeySection::General,
    },
    Accelerator {
        action: "window.close",
        keys: &["<Control>w"],
        title: "Close the window",
        section: KeySection::General,
    },
    Accelerator {
        action: "app.quit",
        keys: &["<Control>q"],
        title: "Quit",
        section: KeySection::General,
    },
    Accelerator {
        action: "win.view-design",
        keys: &["<Control>1"],
        title: "Show the design",
        section: KeySection::View,
    },
    Accelerator {
        action: "win.editor",
        keys: &["<Control>e"],
        title: "Open the editor",
        section: KeySection::Editor,
    },
    Accelerator {
        action: "win.save",
        keys: &["<Control>s"],
        title: "Save the file",
        section: KeySection::Editor,
    },
    Accelerator {
        action: "win.format",
        keys: &["<Control><Shift>i"],
        title: "Tidy the file's layout",
        section: KeySection::Editor,
    },
    Accelerator {
        action: "win.search",
        keys: &["<Control>f"],
        title: "Search in the file",
        section: KeySection::Editor,
    },
];

/// Keys that a widget handles itself, not through an action: the
/// shortcuts dialog lists them too.
pub(super) const OTHER_KEYS: &[(KeySection, &str, &str)] = &[
    (KeySection::View, "Frame the selection", "f"),
    (
        KeySection::View,
        "Leave focus, or clear the selection",
        "Escape",
    ),
    (KeySection::View, "Remove the selected piece", "Delete"),
    (KeySection::Editor, "Go to where a name is made", "F12"),
    (
        KeySection::Editor,
        "Peek at where a name is made",
        "<Alt>F12",
    ),
    (
        KeySection::Editor,
        "List where a name is used",
        "<Shift>F12",
    ),
    (KeySection::Editor, "Undo", "<Control>z"),
    (KeySection::Editor, "Redo", "<Control><Shift>z"),
    (
        KeySection::Editor,
        "Close the card, peek or search",
        "Escape",
    ),
];

/// The application: its windows, what it remembers, and the command
/// line's options for the first window.
pub(super) struct Shell {
    application: adw::Application,
    /// The project windows (closed ones drop out when looked at).
    windows: RefCell<Vec<Weak<Window>>>,
    /// The start page, while it shows. A `Cell`, not a `RefCell`: it is
    /// only taken and set, so no borrow can be held while closing the page
    /// re-enters `start_closed` through GTK's `close-request` signal.
    start: Cell<Option<Rc<StartPage>>>,
    /// The command line's options, taken by the first project window.
    first: Cell<Option<Options>>,
    /// The style the window starts with: the command line's, or the one
    /// remembered.
    theme: Theme,
    /// What is remembered from one run to the next.
    remembered: Cell<Remembered>,
    /// Remembering is on (off for screenshots).
    remembers: bool,
    /// A failure to save the settings has been reported.
    reported: Cell<bool>,
    /// The Open Recent submenu every project window's menu shares.
    recent: gio::Menu,
}

impl Shell {
    /// The application for these options.
    pub(super) fn new(application: &adw::Application, options: Options) -> Rc<Shell> {
        let remembers = options.window_screenshot.is_none();
        let remembered = if remembers {
            Remembered::load()
        } else {
            Remembered::default()
        };
        Rc::new(Shell {
            application: application.clone(),
            windows: RefCell::new(Vec::new()),
            start: Cell::new(None),
            theme: options.theme.unwrap_or(remembered.theme),
            first: Cell::new(Some(options)),
            remembered: Cell::new(remembered),
            remembers,
            reported: Cell::new(false),
            recent: gio::Menu::new(),
        })
    }

    /// The application is starting: the stylesheet, the style, the
    /// application's actions and every action's keys.
    pub(super) fn startup(self: &Rc<Self>) {
        if let Some(display) = gdk::Display::default() {
            style::install(&display);
        }
        gtk::Window::set_default_icon_name(APP_ID);
        adw::StyleManager::default().set_color_scheme(self.theme.scheme());
        let application = &self.application;
        let quit = gio::SimpleAction::new("quit", None);
        let this = Rc::downgrade(self);
        quit.connect_activate(move |_, _| {
            if let Some(shell) = this.upgrade() {
                shell.quit();
            }
        });
        application.add_action(&quit);
        let about = gio::SimpleAction::new("about", None);
        let this = Rc::downgrade(self);
        about.connect_activate(move |_, _| {
            if let Some(shell) = this.upgrade() {
                shell.about();
            }
        });
        application.add_action(&about);
        let shortcuts = gio::SimpleAction::new("shortcuts", None);
        let this = Rc::downgrade(self);
        shortcuts.connect_activate(move |_, _| {
            if let Some(shell) = this.upgrade() {
                shell.shortcuts();
            }
        });
        application.add_action(&shortcuts);
        let style = gio::SimpleAction::new_stateful(
            "style",
            Some(glib::VariantTy::STRING),
            &self.theme.word().to_variant(),
        );
        let this = Rc::downgrade(self);
        style.connect_activate(move |action, parameter| {
            let Some(theme) = parameter
                .and_then(glib::Variant::str)
                .and_then(Theme::from_word)
            else {
                return;
            };
            action.set_state(&theme.word().to_variant());
            if let Some(shell) = this.upgrade() {
                shell.set_theme(theme);
            }
        });
        application.add_action(&style);
        for accelerator in ACCELERATORS {
            application.set_accels_for_action(accelerator.action, accelerator.keys);
        }
        self.refresh_recent();
    }

    /// Stave was started (or started again) with no folder: the window
    /// already open comes forward, or the start page shows.
    pub(super) fn activate(self: &Rc<Self>) {
        if let Some(window) = self.application.active_window() {
            window.present();
            return;
        }
        let new_project = self.first.take().is_some_and(|options| options.new_project);
        let page = StartPage::new(self);
        page.window.present();
        if new_project {
            page.ask_new_project();
        }
        self.start.set(Some(page));
    }

    /// Opens a project folder in a window of its own, or brings forward
    /// the window already showing it. A `.stave` file opens its folder.
    pub(super) fn open_folder(self: &Rc<Self>, folder: &Path) -> Option<Rc<Window>> {
        let folder = match project_folder(folder) {
            Ok(folder) => folder,
            Err(error) => {
                eprintln!("error: {error:#}");
                self.notice(&format!("{error:#}"));
                return None;
            }
        };
        if let Some(window) = self.window_showing(&folder) {
            window.widgets.window.present();
            return Some(window);
        }
        let mut options = self.first.take().unwrap_or_default();
        if options.size.is_none() {
            if self.remembers {
                let remembered = self.remembered.get();
                options.size = Some((
                    u32::try_from(remembered.size.0).unwrap_or(1200),
                    u32::try_from(remembered.size.1).unwrap_or(800),
                ));
                options.maximized = remembered.maximized;
            } else {
                options.size = Some(SCREENSHOT_SIZE);
            }
        }
        match Window::open(&self.application, folder.clone(), options, self.clone()) {
            Ok(window) => {
                self.windows.borrow_mut().push(Rc::downgrade(&window));
                self.record_recent(&folder);
                if let Some(page) = self.start.take() {
                    page.window.close();
                }
                Some(window)
            }
            Err(error) => {
                eprintln!("error: {error:#}");
                self.notice(&format!("{error:#}"));
                None
            }
        }
    }

    /// The application.
    pub(super) fn application(&self) -> &adw::Application {
        &self.application
    }

    /// The start page has closed.
    pub(super) fn start_closed(&self) {
        self.start.take();
    }

    /// The open project windows.
    fn windows(&self) -> Vec<Rc<Window>> {
        let mut windows = self.windows.borrow_mut();
        windows.retain(|window| {
            window
                .upgrade()
                .is_some_and(|window| window.widgets.window.application().is_some())
        });
        windows.iter().filter_map(Weak::upgrade).collect()
    }

    /// The project window showing this folder.
    pub(super) fn window_showing(&self, folder: &Path) -> Option<Rc<Window>> {
        self.windows()
            .into_iter()
            .find(|window| window.session.borrow().folder() == folder)
    }

    /// Closes every window, each asking about its unsaved changes; the
    /// application ends when the last has closed.
    fn quit(&self) {
        for window in self.application.windows() {
            window.close();
        }
    }

    /// Shows a short line in the window in front.
    fn notice(&self, text: &str) {
        let toast = || adw::Toast::builder().title(text).timeout(5).build();
        let active = self.application.active_window();
        let windows = self.windows();
        let in_front = windows
            .iter()
            .find(|window| Some(window.widgets.window.upcast_ref()) == active.as_ref())
            .or(windows.first());
        if let Some(window) = in_front {
            window.widgets.toasts.add_toast(toast());
        } else if let Some(page) = self.start.take() {
            page.toasts.add_toast(toast());
            self.start.set(Some(page));
        }
    }

    /// The About dialog, over the window in front.
    fn about(&self) {
        let dialog = adw::AboutDialog::builder()
            .application_name("Stave")
            .application_icon(APP_ID)
            .developer_name("The Stave contributors")
            .version(env!("CARGO_PKG_VERSION"))
            .comments("Woodworking design for beginners: describe a project in words, see it in 3D, and let Stave check that it can be built.")
            .license_type(gtk::License::Agpl30)
            .website(WEBSITE)
            .issue_url(ISSUES)
            .build();
        dialog.present(self.application.active_window().as_ref());
    }

    /// Every keyboard shortcut, over the window in front.
    fn shortcuts(&self) {
        let dialog = adw::ShortcutsDialog::new();
        for section in KeySection::ALL {
            let group = adw::ShortcutsSection::new(Some(section.title()));
            for (title, keys) in keys_in(section) {
                group.add(adw::ShortcutsItem::new(title, &keys));
            }
            dialog.add(group);
        }
        dialog.present(self.application.active_window().as_ref());
    }

    /// The style was chosen in the menu: it shows and is remembered.
    fn set_theme(&self, theme: Theme) {
        adw::StyleManager::default().set_color_scheme(theme.scheme());
        if !self.remembers {
            return;
        }
        let mut remembered = self.remembered.get();
        remembered.theme = theme;
        self.remember(remembered);
    }

    /// A project window is closing: its size and whether it is maximised
    /// are remembered.
    pub(super) fn remember_window(&self, window: &adw::ApplicationWindow) {
        if !self.remembers {
            return;
        }
        let mut remembered = self.remembered.get();
        let (width, height) = window.default_size();
        if width > 0 && height > 0 {
            remembered.size = (width, height);
        }
        remembered.maximized = window.is_maximized();
        self.remember(remembered);
    }

    fn remember(&self, remembered: Remembered) {
        self.remembered.set(remembered);
        if let Err(error) = remembered.save() {
            eprintln!("warning: {error:#}");
            if !self.reported.replace(true) {
                self.notice(&format!(
                    "Stave couldn't save its settings for next time: {error:#}"
                ));
            }
        }
    }

    /// Puts a project folder at the top of the recent projects.
    pub(super) fn record_recent(&self, folder: &Path) {
        if !self.remembers {
            return;
        }
        let uri = gio::File::for_path(folder).uri();
        let name = folder_name(folder);
        let data = gtk::RecentData::new(
            Some(&name),
            None,
            "inode/directory",
            RECENT_NAME,
            "stave-gtk %u",
            &[],
            false,
        );
        gtk::RecentManager::default().add_full(&uri, &data);
        self.refresh_recent();
    }

    /// The project folders opened lately, newest first, that are still
    /// there.
    pub(super) fn recent_projects(&self) -> Vec<PathBuf> {
        let items = gtk::RecentManager::default()
            .items()
            .into_iter()
            .filter(|item| item.has_application(RECENT_NAME))
            .filter_map(|item| {
                let path = gio::File::for_uri(&item.uri()).path()?;
                Some((path, item.modified().to_unix()))
            })
            .collect();
        newest_folders(items)
    }

    /// Fills the Open Recent submenu again.
    fn refresh_recent(&self) {
        self.recent.remove_all();
        for folder in self.recent_projects() {
            let item = gio::MenuItem::new(Some(&menu_label(&folder_name(&folder))), None);
            item.set_action_and_target_value(
                Some("win.open-recent"),
                Some(&folder.display().to_string().to_variant()),
            );
            self.recent.append_item(&item);
        }
    }

    /// The Open Recent submenu.
    pub(super) fn recent_menu(&self) -> &gio::Menu {
        &self.recent
    }
}

/// The keys the shortcuts dialog lists in a section, in order.
pub(super) fn keys_in(section: KeySection) -> Vec<(&'static str, String)> {
    ACCELERATORS
        .iter()
        .filter(|accelerator| accelerator.section == section)
        .map(|accelerator| (accelerator.title, accelerator.keys.join(" ")))
        .chain(
            OTHER_KEYS
                .iter()
                .filter(|(other, _, _)| *other == section)
                .map(|(_, title, keys)| (*title, (*keys).to_owned())),
        )
        .collect()
}

/// The folder a path opens: the folder itself, or a `.stave` file's
/// folder.
pub(super) fn project_folder(path: &Path) -> Result<PathBuf> {
    let path = path
        .canonicalize()
        .with_context(|| format!("can't find the folder '{}'", path.display()))?;
    if path.is_dir() {
        return Ok(path);
    }
    if path
        .extension()
        .is_some_and(|extension| extension == "stave")
        && let Some(folder) = path.parent()
    {
        return Ok(folder.to_path_buf());
    }
    bail!(
        "'{}' is not a project folder: open the folder that holds its .stave files",
        path.display()
    )
}

/// The newest of these folders first, each once, those still there, no
/// more than the limit.
fn newest_folders(mut items: Vec<(PathBuf, i64)>) -> Vec<PathBuf> {
    items.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let mut folders: Vec<PathBuf> = Vec::new();
    for (path, _) in items {
        if path.is_dir() && !folders.contains(&path) {
            folders.push(path);
        }
        if folders.len() == RECENT_LIMIT {
            break;
        }
    }
    folders
}

/// A folder's own name, or the whole path for the root.
pub(super) fn folder_name(folder: &Path) -> String {
    folder.file_name().map_or_else(
        || folder.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// A menu shows an underscore as the next letter's mnemonic: a folder's
/// own underscores are doubled.
fn menu_label(text: &str) -> String {
    text.replace('_', "__")
}

/// A drop target for a folder (or a `.stave` file) dragged in from Files:
/// `dropped` gets its path.
pub(super) fn folder_drop(dropped: impl Fn(PathBuf) + 'static) -> gtk::DropTarget {
    let target = gtk::DropTarget::new(glib::Type::INVALID, gdk::DragAction::COPY);
    target.set_types(&[gdk::FileList::static_type(), gio::File::static_type()]);
    target.connect_drop(move |_, value, _, _| {
        let file = value
            .get::<gdk::FileList>()
            .ok()
            .and_then(|list| list.files().into_iter().next())
            .or_else(|| value.get::<gio::File>().ok());
        let Some(path) = file.and_then(|file| file.path()) else {
            return false;
        };
        dropped(path);
        true
    });
    target
}

/// The menu's style section, shared by every window.
pub(super) fn style_section() -> gio::Menu {
    let style = gio::Menu::new();
    style.append(Some("Follow System Style"), Some("app.style::system"));
    style.append(Some("Light"), Some("app.style::light"));
    style.append(Some("Dark"), Some("app.style::dark"));
    style
}

/// The menu's last section: the keyboard shortcuts and About.
pub(super) fn help_section() -> gio::Menu {
    let help = gio::Menu::new();
    help.append(Some("Keyboard Shortcuts"), Some("app.shortcuts"));
    help.append(Some("About Stave"), Some("app.about"));
    help
}

#[cfg(test)]
mod tests {
    use super::*;

    const DESKTOP: &str = include_str!("../../../../data/io.github.OceanEilonwy.Stave.desktop");
    const METAINFO: &str =
        include_str!("../../../../data/io.github.OceanEilonwy.Stave.metainfo.xml");

    #[test]
    fn the_data_files_carry_the_application_id() {
        assert!(DESKTOP.contains(&format!("Icon={APP_ID}\n")));
        assert!(DESKTOP.contains("Exec=stave-gtk %U\n"));
        assert!(METAINFO.contains(&format!("<id>{APP_ID}</id>")));
        assert!(METAINFO.contains(&format!(
            "<launchable type=\"desktop-id\">{APP_ID}.desktop</launchable>"
        )));
        let icons = concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/icons/hicolor");
        for icon in [
            format!("{icons}/scalable/apps/{APP_ID}.svg"),
            format!("{icons}/symbolic/apps/{APP_ID}-symbolic.svg"),
        ] {
            assert!(Path::new(&icon).is_file(), "{icon} is missing");
        }
    }

    #[test]
    fn the_metainfo_names_this_version() {
        assert!(
            METAINFO.contains(&format!("version=\"{}\"", env!("CARGO_PKG_VERSION"))),
            "add a <release> for this version to the metainfo"
        );
    }

    #[test]
    fn every_key_is_listed_and_parses() {
        let mut listed = 0;
        for section in KeySection::ALL {
            listed += keys_in(section).len();
        }
        assert_eq!(listed, ACCELERATORS.len() + OTHER_KEYS.len());
        let keys: Vec<String> = KeySection::ALL
            .into_iter()
            .flat_map(keys_in)
            .flat_map(|(_, keys)| keys.split(' ').map(str::to_owned).collect::<Vec<String>>())
            .collect();
        crate::ui::gtk_test::on_gtk_thread(move || {
            for key in keys {
                assert!(
                    gtk::accelerator_parse(&key).is_some(),
                    "{key} doesn't parse"
                );
            }
        });
    }

    #[test]
    fn a_stave_file_opens_its_folder() {
        let folder = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/wall_shelf");
        let canonical = Path::new(folder).canonicalize().unwrap();
        assert_eq!(project_folder(Path::new(folder)).unwrap(), canonical);
        let file = std::fs::read_dir(&canonical)
            .unwrap()
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .find(|path| path.extension().is_some_and(|e| e == "stave"))
            .unwrap();
        assert_eq!(project_folder(&file).unwrap(), canonical);
        let cargo = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml");
        assert!(project_folder(Path::new(cargo)).is_err());
        assert!(project_folder(Path::new("/no/such/folder")).is_err());
    }

    #[test]
    fn recent_folders_are_newest_first_once_each() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let shelf = root.join("examples/wall_shelf");
        let stool = root.join("examples/stool");
        let gone = root.join("examples/no_such_project");
        let folders = newest_folders(vec![
            (shelf.clone(), 10),
            (stool.clone(), 30),
            (gone, 40),
            (shelf.clone(), 20),
        ]);
        assert_eq!(folders, vec![stool, shelf]);
    }

    #[test]
    fn underscores_survive_the_menu() {
        assert_eq!(menu_label("hall_table"), "hall__table");
    }
}
