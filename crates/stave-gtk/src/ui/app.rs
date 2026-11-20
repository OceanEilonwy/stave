//! The application round the windows: one process for every project
//! window. Launching Stave again, or opening a folder with it from Files,
//! hands the folder to the running Stave, which opens a window for it
//! (or brings forward the one already showing it). Started with no
//! folder, it shows the start page (`start_page`).
//!
//! Here too: what belongs to the application rather than one window
//! (Quit, About, the keyboard shortcuts, the style, recent projects) and
//! what it remembers between runs (`remember`).

use stave_app::i18n::{fill_in, tr, tr_noop};
use std::path::{Path, PathBuf};

use adw::prelude::*;
use anyhow::{Context as _, Result, bail};
use gtk::{gdk, gio, glib};
use relm4::{Component, ComponentController, ComponentParts, ComponentSender, Controller};

use super::launch::Launch;
use super::remember::Remembered;
use super::start_page::StartPage;
use super::update::project::ProjectInput;
use super::{Options, Theme, Window, WindowInit, WindowInput, WindowOutput, build_widgets, style};

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
            KeySection::General => tr_noop("General"),
            KeySection::View => tr_noop("3D View"),
            KeySection::Editor => tr_noop("Editor"),
        }
    }
}

/// Where an action's keys are bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum KeyScope {
    /// By the application (`set_accels_for_action`): anywhere in a
    /// window, before the focused widget sees the key.
    Application,
    /// By the window, after the focused widget had its turn, and never
    /// while the keyboard is in a text field: plain letters and Escape,
    /// which typing needs.
    Window,
    /// By the text editor, while the keyboard is in the text.
    Editor,
    /// By the 3D view, while the keyboard is in it: the arrow keys and
    /// plus and minus, which other widgets use for themselves.
    View,
}

/// An action with keys: what binds them (`scope`), and what the shortcuts
/// dialog lists.
pub(super) struct Accelerator {
    /// The action, `win.check`.
    pub(super) action: &'static str,
    /// Its keys, as GTK writes them.
    pub(super) keys: &'static [&'static str],
    /// What the shortcuts dialog calls it.
    pub(super) title: &'static str,
    /// Where it lists it.
    pub(super) section: KeySection,
    /// What binds the keys.
    pub(super) scope: KeyScope,
}

