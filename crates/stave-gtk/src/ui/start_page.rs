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
//!
//! The page's window is a relm4 component, [`StartPageComponent`]: its
//! actions, rows and drop target only send [`StartPageInput`] messages,
//! and it tells its owner what people asked for as [`StartPageOutput`]s.
//! [`StartPage`] keeps the older API the application uses and does what
//! they ask with the application ([`Shell`]).

use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};
use relm4::{Component, ComponentParts, ComponentSender};

use super::app::{self, APP_ID, Shell};
use super::relm4_shim::Bridge;
use super::{new_project, shown_path};

/// The start page has no state of its own: what it lists is fixed when
/// it opens, and what people ask for goes to its owner.
#[derive(Debug)]
pub(super) struct StartPageComponent;

/// What a start page is made with.
#[derive(Debug)]
pub(super) struct StartPageInit {
    /// The application its window belongs to.
    pub(super) application: adw::Application,
    /// The recent projects, newest first.
    pub(super) recent: Vec<PathBuf>,
    /// Where its short lines show; the application adds them.
    pub(super) toasts: adw::ToastOverlay,
}

/// What the start page is told.
#[derive(Debug)]
pub(super) enum StartPageInput {
    /// People asked to open a project folder.
    Open,
    /// People asked to start a new project.
    Create,
    /// People chose a recent project.
    OpenRecent(PathBuf),
    /// People dropped a folder on the page.
    Dropped(PathBuf),
    /// People asked to close the page.
    CloseRequested,
    /// Ask for a new project, as if people had.
    AskNewProject,
}

/// What the start page tells its owner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum StartPageOutput {
    /// Ask for a project folder and open it.
    Open,
    /// Ask for a new project and open it.
    Create,
    /// Open this recent project.
    OpenRecent(PathBuf),
    /// Open this folder, dropped on the page.
    Dropped(PathBuf),
    /// The page's window is closing.
    CloseRequest,
}

/// The recent projects, one row each; choosing one sends its folder.
fn recent_list(
    recent: &[PathBuf],
    sender: &ComponentSender<StartPageComponent>,
) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder()
        .title("Recent Projects")
        .build();
    for folder in recent {
        let row = adw::ActionRow::builder()
            .title(app::folder_name(folder))
            .subtitle(shown_path(folder))
            .use_markup(false)
            .activatable(true)
            .build();
        row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
        let input = sender.input_sender().clone();
        let folder = folder.clone();
        row.connect_activated(move |_| input.emit(StartPageInput::OpenRecent(folder.clone())));
        group.add(&row);
    }
    group
}

/// The page's main menu: Project, Style, Help.
fn main_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    let project = gio::Menu::new();
    project.append(Some("New Project…"), Some("win.new-project"));
    project.append(Some("Open Project…"), Some("win.open"));
    menu.append_section(None, &project);
    menu.append_section(Some("Style"), &app::style_section());
    menu.append_section(None, &app::help_section());
    menu
}

/// A window action that sends `message` when activated.
fn action(
    name: &str,
    sender: &ComponentSender<StartPageComponent>,
    message: impl Fn() -> StartPageInput + 'static,
) -> gio::SimpleAction {
    let action = gio::SimpleAction::new(name, None);
    let input = sender.input_sender().clone();
    action.connect_activate(move |_, _| input.emit(message()));
    action
}

#[relm4::component(pub(super))]
impl Component for StartPageComponent {
    type Init = StartPageInit;
    type Input = StartPageInput;
    type Output = StartPageOutput;
    type CommandOutput = ();

