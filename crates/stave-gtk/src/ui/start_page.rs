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
//! actions, rows and drop target only send [`StartPageInput`] messages.
//! It asks for a folder and shows the New Project dialog itself (its model
//! owns the dialog while it shows), and tells its owner which folder to
//! open as [`StartPageOutput`]s. [`StartPage`] is the application's
//! handle: the one owner of the component's controller, its outputs
//! forwarded at launch as [`ShellInput`]s.

use stave_app::i18n::tr;
use std::path::PathBuf;

use adw::prelude::*;
use gtk::{gio, glib};
use relm4::{Component, ComponentController, ComponentParts, ComponentSender, Controller};

use super::app::{self, APP_ID, ShellInput};
use super::new_project::{NewProjectDialog, NewProjectOutput};
use super::shown_path;

/// Asks for a project folder over the page's window; the folder chosen
/// comes back to the page as [`StartPageInput::FolderChosen`].
pub(super) type FolderChooser = fn(&adw::ApplicationWindow, relm4::Sender<StartPageInput>);

/// The start page's state: what it lists is fixed when it opens; it keeps
/// the New Project dialog while it shows, and how it asks for a folder.
pub(super) struct StartPageComponent {
    /// The New Project dialog, while it shows.
    new_project: Option<NewProjectDialog>,
    /// How it asks for a folder ([`choose_folder`], or a test's).
    choose_folder: FolderChooser,
    /// Where its short lines show: its own overlay, which only its
    /// update adds to ([`StartPageInput::Notice`]).
    toasts: adw::ToastOverlay,
}

impl std::fmt::Debug for StartPageComponent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StartPageComponent")
            .field("new_project", &self.new_project.is_some())
            .finish_non_exhaustive()
    }
}

/// What a start page is made with.
#[derive(Debug)]
pub(super) struct StartPageInit {
    /// The application its window belongs to.
    pub(super) application: adw::Application,
    /// The recent projects, newest first.
    pub(super) recent: Vec<PathBuf>,
    /// How it asks for a folder: [`choose_folder`].
    pub(super) choose_folder: FolderChooser,
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
    /// People chose a folder to open.
    FolderChosen(PathBuf),
    /// The New Project dialog wrote a project into this folder.
    NewProjectWritten(PathBuf),
    /// The New Project dialog has closed.
    NewProjectClosed,
    /// Show this short line on the page for a few seconds (the
    /// application's notices while no project window is open).
    Notice(String),
}

/// What the start page tells its owner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum StartPageOutput {
    /// Open this folder: chosen, recent or dropped on the page.
    OpenFolder(PathBuf),
    /// Open this project, just written by the New Project dialog, and
    /// welcome it.
    OpenNewProject(PathBuf),
    /// The page's window is closing.
    CloseRequest,
}

/// The recent projects, one row each; choosing one sends its folder.
fn recent_list(
    recent: &[PathBuf],
    sender: &ComponentSender<StartPageComponent>,
) -> adw::PreferencesGroup {
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
    project.append(Some(&tr("New Project…")), Some("win.new-project"));
    project.append(Some(&tr("Open Project…")), Some("win.open"));
    menu.append_section(None, &project);
    menu.append_section(Some(&tr("Style")), &app::style_section());
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
                        set_tooltip_text: Some(&tr("Main Menu")),
                        update_property: &[gtk::accessible::Property::Label(&tr("Main Menu"))],
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
                            set_description: Some(&tr(
                                "Open a project folder to see its design, or start a new project. \
                                 You can also drop a folder here.",
                            )),
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
                                            set_label: &tr("_Open Folder…"),
                                            set_use_underline: true,
                                            set_action_name: Some("win.open"),
                                            // After the label, which adds
                                            // GTK's `text-button`: these
                                            // replace it.
                                            set_css_classes: &["pill", "suggested-action"],
                                        },
                                        gtk::Button {
                                            set_label: &tr("_New Project…"),
                                            set_use_underline: true,
                                            set_action_name: Some("win.new-project"),
                                            set_css_classes: &["pill"],
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
            choose_folder,
        } = init;
        let toasts = adw::ToastOverlay::new();
        let toasts = &toasts;
        let has_icon = gtk::gdk::Display::default()
            .is_some_and(|display| gtk::IconTheme::for_display(&display).has_icon(APP_ID));
        let icon = if has_icon {
            APP_ID
        } else {
            "folder-open-symbolic"
        };
        let dropped = sender.input_sender().clone();
        let model = StartPageComponent {
            new_project: None,
            choose_folder,
            toasts: toasts.clone(),
        };
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
        root: &Self::Root,
    ) {
        let output = match message {
            StartPageInput::Open => {
                (self.choose_folder)(root, sender.input_sender().clone());
                None
            }
            StartPageInput::Create | StartPageInput::AskNewProject => {
                // One at a time: the dialog showing is the one people
                // answer.
                if self.new_project.is_none() {
                    let dialog =
                        NewProjectDialog::new(sender.input_sender(), |output| match output {
                            NewProjectOutput::Created(folder) => {
                                StartPageInput::NewProjectWritten(folder)
                            }
                            NewProjectOutput::Closed => StartPageInput::NewProjectClosed,
                        });
                    dialog.present(root);
                    self.new_project = Some(dialog);
                }
                None
            }
            StartPageInput::NewProjectClosed => {
                self.new_project = None;
                None
            }
            StartPageInput::NewProjectWritten(folder) => {
                Some(StartPageOutput::OpenNewProject(folder))
            }
            StartPageInput::OpenRecent(folder)
            | StartPageInput::Dropped(folder)
            | StartPageInput::FolderChosen(folder) => Some(StartPageOutput::OpenFolder(folder)),
            StartPageInput::CloseRequested => Some(StartPageOutput::CloseRequest),
            StartPageInput::Notice(text) => {
                self.toasts
                    .add_toast(adw::Toast::builder().title(text).timeout(5).build());
                None
            }
        };
        if let Some(output) = output {
            // Nobody listening is not a mistake: the owner may not want
            // to hear.
            let _ = sender.output(output);
        }
    }
}

