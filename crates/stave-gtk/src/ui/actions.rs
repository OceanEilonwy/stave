//! The window's actions (`win.*`, the menus' and keys' targets) and
//! opening another project folder from them; the main menu.

use stave_app::i18n::{fill_in, ntr, tr, tr_noop};
use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};
use stave_app::inspect;
use stave_app::session::{Command, Editor, ProblemIndex, Selection, Session, View};

use super::{ActionHandler, Window, app, header, workbench};

impl Window {
    pub(super) fn install_actions(self: &Rc<Self>) {
        let window = &self.widgets.window;
        // The window's own action holds it weakly: a closed window is freed.
        let add = |name: &str, handler: ActionHandler| {
            let action = gio::SimpleAction::new(name, None);
            let this = Rc::downgrade(self);
            action.connect_activate(move |_, _| {
                if let Some(this) = this.upgrade() {
                    handler(&this);
                }
            });
            window.add_action(&action);
        };
        add("new-project", Box::new(move |this| this.ask_new_project()));
        add("back", Box::new(move |this| this.dispatch(Command::Back)));
        add(
            "forward",
            Box::new(move |this| this.dispatch(Command::Forward)),
        );
        add("check", Box::new(move |this| this.dispatch(Command::Check)));
        add("frame", Box::new(move |this| this.frame()));
        add("export-view", Box::new(move |this| this.export_view()));
        add(
            "next-problem",
            Box::new(move |this| this.dispatch(Command::NextProblem)),
        );
        add(
            "previous-problem",
            Box::new(move |this| this.dispatch(Command::PreviousProblem)),
        );
        add(
            "find",
            Box::new(move |this| this.widgets.finder.present(&this.widgets.window, "")),
        );
        add(
            "search",
            Box::new(move |this| {
                // In the text, its own search bar; anywhere else, the
                // finder.
                if !this.widgets.files.focus_search() {
                    this.widgets.finder.present(&this.widgets.window, "");
                }
            }),
        );
        add(
            "editor",
            // Shows the Code panel wherever it is docked, with the keyboard
            // in the text.
            Box::new(move |this| {
                if this.session.borrow().editor() == Editor::Open {
                    this.widgets.workbench.present(workbench::PanelId::Code);
                    this.widgets.files.focus_text();
                } else {
                    this.dispatch(Command::ShowEditor(Editor::Open));
                }
            }),
        );
        let show_panel = gio::SimpleAction::new("show-panel", Some(glib::VariantTy::STRING));
        let this = Rc::downgrade(self);
        show_panel.connect_activate(move |_, word| {
            let Some(this) = this.upgrade() else {
                return;
            };
            let id = word
                .and_then(glib::Variant::str)
                .and_then(workbench::PanelId::from_word);
            match id {
                Some(workbench::PanelId::Code) => {
                    let _ = WidgetExt::activate_action(&this.widgets.window, "win.editor", None);
                }
                Some(id) => this.widgets.workbench.present(id),
                None => {}
            }
        });
        self.widgets.window.add_action(&show_panel);
        // "code:end": move the Code panel to the right edge.
        let move_panel = gio::SimpleAction::new("move-panel", Some(glib::VariantTy::STRING));
        let this = Rc::downgrade(self);
        move_panel.connect_activate(move |_, target| {
            let Some(this) = this.upgrade() else {
                return;
            };
            let Some((id, side)) = target
                .and_then(glib::Variant::str)
                .and_then(|t| t.split_once(':'))
            else {
                return;
            };
            if let (Some(id), Some(side)) = (
                workbench::PanelId::from_word(id),
                workbench::Side::from_word(side),
            ) {
                this.widgets.workbench.move_to(id, side);
            }
        });
        self.widgets.window.add_action(&move_panel);
        add(
            "reset-layout",
            Box::new(move |this| this.widgets.workbench.reset()),
        );
        add(
            "model",
            Box::new(move |this| this.widgets.workbench.present(workbench::PanelId::Parts)),
        );
        add("save", Box::new(move |this| this.widgets.files.save()));
        add("format", Box::new(move |this| this.widgets.files.format()));
        add("open", Box::new(move |this| this.choose_folder()));
        for view in View::ALL.into_iter().filter(|view| header::ready(*view)) {
            add(
                &format!("view-{}", view.word()),
                Box::new(move |this| this.dispatch(Command::ShowView(view))),
            );
        }
        add(
            "escape",
            Box::new(move |this| {
                if this.cancel_edit() {
                    return;
                }
                let focused = this.session.borrow().focus().is_some();
                if focused {
                    this.dispatch(Command::LeaveFocus);
                } else {
                    this.dispatch(Command::Select(None));
                }
            }),
        );
        // The chip at the view's edge names a problem off screen.
        let off_screen = gio::SimpleAction::new("show-problem", Some(glib::VariantTy::UINT64));
        let this = Rc::downgrade(self);
        off_screen.connect_activate(move |_, parameter| {
            let Some(this) = this.upgrade() else {
                return;
            };
            let Some(index) = parameter
                .and_then(|p| p.get::<u64>())
                .and_then(|index| usize::try_from(index).ok())
            else {
                return;
            };
            this.dispatch(Command::Select(Some(Selection::Problem(ProblemIndex(
                index,
            )))));
        });
        window.add_action(&off_screen);
        // Open Recent: a folder named in the menu.
        let recent = gio::SimpleAction::new("open-recent", Some(glib::VariantTy::STRING));
        let this = Rc::downgrade(self);
        recent.connect_activate(move |_, parameter| {
            let Some(this) = this.upgrade() else {
                return;
            };
            if let Some(path) = parameter.and_then(glib::Variant::str) {
                this.go_to_folder(PathBuf::from(path));
            }
        });
        window.add_action(&recent);
        // A folder dragged in from Files.
        let this = Rc::downgrade(self);
        window.add_controller(app::folder_drop(move |path| {
            if let Some(this) = this.upgrade() {
                this.go_to_folder(path);
            }
        }));
    }