    view! {
        adw::ApplicationWindow {
            set_application: Some(&application),
            set_title: Some("Stave"),
            set_default_width: 720,
            set_default_height: 640,
            set_width_request: 360,
            set_height_request: 400,
            add_action: &action("open", &sender, || StartPageInput::Open),
            add_action: &action("new-project", &sender, || StartPageInput::Create),
            add_controller: app::folder_drop(move |path| {
                dropped.emit(StartPageInput::Dropped(path));
            }),
            connect_close_request[sender] => move |_| {
                sender.input_sender().emit(StartPageInput::CloseRequested);
                glib::Propagation::Proceed
            },

            #[wrap(Some)]
            set_content = &adw::ToolbarView {
                add_top_bar = &adw::HeaderBar {
                    pack_end = &gtk::MenuButton {
                        set_icon_name: "open-menu-symbolic",
                        set_menu_model: Some(&main_menu()),
                        set_primary: true,
                        set_tooltip_text: Some("Main Menu"),
                    },
                },

                #[local_ref]
                #[wrap(Some)]
                set_content = toasts -> adw::ToastOverlay {
                    #[wrap(Some)]
                    set_child = &gtk::ScrolledWindow {
                        set_hscrollbar_policy: gtk::PolicyType::Never,

                        adw::StatusPage {
                            set_icon_name: Some(icon),
                            set_title: "Stave",
                            set_description: Some(
                                "Open a project folder to see its design, or start a new project. \
                                 You can also drop a folder here.",
                            ),
                            set_vexpand: true,

                            #[wrap(Some)]
                            set_child = &adw::Clamp {
                                set_maximum_size: 520,

                                #[name = "content"]
                                gtk::Box {
                                    set_orientation: gtk::Orientation::Vertical,
                                    set_spacing: 36,

                                    gtk::Box {
                                        set_spacing: 12,
                                        set_halign: gtk::Align::Center,

                                        gtk::Button {
                                            set_label: "_Open Folder…",
                                            set_use_underline: true,
                                            set_action_name: Some("win.open"),
                                            add_css_class: "pill",
                                            add_css_class: "suggested-action",
                                        },
                                        gtk::Button {
                                            set_label: "_New Project…",
                                            set_use_underline: true,
                                            set_action_name: Some("win.new-project"),
                                            add_css_class: "pill",
                                        },
                                    },
                                },
                            },
                        },
                    },
                },
            },
        }
    }

    fn init(
        init: StartPageInit,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let StartPageInit {
            application,
            recent,
            toasts,
        } = init;
        let toasts = &toasts;
        let has_icon = gtk::gdk::Display::default()
            .is_some_and(|display| gtk::IconTheme::for_display(&display).has_icon(APP_ID));
        let icon = if has_icon {
            APP_ID
        } else {
            "folder-open-symbolic"
        };
        let dropped = sender.input_sender().clone();
        let model = StartPageComponent;
        let widgets = view_output!();
        if !recent.is_empty() {
            widgets.content.append(&recent_list(&recent, &sender));
        }
        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        message: StartPageInput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        let output = match message {
            StartPageInput::Open => StartPageOutput::Open,
            StartPageInput::Create | StartPageInput::AskNewProject => StartPageOutput::Create,
            StartPageInput::OpenRecent(folder) => StartPageOutput::OpenRecent(folder),
            StartPageInput::Dropped(folder) => StartPageOutput::Dropped(folder),
            StartPageInput::CloseRequested => StartPageOutput::CloseRequest,
        };
        // Nobody listening is not a mistake: the owner may not want to
        // hear.
        let _ = sender.output(output);
    }
}

/// The start page's window, behind the API the application still uses:
/// it owns the component (dropping it shuts the component down) and does
/// what people ask for with the application.
pub(super) struct StartPage {
    /// Its window.
    pub(super) window: adw::ApplicationWindow,
    /// Where its short lines show.
    pub(super) toasts: adw::ToastOverlay,
    bridge: Bridge<StartPageComponent>,
}

impl StartPage {
    /// The start page, listing the recent projects.
    pub(super) fn new(shell: &Rc<Shell>) -> Rc<StartPage> {
        let toasts = adw::ToastOverlay::new();
        let bridge = Bridge::<StartPageComponent>::launch(StartPageInit {
            application: shell.application().clone(),
            recent: shell.recent_projects(),
            toasts: toasts.clone(),
        });
        let window = bridge.widget().clone();
        let owner = Rc::downgrade(shell);
        let parent = window.downgrade();
        bridge.connect(move |output| {
            let Some(shell) = owner.upgrade() else {
                return;
            };
            match output {
                StartPageOutput::Open => {
                    if let Some(window) = parent.upgrade() {
                        choose_folder(&window, &shell);
                    }
                }
                StartPageOutput::Create => {
                    if let Some(window) = parent.upgrade() {
                        ask_new_project(&window, &shell);
                    }
                }
                StartPageOutput::OpenRecent(folder) | StartPageOutput::Dropped(folder) => {
                    shell.open_folder(&folder);
                }
                StartPageOutput::CloseRequest => shell.start_closed(),
            }
        });
        Rc::new(StartPage {
            window,
            toasts,
            bridge,
        })
    }

    /// The New Project dialog; the project it makes opens in its window.
    pub(super) fn ask_new_project(&self) {
        self.bridge.send(StartPageInput::AskNewProject);
    }
}

/// Asks for a project folder over `window` and opens it.
fn choose_folder(window: &adw::ApplicationWindow, shell: &Rc<Shell>) {
    let dialog = gtk::FileDialog::builder()
        .title("Open a project folder")
        .modal(true)
        .build();
    let shell = Rc::downgrade(shell);
    dialog.select_folder(Some(window), gio::Cancellable::NONE, move |result| {
        if let (Ok(folder), Some(shell)) = (result, shell.upgrade())
            && let Some(path) = folder.path()
        {
            shell.open_folder(&path);
        }
    });
}