/// The start page's window as the application holds it: the one owner
/// of the component's controller (dropping it shuts the component down),
/// its outputs forwarded at launch as [`ShellInput`]s.
pub(super) struct StartPage {
    /// Its window.
    pub(super) window: adw::ApplicationWindow,
    controller: Controller<StartPageComponent>,
}

impl StartPage {
    /// The start page, listing the recent projects; what people ask for
    /// goes to the application as [`ShellInput`]s.
    pub(super) fn new(
        application: &adw::Application,
        recent: Vec<PathBuf>,
        shell: relm4::Sender<ShellInput>,
    ) -> StartPage {
        let controller = StartPageComponent::builder()
            .launch(StartPageInit {
                application: application.clone(),
                recent,
                choose_folder,
            })
            .forward(&shell, |output| match output {
                StartPageOutput::OpenFolder(folder) => ShellInput::OpenFolder(folder),
                StartPageOutput::OpenNewProject(folder) => ShellInput::OpenNewProject(folder),
                StartPageOutput::CloseRequest => ShellInput::StartClosed,
            });
        StartPage {
            window: controller.widget().clone(),
            controller,
        }
    }

    /// The New Project dialog; the project it makes opens in its window.
    pub(super) fn ask_new_project(&self) {
        let _ = self.controller.sender().send(StartPageInput::AskNewProject);
    }

    /// Shows a short line on the page for a few seconds.
    pub(super) fn notice(&self, text: &str) {
        let _ = self
            .controller
            .sender()
            .send(StartPageInput::Notice(text.to_owned()));
    }
}