/// Every action that has keys.
pub(super) const ACCELERATORS: &[Accelerator] = &[
    Accelerator {
        action: "win.new-project",
        keys: &["<Control>n"],
        title: tr_noop("New project"),
        section: KeySection::General,
        scope: KeyScope::Application,
    },
    Accelerator {
        action: "win.open",
        keys: &["<Control>o"],
        title: tr_noop("Open a project folder"),
        section: KeySection::General,
        scope: KeyScope::Application,
    },
    Accelerator {
        action: "win.check",
        keys: &["<Control>r"],
        title: tr_noop("Check the project again"),
        section: KeySection::General,
        scope: KeyScope::Application,
    },
    Accelerator {
        action: "win.find",
        keys: &["<Control>k"],
        title: tr_noop("Find a part, a problem or a name"),
        section: KeySection::General,
        scope: KeyScope::Application,
    },
    Accelerator {
        action: "win.next-problem",
        keys: &["F8"],
        title: tr_noop("Next problem"),
        section: KeySection::General,
        scope: KeyScope::Application,
    },
    Accelerator {
        action: "win.previous-problem",
        keys: &["<Shift>F8"],
        title: tr_noop("Previous problem"),
        section: KeySection::General,
        scope: KeyScope::Application,
    },
    Accelerator {
        action: "win.back",
        keys: &["<Alt>Left"],
        title: tr_noop("Go back"),
        section: KeySection::General,
        scope: KeyScope::Application,
    },
    Accelerator {
        action: "win.forward",
        keys: &["<Alt>Right"],
        title: tr_noop("Go forward"),
        section: KeySection::General,
        scope: KeyScope::Application,
    },
    Accelerator {
        action: "win.model",
        keys: &["F9"],
        title: tr_noop("Show the parts"),
        section: KeySection::General,
        scope: KeyScope::Application,
    },
    Accelerator {
        action: "app.shortcuts",
        keys: &["<Control>question"],
        title: tr_noop("Keyboard shortcuts"),
        section: KeySection::General,
        scope: KeyScope::Application,
    },
    Accelerator {
        action: "window.close",
        keys: &["<Control>w"],
        title: tr_noop("Close the window"),
        section: KeySection::General,
        scope: KeyScope::Application,
    },
    Accelerator {
        action: "app.quit",
        keys: &["<Control>q"],
        title: tr_noop("Quit"),
        section: KeySection::General,
        scope: KeyScope::Application,
    },
    Accelerator {
        action: "win.view-design",
        keys: &["<Control>1"],
        title: tr_noop("Show the design"),
        section: KeySection::View,
        scope: KeyScope::Application,
    },
    Accelerator {
        action: "win.frame",
        keys: &["f"],
        title: tr_noop("Frame the selection"),
        section: KeySection::View,
        scope: KeyScope::Window,
    },
    Accelerator {
        action: "win.escape",
        keys: &["Escape"],
        title: tr_noop("Leave focus, or clear the selection"),
        section: KeySection::View,
        scope: KeyScope::Window,
    },
    Accelerator {
        action: "view.turn-left",
        keys: &["Left"],
        title: tr_noop("Turn the view left"),
        section: KeySection::View,
        scope: KeyScope::View,
    },
    Accelerator {
        action: "view.turn-right",
        keys: &["Right"],
        title: tr_noop("Turn the view right"),
        section: KeySection::View,
        scope: KeyScope::View,
    },
    Accelerator {
        action: "view.look-higher",
        keys: &["Up"],
        title: tr_noop("Look from higher up"),
        section: KeySection::View,
        scope: KeyScope::View,
    },
    Accelerator {
        action: "view.look-lower",
        keys: &["Down"],
        title: tr_noop("Look from lower down"),
        section: KeySection::View,
        scope: KeyScope::View,
    },
    Accelerator {
        action: "view.zoom-in",
        keys: &["plus", "equal", "KP_Add"],
        title: tr_noop("Zoom in"),
        section: KeySection::View,
        scope: KeyScope::View,
    },
    Accelerator {
        action: "view.zoom-out",
        keys: &["minus", "KP_Subtract"],
        title: tr_noop("Zoom out"),
        section: KeySection::View,
        scope: KeyScope::View,
    },
    Accelerator {
        action: "win.editor",
        keys: &["<Control>e"],
        title: tr_noop("Open the editor"),
        section: KeySection::Editor,
        scope: KeyScope::Application,
    },
    Accelerator {
        action: "win.save",
        keys: &["<Control>s"],
        title: tr_noop("Save the file"),
        section: KeySection::Editor,
        scope: KeyScope::Application,
    },
    Accelerator {
        action: "win.format",
        keys: &["<Control><Shift>i"],
        title: tr_noop("Tidy the file's layout"),
        section: KeySection::Editor,
        scope: KeyScope::Application,
    },
    Accelerator {
        action: "win.search",
        keys: &["<Control>f"],
        title: tr_noop("Search in the file"),
        section: KeySection::Editor,
        scope: KeyScope::Application,
    },
    Accelerator {
        action: "editor.follow",
        keys: &["F12"],
        title: tr_noop("Go to where a name is made (or Ctrl+click the name)"),
        section: KeySection::Editor,
        scope: KeyScope::Editor,
    },
    Accelerator {
        action: "editor.peek",
        keys: &["<Alt>F12"],
        title: tr_noop("Peek at where a name is made"),
        section: KeySection::Editor,
        scope: KeyScope::Editor,
    },
    Accelerator {
        action: "editor.uses",
        keys: &["<Shift>F12"],
        title: tr_noop("List where a name is used"),
        section: KeySection::Editor,
        scope: KeyScope::Editor,
    },
    Accelerator {
        action: "editor.show-info",
        keys: &["<Control>i"],
        title: tr_noop("Show the card for the word at the cursor"),
        section: KeySection::Editor,
        scope: KeyScope::Editor,
    },
];