    /// Opens another project folder.
    pub(super) fn choose_folder(self: &Rc<Self>) {
        let dialog = gtk::FileDialog::builder()
            .title(tr("Open a project folder"))
            .modal(true)
            .build();
        let window = self.clone();
        dialog.select_folder(
            Some(&self.widgets.window),
            gio::Cancellable::NONE,
            move |result| {
                let Ok(folder) = result else { return };
                let Some(path) = folder.path() else { return };
                window.go_to_folder(path);
            },
        );
    }

    /// Shows a project folder: brings forward the window already showing
    /// it, or opens it in this one once unsaved changes are dealt with. A
    /// `.stave` file opens its folder.
    pub(super) fn go_to_folder(self: &Rc<Self>, path: PathBuf) {
        let folder = match app::project_folder(&path) {
            Ok(folder) => folder,
            Err(error) => {
                self.widgets.toasts.add_toast(
                    adw::Toast::builder()
                        .title(format!("{error:#}"))
                        .timeout(5)
                        .build(),
                );
                return;
            }
        };
        if let Some(other) = self.shell.window_showing(&folder) {
            other.widgets.window.present();
            return;
        }
        let next = self.clone();
        self.ask_about_unsaved(move || next.open_folder(folder));
    }

    /// Asks what to do with unsaved changes (save, throw away or stay),
    /// then goes on, unless people chose to stay.
    pub(super) fn ask_about_unsaved(self: &Rc<Self>, then: impl FnOnce() + 'static) {
        let files = &self.widgets.files;
        let count = files.unsaved_count();
        if count == 0 {
            then();
            return;
        }
        let dialog = adw::AlertDialog::builder()
            .heading(tr("Save Changes?"))
            .body(fill_in(
                &ntr(
                    "One file has changes that aren't saved. If you don't save them, they are lost.",
                    "{count} files have changes that aren't saved. If you don't save them, they are lost.",
                    count,
                ),
                &[("count", &count)],
            ))
            .close_response("cancel")
            .default_response("save")
            .build();
        dialog.add_responses(&[
            ("cancel", tr("_Cancel").as_str()),
            ("discard", tr("_Don't Save").as_str()),
            ("save", tr("_Save").as_str()),
        ]);
        dialog.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
        dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);
        let window = self.clone();
        let then = Cell::new(Some(then));
        dialog.connect_response(None, move |_, response| {
            let go_on = match response {
                "save" => window.widgets.files.save_all(),
                "discard" => true,
                _ => false,
            };
            if go_on && let Some(then) = then.take() {
                then();
            }
        });
        dialog.present(Some(&self.widgets.window));
    }

    pub(super) fn open_folder(self: &Rc<Self>, folder: PathBuf) {
        self.shell.record_recent(&folder);
        *self.session.borrow_mut() = Session::new(folder);
        self.editing_folder_changed();
        {
            let mut state = self.state.borrow_mut();
            state.uploaded = None;
            state.posed = None;
            state.orbit = None;
            state.focus = None;
            state.problem = None;
            state.opening = None;
        }
        self.watch();
        self.show_folder();
        self.widgets.tree.show_tree(
            inspect::PartsTree::default(),
            stave_app::part_joints::PartJoints::default(),
        );
        self.show_status(&tr("Checking…"), "");
        self.dispatch(Command::Check);
        self.view_changed();
        self.selection_changed();
    }
}

/// Closes the window after a screenshot and stops the application, so
/// nothing left open (a dialog) keeps it running.
pub(super) fn close_for_good(window: &adw::ApplicationWindow) {
    let application = window.application();
    window.close();
    if let Some(application) = application {
        application.quit();
    }
}

/// The title bar's menus: Project, Edit, View and Help, each a submenu
/// (the menu bar shows them side by side; a narrow window's one menu
/// button lists them).
pub(super) fn main_menu(recent: &gio::Menu) -> gio::Menu {
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