/// Asks for a project folder over `window`; the folder chosen goes to the
/// page as [`StartPageInput::FolderChosen`].
fn choose_folder(window: &adw::ApplicationWindow, page: relm4::Sender<StartPageInput>) {
    let dialog = gtk::FileDialog::builder()
        .title(tr("Open a project folder"))
        .modal(true)
        .build();
    dialog.select_folder(Some(window), gio::Cancellable::NONE, move |result| {
        if let Ok(folder) = result
            && let Some(path) = folder.path()
        {
            // The page may have gone; then nobody needs to know.
            let _ = page.send(StartPageInput::FolderChosen(path));
        }
    });
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;
    use crate::ui::gtk_test::{listener, on_gtk_thread, settle};

    /// The folder a test's page is told was chosen, as the file chooser
    /// would tell it.
    const CHOSEN: &str = "/projects/chosen";

    /// A folder chooser that answers at once with [`CHOSEN`], so no real
    /// chooser opens in a test.
    fn choose_at_once(_: &adw::ApplicationWindow, page: relm4::Sender<StartPageInput>) {
        let _ = page.send(StartPageInput::FolderChosen(PathBuf::from(CHOSEN)));
    }

    /// A start page listing `recent`, for an application of its own
    /// (named `name`: each test's has a name of its own on the bus), its
    /// outputs going to `to`.
    fn page(
        name: &str,
        recent: Vec<PathBuf>,
        to: &relm4::Sender<StartPageOutput>,
    ) -> Controller<StartPageComponent> {
        let application = adw::Application::new(
            Some(&format!("org.stave.StartPageTest.{name}")),
            gio::ApplicationFlags::NON_UNIQUE,
        );
        // A window joins only an application that has started.
        if let Err(error) = application.register(gio::Cancellable::NONE) {
            panic!("the test's application registers: {error}");
        }
        StartPageComponent::builder()
            .launch(StartPageInit {
                application,
                recent,
                choose_folder: choose_at_once,
            })
            .forward(to, |output| output)
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
        let token = Rc::new(());
        let held = Rc::clone(&token);
        let to = listener(move |_: StartPageOutput| {
            let _ = &held;
        });
        let page = page("Freed", Vec::new(), &to);
        drop(to);
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
    fn a_notice_is_shown_by_the_page_itself() {
        on_gtk_thread(notice_is_the_pages);
    }

    /// The application used to add its toast to the page's overlay
    /// itself; now it tells the page, which shows it from its own update
    /// and tells nobody anything.
    fn notice_is_the_pages() {
        let heard: Rc<RefCell<Vec<StartPageOutput>>> = Rc::default();
        let log = Rc::clone(&heard);
        let to = listener(move |output| log.borrow_mut().push(output));
        let page = page("Notice", Vec::new(), &to);
        page.widget().present();
        settle();
        let toast_labels = |page: &Controller<StartPageComponent>| {
            labels_in(page.model().toasts.upcast_ref())
                .into_iter()
                .filter(|text| text == "Couldn't open that folder")
                .count()
        };
        assert_eq!(toast_labels(&page), 0);
        let _ = page.sender().send(StartPageInput::Notice(
            "Couldn't open that folder".to_owned(),
        ));
        assert_eq!(toast_labels(&page), 0, "only queued");
        settle();
        assert_eq!(toast_labels(&page), 1, "the page shows it");
        assert!(heard.borrow().is_empty(), "and tells nobody");
        page.widget().close();
        settle();
    }

    /// Every label's text under `widget`, depth first.
    fn labels_in(widget: &gtk::Widget) -> Vec<String> {
        let mut found = Vec::new();
        let mut child = widget.first_child();
        while let Some(current) = child {
            if let Some(label) = current.downcast_ref::<gtk::Label>() {
                found.push(label.label().to_string());
            }
            found.extend(labels_in(&current));
            child = current.next_sibling();
        }
        found
    }

    #[test]
    fn what_people_ask_for_is_queued_and_told_in_order() {
        on_gtk_thread(asks_are_queued);
    }

    /// The old page called the application from inside GTK's signals.
    /// Now each action, row and close only queues a message, and the
    /// owner hears of them in order after each update; an action fired
    /// from inside the owner's handler is queued too. Open asks for a
    /// folder (here answered at once) and the folder chosen is told;
    /// New Project shows the dialog, which the page keeps while it shows
    /// and lets go once it has closed.
    fn asks_are_queued() {
        let shelf = PathBuf::from("/projects/wall_shelf");
        let dropped = PathBuf::from("/projects/hall_table");
        let window_slot: Rc<RefCell<Option<adw::ApplicationWindow>>> = Rc::default();
        let heard: Rc<RefCell<Vec<StartPageOutput>>> = Rc::default();
        let (log, window_of) = (Rc::clone(&heard), Rc::clone(&window_slot));
        let to = listener(move |output| {
            let first = log.borrow().is_empty();
            if first && let Some(window) = window_of.borrow().as_ref() {
                // Activating an action from in here only queues it.
                let _ = WidgetExt::activate_action(window, "win.open", None);
            }
            log.borrow_mut().push(output);
        });
        let page = page("Queued", vec![shelf.clone()], &to);
        window_slot.replace(Some(page.widget().clone()));
        let Some(row) = rows(page.widget().upcast_ref()).into_iter().next() else {
            panic!("the recent project has a row");
        };
        ActionRowExt::activate(&row);
        let _ = page.sender().send(StartPageInput::Dropped(dropped.clone()));
        assert!(heard.borrow().is_empty(), "nothing has run yet");
        settle();
        assert_eq!(
            *heard.borrow(),
            vec![
                StartPageOutput::OpenFolder(shelf),
                StartPageOutput::OpenFolder(dropped),
                StartPageOutput::OpenFolder(PathBuf::from(CHOSEN)),
            ],
            "in order, the folder chosen from the handler's action last"
        );

        page.widget().present();
        let _ = WidgetExt::activate_action(page.widget(), "win.new-project", None);
        let _ = page.sender().send(StartPageInput::AskNewProject);
        settle();
        let Some(dialog) = page.widget().visible_dialog() else {
            panic!("New Project shows its dialog over the page");
        };
        assert!(page.model().new_project.is_some(), "the page keeps it");
        let shown = dialog.downgrade();
        drop(dialog);
        if let Some(dialog) = shown.upgrade() {
            dialog.force_close();
        }
        settle();
        assert!(page.model().new_project.is_none(), "let go once closed");
        assert!(shown.upgrade().is_none(), "and freed");

        page.widget().close();
        settle();
        assert_eq!(heard.borrow().last(), Some(&StartPageOutput::CloseRequest));
    }
}