/// Keys that a widget handles itself, not through an action: the
/// shortcuts dialog lists them too.
pub(super) const OTHER_KEYS: &[(KeySection, &str, &str)] = &[
    (
        KeySection::View,
        tr_noop("Remove the selected piece"),
        "Delete",
    ),
    (KeySection::Editor, tr_noop("Undo"), "<Control>z"),
    (KeySection::Editor, tr_noop("Redo"), "<Control><Shift>z"),
    (
        KeySection::Editor,
        tr_noop("Close the card, peek or search"),
        "Escape",
    ),
];

/// What one of the application's own actions sends.
type ShellAction = fn() -> ShellInput;

/// Which project window: numbered as they open, never reused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct WindowId(u64);

/// A project window the application owns while it is open.
struct OpenWindow {
    id: WindowId,
    /// The folder it shows.
    folder: PathBuf,
    /// Its component: dropping it shuts the window's component down.
    controller: Controller<Window>,
}

/// The application: a relm4 component with no widget of its own. It owns
/// each project window's [`Controller`] from opening until GTK takes its
/// window out of the application (GtkApplication's `window-removed`, when
/// the window is closed), and the start page while it shows. GApplication's signals, the application's
/// actions, the windows and the start page only send it [`ShellInput`]s.
pub(super) struct Shell {
    application: adw::Application,
    /// The open project windows, in the order they opened.
    windows: Vec<OpenWindow>,
    /// The number the next window gets.
    next_window: u64,
    /// The start page, while it shows.
    start: Option<StartPage>,
    /// The command line's options, taken by the first project window.
    first: Option<Options>,
    /// The style shown: the command line's or the one remembered at
    /// first, then the menu's or a later launch's.
    theme: Theme,
    /// What is remembered from one run to the next.
    remembered: Remembered,
    /// Remembering is on (off for screenshots).
    remembers: bool,
    /// A failure to save the settings has been reported.
    reported: bool,
    /// The Open Recent submenu every project window's menu shares.
    recent: gio::Menu,
    /// Where the application's own messages go.
    sender: ComponentSender<Shell>,
}

/// What the application is made with.
pub(super) struct ShellInit {
    /// The GApplication it runs in.
    pub(super) application: adw::Application,
    /// The command line's options.
    pub(super) options: Options,
}

/// What the application is told.
#[derive(Debug)]
pub(super) enum ShellInput {
    /// GApplication started: the style, the actions and their keys.
    Startup,
    /// Stave was started (or started again) with no folder. The hold
    /// keeps the application running until a window shows.
    Activate(gio::ApplicationHoldGuard),
    /// Stave was asked to open these folders.
    Open(Vec<PathBuf>, gio::ApplicationHoldGuard),
    /// Stave was launched (the first time, or again while running): the
    /// style it names, then its folders or, given none, as `Activate`.
    Launch(Launch, gio::ApplicationHoldGuard),
    /// Opens a project folder, or brings forward the window showing it.
    OpenFolder(PathBuf),
    /// Opens a project just written by the New Project dialog, and
    /// welcomes it.
    OpenNewProject(PathBuf),
    /// The Quit action.
    Quit,
    /// The About action.
    About,
    /// The Keyboard Shortcuts action.
    Shortcuts,
    /// A style was chosen in the menu.
    SetTheme(Theme),
    /// The start page has closed.
    StartClosed,
    /// A project window said something.
    Window(WindowId, WindowOutput),
    /// GTK took this window out of the application: it has closed.
    WindowRemoved(gtk::Window),
}

impl Component for Shell {
    type Init = ShellInit;
    type Input = ShellInput;
    type Output = ();
    type CommandOutput = ();
    type Root = ();
    type Widgets = ();

    fn init_root() {}

    fn init(init: ShellInit, (): (), sender: ComponentSender<Shell>) -> ComponentParts<Shell> {
        let ShellInit {
            application,
            options,
        } = init;
        let remembers = options.window_screenshot.is_none();
        let remembered = if remembers {
            Remembered::load()
        } else {
            Remembered::default()
        };
        // A window leaves the application when it is closed: its
        // controller goes then.
        let input = sender.input_sender().clone();
        application.connect_window_removed(move |_, window| {
            input.emit(ShellInput::WindowRemoved(window.clone()));
        });
        let model = Shell {
            application,
            windows: Vec::new(),
            next_window: 0,
            start: None,
            theme: options.theme.unwrap_or(remembered.theme),
            first: Some(options),
            remembered,
            remembers,
            reported: false,
            recent: gio::Menu::new(),
            sender,
        };
        ComponentParts { model, widgets: () }
    }