/// The New Project dialog over `window`; the project it makes opens in
/// its window.
fn ask_new_project(window: &adw::ApplicationWindow, shell: &Rc<Shell>) {
    let shell = Rc::downgrade(shell);
    new_project::ask(window.upcast_ref(), move |folder: PathBuf| {
        if let Some(window) = shell.upgrade().and_then(|shell| shell.open_folder(&folder)) {
            window.welcome_new_project();
        }
    });
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::ui::gtk_test::{on_gtk_thread, settle};

    /// A start page listing `recent`, for an application of its own
    /// (named `name`: each test's has a name of its own on the bus).
    fn page(name: &str, recent: Vec<PathBuf>) -> Bridge<StartPageComponent> {
        let application = adw::Application::new(
            Some(&format!("org.stave.StartPageTest.{name}")),
            gio::ApplicationFlags::NON_UNIQUE,
        );
        // A window joins only an application that has started.
        if let Err(error) = application.register(gio::Cancellable::NONE) {
            panic!("the test's application registers: {error}");
        }
        Bridge::<StartPageComponent>::launch(StartPageInit {
            application,
            recent,
            toasts: adw::ToastOverlay::new(),
        })
    }

    /// The rows people can activate, depth first.
    fn rows(widget: &gtk::Widget) -> Vec<adw::ActionRow> {
        let mut found = Vec::new();
        let mut child = widget.first_child();
        while let Some(current) = child {
            if let Ok(row) = current.clone().downcast::<adw::ActionRow>() {
                found.push(row);
            }
            found.extend(rows(&current));
            child = current.next_sibling();
        }
        found
    }

    #[test]
    fn a_dropped_start_page_is_freed_once_closed() {
        on_gtk_thread(start_page_is_freed);
    }

    /// Closing the window and dropping the page (as the application does
    /// when it hears the page close) shuts the component down: the
    /// window, the runtime and the callback are freed.
    fn start_page_is_freed() {
        let page = page("Freed", Vec::new());
        let token = Rc::new(());
        let held = Rc::clone(&token);
        page.connect(move |_| {
            let _ = &held;
        });
        let callback = Rc::downgrade(&token);
        drop(token);
        let window = page.widget().downgrade();
        let input = page.sender().clone();
        settle();
        assert!(window.upgrade().is_some(), "the window lives while held");
        // Only a window that has shown can be closed.
        page.widget().present();
        page.widget().close();
        drop(page);
        settle();
        assert!(window.upgrade().is_none(), "the window is freed");
        assert!(
            input.send(StartPageInput::Open).is_err(),
            "the component's runtime is gone"
        );
        assert!(callback.upgrade().is_none(), "the callback is freed");
    }

    #[test]
    fn what_people_ask_for_is_queued_and_told_in_order() {
        on_gtk_thread(asks_are_queued);
    }

    /// The old page called the application from inside GTK's signals.
    /// Now each action, row and close only queues a message, and the
    /// owner hears of them in order after each update; an action fired
    /// from inside the owner's callback is queued too.
    fn asks_are_queued() {
        let shelf = PathBuf::from("/projects/wall_shelf");
        let page = Rc::new(page("Queued", vec![shelf.clone()]));
        let heard: Rc<RefCell<Vec<StartPageOutput>>> = Rc::default();
        let log = Rc::clone(&heard);
        let window = page.widget().clone();
        page.connect(move |output| {
            let first = log.borrow().is_empty();
            if first {
                // Activating an action from in here only queues it.
                let _ = WidgetExt::activate_action(&window, "win.new-project", None);
            }
            log.borrow_mut().push(output);
        });
        let Some(row) = rows(page.widget().upcast_ref()).into_iter().next() else {
            panic!("the recent project has a row");
        };
        let _ = WidgetExt::activate_action(page.widget(), "win.open", None);
        ActionRowExt::activate(&row);
        page.send(StartPageInput::AskNewProject);
        assert!(heard.borrow().is_empty(), "nothing has run yet");
        settle();
        assert_eq!(
            *heard.borrow(),
            vec![
                StartPageOutput::Open,
                StartPageOutput::OpenRecent(shelf),
                StartPageOutput::Create,
                StartPageOutput::Create,
            ],
            "in order, the action fired from the callback last"
        );
        page.widget().present();
        page.widget().close();
        settle();
        assert_eq!(heard.borrow().last(), Some(&StartPageOutput::CloseRequest));
    }
}