    fn update(&mut self, message: ShellInput, _sender: ComponentSender<Shell>, (): &()) {
        match message {
            ShellInput::Startup => self.startup(),
            ShellInput::Activate(hold) => {
                self.activate();
                drop(hold);
            }
            ShellInput::Open(folders, hold) => {
                for folder in folders {
                    self.open_folder(&folder);
                }
                drop(hold);
            }
            ShellInput::Launch(launch, hold) => {
                // App-wide, as the style menu: a second launch with
                // `--theme` restyles every open window.
                if let Some(theme) = launch.theme {
                    self.show_theme(theme);
                }
                if launch.folders.is_empty() {
                    self.activate();
                }
                for folder in launch.folders {
                    self.open_folder(&folder);
                }
                drop(hold);
            }
            ShellInput::OpenFolder(folder) => {
                self.open_folder(&folder);
            }
            ShellInput::OpenNewProject(folder) => {
                if let Some(id) = self.open_folder(&folder) {
                    self.send_to(id, ProjectInput::Welcome);
                }
            }
            ShellInput::Quit => self.quit(),
            ShellInput::About => self.about(),
            ShellInput::Shortcuts => self.shortcuts(),
            ShellInput::SetTheme(theme) => self.set_theme(theme),
            ShellInput::StartClosed => self.start = None,
            ShellInput::Window(id, output) => self.window_said(id, output),
            // Dropping the controller shuts the window's component down
            // (its model and widgets go when the main loop next runs).
            ShellInput::WindowRemoved(window) => self
                .windows
                .retain(|open| open.controller.widget().upcast_ref::<gtk::Window>() != &window),
        }
    }
}

impl Shell {
    /// The application is starting: the stylesheet, the style, the
    /// application's actions and every action's keys.
    fn startup(&mut self) {
        if let Some(display) = gdk::Display::default() {
            style::install(&display);
            super::edit_view::changes::install_style(&display);
            super::edit_view::grip_panel::install_style(&display);
        }
        gtk::Window::set_default_icon_name(APP_ID);
        self.theme.apply();
        let application = &self.application;
        let actions: [(&str, ShellAction); 3] = [
            ("quit", || ShellInput::Quit),
            ("about", || ShellInput::About),
            ("shortcuts", || ShellInput::Shortcuts),
        ];
        for (name, message) in actions {
            let action = gio::SimpleAction::new(name, None);
            let input = self.sender.input_sender().clone();
            action.connect_activate(move |_, _| input.emit(message()));
            application.add_action(&action);
        }
        let style = gio::SimpleAction::new_stateful(
            "style",
            Some(glib::VariantTy::STRING),
            &self.theme.word().to_variant(),
        );
        let input = self.sender.input_sender().clone();
        style.connect_activate(move |action, parameter| {
            let Some(theme) = parameter
                .and_then(glib::Variant::str)
                .and_then(Theme::from_word)
            else {
                return;
            };
            action.set_state(&theme.word().to_variant());
            input.emit(ShellInput::SetTheme(theme));
        });
        application.add_action(&style);
        for accelerator in ACCELERATORS
            .iter()
            .filter(|accelerator| accelerator.scope == KeyScope::Application)
        {
            application.set_accels_for_action(accelerator.action, accelerator.keys);
        }
        self.refresh_recent();
    }

    /// Stave was started (or started again) with no folder: the window
    /// already open comes forward, or the start page shows.
    fn activate(&mut self) {
        if let Some(window) = self.application.active_window() {
            window.present();
            return;
        }
        let new_project = self.first.take().is_some_and(|options| options.new_project);
        let page = StartPage::new(
            &self.application,
            self.recent_projects(),
            self.sender.input_sender().clone(),
        );
        page.window.present();
        if new_project {
            page.ask_new_project();
        }
        self.start = Some(page);
    }

    /// Opens a project folder in a window of its own, or brings forward
    /// the window already showing it. A `.stave` file opens its folder.
    /// The window it opened, if it opened one.
    fn open_folder(&mut self, folder: &Path) -> Option<WindowId> {
        let folder = match project_folder(folder) {
            Ok(folder) => folder,
            Err(error) => {
                eprintln!("error: {error:#}");
                self.notice(&format!("{error:#}"));
                return None;
            }
        };
        if let Some(open) = self.windows.iter().find(|open| open.folder == folder) {
            open.controller.widget().present();
            return Some(open.id);
        }
        let mut options = self.first.take().unwrap_or_default();
        if options.size.is_none() {
            if self.remembers {
                let remembered = self.remembered;
                options.size = Some((
                    u32::try_from(remembered.size.0).unwrap_or(1200),
                    u32::try_from(remembered.size.1).unwrap_or(800),
                ));
                options.maximized = remembered.maximized;
            } else {
                options.size = Some(SCREENSHOT_SIZE);
            }
        }
        let builder = Window::builder();
        let root = builder.widget().clone();
        let widgets = match build_widgets(&root, &self.application, &folder, &self.recent, &options)
        {
            Ok(widgets) => widgets,
            Err(error) => {
                // The root was never shown; nothing else holds it.
                root.destroy();
                eprintln!("error: {error:#}");
                self.notice(&format!("{error:#}"));
                return None;
            }
        };
        let id = WindowId(self.next_window);
        self.next_window = self.next_window.saturating_add(1);
        let controller = builder
            .launch(WindowInit {
                widgets,
                folder: folder.clone(),
                options,
            })
            .forward(self.sender.input_sender(), move |output| {
                ShellInput::Window(id, output)
            });
        self.windows.push(OpenWindow {
            id,
            folder: folder.clone(),
            controller,
        });
        self.record_recent(&folder);
        if let Some(page) = self.start.take() {
            page.window.close();
        }
        Some(id)
    }

    /// A project window said something.
    fn window_said(&mut self, id: WindowId, output: WindowOutput) {
        match output {
            WindowOutput::GoToFolder(folder) => {
                if let Some(open) = self.windows.iter().find(|open| open.folder == folder) {
                    open.controller.widget().present();
                } else {
                    self.send_to(id, ProjectInput::OpenHere(folder));
                }
            }
            WindowOutput::Opened(folder) => {
                self.record_recent(&folder);
                if let Some(open) = self.windows.iter_mut().find(|open| open.id == id) {
                    open.folder = folder;
                }
            }
            WindowOutput::Closing {
                size,
                maximized,
                hold,
            } => {
                self.remember_window(size, maximized);
                drop(hold);
            }
        }
    }

    /// Sends a message to a project window.
    fn send_to(&self, id: WindowId, message: impl Into<WindowInput>) {
        if let Some(open) = self.windows.iter().find(|open| open.id == id) {
            // Fails only once the window has shut down.
            let _ = open.controller.sender().send(message.into());
        }
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
        let active = self.application.active_window();
        let in_front = self
            .windows
            .iter()
            .find(|open| Some(open.controller.widget().upcast_ref()) == active.as_ref())
            .or(self.windows.first());
        if let Some(open) = in_front {
            self.send_to(
                open.id,
                ProjectInput::Notice {
                    text: text.to_owned(),
                    seconds: 5,
                },
            );
        } else if let Some(page) = &self.start {
            page.toasts
                .add_toast(adw::Toast::builder().title(text).timeout(5).build());
        }
    }

    /// The About dialog, over the window in front.
    fn about(&self) {
        let dialog = adw::AboutDialog::builder()
            .application_name("Stave")
            .application_icon(APP_ID)
            .developer_name(tr("The Stave contributors"))
            .version(env!("CARGO_PKG_VERSION"))
            .comments(tr("Woodworking design for beginners: describe a project in words, see it in 3D, and let Stave check that it can be built."))
            .translator_credits(tr(
                // TRANSLATORS: your name, and others who translated, one per line.
                "translator-credits",
            ))
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
            let group = adw::ShortcutsSection::new(Some(&tr(section.title())));
            for (title, keys) in keys_in(section) {
                group.add(adw::ShortcutsItem::new(&tr(title), &keys));
            }
            dialog.add(group);
        }
        dialog.present(self.application.active_window().as_ref());
    }

    /// Shows a style in every window, and ticks it in the style menu. Not
    /// remembered: `--theme` from a launch lasts for this run, as it does
    /// at startup.
    fn show_theme(&mut self, theme: Theme) {
        self.theme = theme;
        theme.apply();
        if let Some(style) = self
            .application
            .lookup_action("style")
            .and_downcast::<gio::SimpleAction>()
        {
            style.set_state(&theme.word().to_variant());
        }
    }

    /// The style was chosen in the menu: it shows and is remembered.
    fn set_theme(&mut self, theme: Theme) {
        self.show_theme(theme);
        if !self.remembers {
            return;
        }
        let mut remembered = self.remembered;
        remembered.theme = theme;
        self.remember(remembered);
    }

    /// A project window is closing: its size and whether it is maximised
    /// are remembered.
    fn remember_window(&mut self, (width, height): (i32, i32), maximized: bool) {
        if !self.remembers {
            return;
        }
        let mut remembered = self.remembered;
        if width > 0 && height > 0 {
            remembered.size = (width, height);
        }
        remembered.maximized = maximized;
        self.remember(remembered);
    }

    fn remember(&mut self, remembered: Remembered) {
        self.remembered = remembered;
        if let Err(error) = remembered.save() {
            eprintln!("warning: {error:#}");
            if !std::mem::replace(&mut self.reported, true) {
                self.notice(&fill_in(
                    &tr("Stave couldn't save its settings for next time: {error}"),
                    &[("error", &format!("{error:#}"))],
                ));
            }
        }
    }

    /// Puts a project folder at the top of the recent projects.
    fn record_recent(&self, folder: &Path) {
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
    fn recent_projects(&self) -> Vec<PathBuf> {
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
}

/// The shortcuts for the actions bound in a scope (other than the
/// application's): added to the window, or to the text editor, by whoever
/// owns that scope. A shortcut does nothing, and lets the key go on, while
/// the keyboard is in a text field (for the window's) or while its action
/// is disabled.
pub(super) fn shortcut_controller(scope: KeyScope) -> gtk::ShortcutController {
    let controller = gtk::ShortcutController::new();
    for accelerator in ACCELERATORS
        .iter()
        .filter(|accelerator| accelerator.scope == scope)
    {
        for key in accelerator.keys {
            let Some(trigger) = gtk::ShortcutTrigger::parse_string(key) else {
                continue;
            };
            let action: gtk::ShortcutAction = match scope {
                KeyScope::Window => window_action(accelerator.action).upcast(),
                KeyScope::Application | KeyScope::Editor | KeyScope::View => {
                    gtk::NamedAction::new(accelerator.action).upcast()
                }
            };
            controller.add_shortcut(gtk::Shortcut::new(Some(trigger), Some(action)));
        }
    }
    controller
}

/// A window shortcut's action: the named action, unless the keyboard is
/// in a text field, which keeps its plain letters and Escape.
fn window_action(name: &'static str) -> gtk::CallbackAction {
    gtk::CallbackAction::new(move |widget, _| {
        let typing = widget
            .root()
            .and_then(|root| gtk::prelude::RootExt::focus(&root))
            .is_some_and(|focus| focus.is::<gtk::Text>() || focus.is::<gtk::TextView>());
        if typing || widget.activate_action(name, None).is_err() {
            return glib::Propagation::Proceed;
        }
        glib::Propagation::Stop
    })
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
    let path = path.canonicalize().with_context(|| {
        fill_in(
            &tr("can't find the folder '{folder}'"),
            &[("folder", &path.display())],
        )
    })?;
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
    bail!(fill_in(
        &tr("'{folder}' is not a project folder: open the folder that holds its .stave files"),
        &[("folder", &path.display())],
    ))
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
    style.append(Some(&tr("Follow System Style")), Some("app.style::system"));
    style.append(Some(&tr("Light")), Some("app.style::light"));
    style.append(Some(&tr("Dark")), Some("app.style::dark"));
    style
}

/// The menu's last section: the keyboard shortcuts and About.
pub(super) fn help_section() -> gio::Menu {
    let help = gio::Menu::new();
    help.append(Some(&tr("Keyboard Shortcuts")), Some("app.shortcuts"));
    help.append(Some(&tr("About Stave")), Some("app.about"));
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
    fn keys_a_widget_handled_itself_are_now_actions() {
        // Frame, Escape in the 3D view and the editor's F12 family run
        // actions, which the shortcuts dialog lists from the same table.
        for (action, key) in [
            ("win.frame", "f"),
            ("win.escape", "Escape"),
            ("editor.follow", "F12"),
            ("editor.peek", "<Alt>F12"),
            ("editor.uses", "<Shift>F12"),
            ("editor.show-info", "<Control>i"),
        ] {
            assert!(
                ACCELERATORS
                    .iter()
                    .any(|a| a.action == action && a.keys.contains(&key)),
                "{action} has no {key}"
            );
            let listed_twice = OTHER_KEYS.iter().any(|(section, _, keys)| {
                *keys == key && (*section == KeySection::View) == action.starts_with("win.")
            });
            assert!(!listed_twice, "{key} is listed twice");
        }
        // A plain letter can't be an application key: it would take the
        // letter from every text field.
        for accelerator in ACCELERATORS {
            if accelerator.scope == KeyScope::Application {
                assert!(
                    accelerator
                        .keys
                        .iter()
                        .all(|key| key.contains('<') || key.starts_with('F')),
                    "{} would take typing",
                    accelerator.action
                );
            }
        }
    }

    #[test]
    fn shortcuts_run_their_actions_but_leave_typing_alone() {
        crate::ui::gtk_test::on_gtk_thread(shortcuts_run_actions);
    }

    fn shortcuts_run_actions() {
        use std::cell::RefCell;
        use std::rc::Rc;
        let ran: Rc<RefCell<Vec<String>>> = Rc::default();
        let group = gio::SimpleActionGroup::new();
        for name in ["follow", "peek", "uses", "show-info", "frame", "escape"] {
            let action = gio::SimpleAction::new(name, None);
            let ran = Rc::clone(&ran);
            action.connect_activate(move |action, _| ran.borrow_mut().push(action.name().into()));
            group.add_action(&action);
        }
        let text = gtk::TextView::new();
        let button = gtk::Button::with_label("a button");
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.append(&text);
        content.append(&button);
        let window = gtk::Window::builder().child(&content).build();
        text.insert_action_group("editor", Some(&group));
        window.insert_action_group("win", Some(&group));
        window.present();
        let activate = |controller: &gtk::ShortcutController, widget: &gtk::Widget, key: &str| {
            let shortcut = (0..controller.n_items())
                .filter_map(|index| controller.item(index).and_downcast::<gtk::Shortcut>())
                .find(|shortcut| {
                    shortcut.trigger().is_some_and(|t| {
                        t.to_str() == gtk::ShortcutTrigger::parse_string(key).unwrap().to_str()
                    })
                })
                .unwrap_or_else(|| panic!("no shortcut for {key}"));
            shortcut
                .action()
                .unwrap()
                .activate(gtk::ShortcutActionFlags::empty(), widget, None)
        };
        let editor = shortcut_controller(KeyScope::Editor);
        for key in ["F12", "<Alt>F12", "<Shift>F12", "<Control>i"] {
            assert!(activate(&editor, text.upcast_ref(), key));
        }
        assert_eq!(*ran.borrow(), ["follow", "peek", "uses", "show-info"]);
        ran.borrow_mut().clear();
        // The window's keys: not while typing, but from a button.
        let shortcuts = shortcut_controller(KeyScope::Window);
        text.grab_focus();
        assert!(!activate(&shortcuts, window.upcast_ref(), "f"));
        assert!(ran.borrow().is_empty());
        button.grab_focus();
        assert!(activate(&shortcuts, window.upcast_ref(), "f"));
        assert!(activate(&shortcuts, window.upcast_ref(), "Escape"));
        assert_eq!(*ran.borrow(), ["frame", "escape"]);
        window.destroy();
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
