//! The project: its folder, checking, the editor's files, saving and the
//! command line's wishes, closing, and the window's actions for them.

use super::{Unprinted, add_action};
use crate::ui::files::CursorName;
// The window's own items and imports, as the handlers had them in mod.rs.
use crate::ui::*;
use stave_ide::{Analysis, SymbolId};

/// What the project group is told.
#[derive(Debug)]
pub(in crate::ui) enum ProjectInput {
    /// Sends this command to the session.
    Dispatch(Command),
    /// A check finished on its worker thread.
    Checked {
        /// The check it answers.
        ticket: stave_app::session::CheckTicket,
        /// What it found.
        outcome: check::Outcome,
    },
    /// The New Project action: ask for a new project.
    NewProject,
    /// The New Project dialog wrote a project into this folder.
    NewProjectWritten(PathBuf),
    /// Opens the project just written in this folder, and welcomes it.
    OpenNewProject(PathBuf),
    /// A project just made: its envelope lit as the first space to click.
    Welcome,
    /// The Open action: ask for a project folder.
    ChooseFolder,
    /// A folder chosen, dropped on the window or picked from Open Recent.
    GoToFolder(PathBuf),
    /// No other window shows this folder: open it here, once unsaved
    /// changes are dealt with.
    OpenHere(PathBuf),
    /// Opens this folder here, now.
    OpenFolder(PathBuf),
    /// The Save action.
    Save,
    /// The Tidy action.
    Format,
    /// The Editor action: the Code panel, with the keyboard in the text.
    ShowEditor,
    /// The Search action: the text's own search, or the finder.
    Search,
    /// Shows a panel wherever it is docked.
    ShowPanel(workbench::PanelId),
    /// Moves a panel to an edge.
    MovePanel {
        /// The panel.
        panel: workbench::PanelId,
        /// The edge.
        side: workbench::Side,
    },
    /// Lays the panels out as they first were.
    ResetLayout,
    /// The editor's cursor rests on a name.
    TextLinked(Unprinted<CursorName>),
    /// The editor goes to a place: where the cursor was becomes the place
    /// Back returns to.
    Go {
        /// Where the cursor was.
        from: Option<SourceLocation>,
        /// Where it goes.
        to: SourceLocation,
    },
    /// The editor asks to show a name in the 3D view.
    ShowIn3d {
        /// The analysis the name is in.
        analysis: Unprinted<Rc<Analysis>>,
        /// The name.
        symbol: SymbolId,
    },
    /// A short line at the bottom of the window, for this many seconds.
    Notice {
        /// What it says.
        text: String,
        /// How long it stays.
        seconds: u32,
    },
    /// Closing found unsaved changes: ask about them first.
    CloseAsked,
    /// The unsaved changes are dealt with: close.
    CloseNow,
}

impl From<ProjectInput> for WindowInput {
    fn from(message: ProjectInput) -> WindowInput {
        WindowInput::Project(message)
    }
}

/// Wires the project's actions, the editor's outputs and closing to
/// [`ProjectInput`]s.
pub(in crate::ui) fn wire(window: &Window) {
    let widgets = &window.widgets;
    let root = &widgets.window;
    let input = window.input();
    add_action(root, "new-project", &input, || ProjectInput::NewProject);
    add_action(root, "check", &input, || {
        ProjectInput::Dispatch(Command::Check)
    });
    add_action(root, "search", &input, || ProjectInput::Search);
    add_action(root, "editor", &input, || ProjectInput::ShowEditor);
    add_action(root, "reset-layout", &input, || ProjectInput::ResetLayout);
    add_action(root, "model", &input, || {
        ProjectInput::ShowPanel(workbench::PanelId::Parts)
    });
    add_action(root, "save", &input, || ProjectInput::Save);
    add_action(root, "format", &input, || ProjectInput::Format);
    add_action(root, "open", &input, || ProjectInput::ChooseFolder);
    let show_panel = gio::SimpleAction::new("show-panel", Some(glib::VariantTy::STRING));
    let sender = input.clone();
    show_panel.connect_activate(move |_, word| {
        if let Some(id) = word
            .and_then(glib::Variant::str)
            .and_then(workbench::PanelId::from_word)
        {
            sender.emit(ProjectInput::ShowPanel(id).into());
        }
    });
    root.add_action(&show_panel);
    // "code:end": move the Code panel to the right edge.
    let move_panel = gio::SimpleAction::new("move-panel", Some(glib::VariantTy::STRING));
    let sender = input.clone();
    move_panel.connect_activate(move |_, target| {
        let Some((id, side)) = target
            .and_then(glib::Variant::str)
            .and_then(|t| t.split_once(':'))
        else {
            return;
        };
        if let (Some(panel), Some(side)) = (
            workbench::PanelId::from_word(id),
            workbench::Side::from_word(side),
        ) {
            sender.emit(ProjectInput::MovePanel { panel, side }.into());
        }
    });
    root.add_action(&move_panel);
    // Open Recent: a folder named in the menu.
    let recent = gio::SimpleAction::new("open-recent", Some(glib::VariantTy::STRING));
    let sender = input.clone();
    recent.connect_activate(move |_, parameter| {
        if let Some(path) = parameter.and_then(glib::Variant::str) {
            sender.emit(ProjectInput::GoToFolder(PathBuf::from(path)).into());
        }
    });
    root.add_action(&recent);
    // A folder dragged in from Files.
    let sender = input.clone();
    root.add_controller(app::folder_drop(move |path| {
        sender.emit(ProjectInput::GoToFolder(path).into());
    }));

    // The editor tells the window where the cursor is, where to go, when
    // typing has paused and when a tab on the rail was chosen.
    let files = &widgets.files;
    let sender = input.clone();
    files.connect_link(move |found| {
        sender.emit(ProjectInput::TextLinked(Unprinted(found)).into());
    });
    let sender = input.clone();
    files.connect_go(move |from, to| sender.emit(ProjectInput::Go { from, to }.into()));
    let sender = input.clone();
    files.connect_check(move || sender.emit(ProjectInput::Dispatch(Command::Check).into()));
    let sender = input.clone();
    files.connect_show_in_3d(move |analysis, symbol| {
        sender.emit(
            ProjectInput::ShowIn3d {
                analysis: Unprinted(analysis),
                symbol,
            }
            .into(),
        );
    });
    let sender = input.clone();
    files.connect_open(move || {
        sender.emit(ProjectInput::Dispatch(Command::ShowEditor(Editor::Open)).into());
    });
    // The open files as tabs beside the Model, wherever the Code panel is
    // docked.
    if let Some(code) = widgets.workbench.panel(workbench::PanelId::Code) {
        let page = Rc::downgrade(files);
        tab_strip::set_page_tabs(code, move || {
            page.upgrade()
                .map(|page| page.page_tabs())
                .unwrap_or_default()
        });
    }
    let sender = input.clone();
    files.connect_notice(move |text| {
        sender.emit(ProjectInput::Notice { text, seconds: 4 }.into());
    });
    let sender = input.clone();
    widgets.workbench.connect_save_failed(move |text| {
        sender.emit(
            ProjectInput::Notice {
                text: text.to_owned(),
                seconds: 5,
            }
            .into(),
        );
    });
    // The Details panel asks whether a file has unsaved changes.
    let files = Rc::clone(&widgets.files);
    widgets
        .inspector
        .connect_unsaved(move |path| files.is_unsaved(Path::new(path)));

    // Closing the window remembers its size and the layout, and asks
    // about unsaved changes first.
    let files = Rc::clone(&widgets.files);
    let workbench = Rc::clone(&widgets.workbench);
    let output = window.sender.output_sender().clone();
    let sender = input;
    root.connect_close_request(move |root| {
        if let Some(application) = root.application() {
            output.emit(WindowOutput::Closing {
                size: root.default_size(),
                maximized: root.is_maximized(),
                hold: application.hold(),
            });
        }
        workbench.save();
        if !files.has_unsaved() {
            return glib::Propagation::Proceed;
        }
        sender.emit(ProjectInput::CloseAsked.into());
        glib::Propagation::Stop
    });
}

impl Window {
    /// Handles a [`ProjectInput`].
    pub(in crate::ui) fn update_project(&mut self, message: ProjectInput) {
        match message {
            ProjectInput::Dispatch(command) => self.dispatch(command),
            ProjectInput::Checked { ticket, outcome } => {
                let events = self.session.finish_check(ticket, outcome);
                self.handle(events);
            }
            ProjectInput::NewProject => self.ask_new_project(),
            ProjectInput::NewProjectWritten(folder) => {
                self.ask_about_unsaved(ProjectInput::OpenNewProject(folder));
            }
            ProjectInput::OpenNewProject(folder) => {
                self.open_folder(folder);
                self.welcome_new_project();
            }
            ProjectInput::Welcome => self.welcome_new_project(),
            ProjectInput::ChooseFolder => self.choose_folder(),
            ProjectInput::GoToFolder(path) => self.go_to_folder(path),
            ProjectInput::OpenHere(folder) => {
                self.ask_about_unsaved(ProjectInput::OpenFolder(folder));
            }
            ProjectInput::OpenFolder(folder) => self.open_folder(folder),
            ProjectInput::Save => self.widgets.files.save(),
            ProjectInput::Format => self.widgets.files.format(),
            ProjectInput::ShowEditor => {
                if self.session.editor() == Editor::Open {
                    self.widgets.workbench.present(workbench::PanelId::Code);
                    self.widgets.files.focus_text();
                } else {
                    self.dispatch(Command::ShowEditor(Editor::Open));
                }
            }
            ProjectInput::Search => {
                // In the text, its own search bar; anywhere else, the
                // finder.
                if !self.widgets.files.focus_search() {
                    self.widgets.finder.present(&self.widgets.window, "");
                }
            }
            ProjectInput::ShowPanel(workbench::PanelId::Code) => {
                self.update_project(ProjectInput::ShowEditor);
            }
            ProjectInput::ShowPanel(id) => self.widgets.workbench.present(id),
            ProjectInput::MovePanel { panel, side } => {
                self.widgets.workbench.move_to(panel, side);
            }
            ProjectInput::ResetLayout => self.widgets.workbench.reset(),
            ProjectInput::TextLinked(Unprinted(found)) => self.text_linked(found),
            ProjectInput::Go { from, to } => {
                if let Some(from) = from {
                    self.dispatch(Command::Link(Some(Selection::Location(from))));
                }
                self.dispatch(Command::GoToSource(to));
            }
            ProjectInput::ShowIn3d {
                analysis: Unprinted(analysis),
                symbol,
            } => {
                let session = &self.session;
                let selection = session.drawable().and_then(|(design, _)| {
                    link::selection_for(design, &analysis, symbol, session.selection())
                });
                match selection {
                    Some(selection) => self.select_and_frame(Some(selection)),
                    None => self.toast("It isn't in the design the 3D view shows"),
                }
            }
            ProjectInput::Notice { text, seconds } => {
                self.widgets
                    .toasts
                    .add_toast(adw::Toast::builder().title(text).timeout(seconds).build());
            }
            ProjectInput::CloseAsked => self.ask_about_unsaved(ProjectInput::CloseNow),
            ProjectInput::CloseNow => {
                self.widgets.files.forget_unsaved();
                self.widgets.window.close();
            }
        }
    }

    /// Runs a check on a worker thread; its outcome comes back as
    /// [`ProjectInput::Checked`].
    pub(in crate::ui) fn run_check(&mut self, ticket: stave_app::session::CheckTicket) {
        if self.state.hold_checks {
            return;
        }
        let folder = self.session.folder().to_path_buf();
        // The project as the editor shows it, unsaved changes included.
        let overlay = self.widgets.files.overlay_for_check();
        let input = self.input();
        crate::render::profile::edit_started();
        glib::spawn_future_local(async move {
            let outcome = gio::spawn_blocking(move || check::run_with(&folder, &overlay))
                .await
                .unwrap_or_else(|_| check::Outcome::CantOpen {
                    message: "checking the design crashed".to_owned(),
                    elapsed: Duration::ZERO,
                });
            crate::render::profile::edit_checked();
            // The window may have gone; then nobody needs to know.
            let _ = input.send(ProjectInput::Checked { ticket, outcome }.into());
        });
    }

    /// The folder in the title until the design is in.
    pub(in crate::ui) fn show_folder(&mut self) {
        let folder = self.session.folder().to_path_buf();
        self.show_title();
        self.widgets.files.set_folder(&folder);
        self.widgets.file_list.set_folder(&folder);
    }

    /// The project's or library's own title (the folder's name until one
    /// is read), with the folder under it.
    pub(in crate::ui) fn show_title(&self) {
        let (title, folder) = {
            let session = &self.session;
            (session.title(), session.folder().to_path_buf())
        };
        self.widgets.header.show_title(&title, &shown_path(&folder));
        self.widgets
            .window
            .set_title(Some(&format!("{title} — Stave")));
    }

    pub(in crate::ui) fn result_changed(&mut self) {
        self.grips_result_changed();
        let (shown, latest, showing_older) = {
            let session = &self.session;
            (
                session.shown().cloned(),
                session.latest().cloned(),
                session.showing_older(),
            )
        };
        self.show_title();
        self.widgets.files.show_result(latest.clone());
        {
            let session = &self.session;
            let selected = match session.selection() {
                Some(Selection::Problem(index)) => Some(index.0),
                _ => None,
            };
            let reports = latest
                .as_deref()
                .map(stave_app::problems::reports)
                .unwrap_or_default();
            self.widgets
                .problems
                .show(&reports, selected, showing_older);
        }
        // The dots: by file always; by part only when the latest result is
        // the one shown (its ids are the tree's).
        // Read-only from here on, so the three holders share one copy.
        let marks = Rc::new(
            latest
                .as_deref()
                .map(stave_app::problems::marks)
                .unwrap_or_default(),
        );
        self.widgets.files.show_marks(Rc::clone(&marks));
        let tree_marks = if showing_older { Rc::default() } else { marks };
        self.state.marks = Rc::clone(&tree_marks);
        self.widgets.tree.show_marks(tree_marks);
        let empty = edit_view::empty_state(&self.session);
        if let Some(empty) = empty {
            // The sidebar and the inspector say why too (for a library,
            // what it defines and how to use it), not "Nothing selected".
            self.widgets.inspector.show_idle(Some(empty.inspector()));
            self.show_empty(&empty.title, &empty.description, empty.offers_open);
            self.widgets.tree.show_empty(empty);
            self.show_card();
            // What doesn't need the 3D view (the editor) still opens as
            // asked.
            self.apply_wanted();
            return;
        }
        self.widgets.inspector.show_idle(None);
        let Some(shown) = shown else {
            return;
        };
        // An older design on show is said by the red problems chip and a
        // line over the Problems panel's list, not a banner.
        let Some((design, graph)) = shown.drawable() else {
            return;
        };
        self.widgets.tree.show_tree(
            inspect::parts_tree(design),
            stave_app::part_joints::PartJoints::new(design),
        );
        let first = {
            let state = &mut self.state;
            let fresh = !state
                .uploaded
                .as_ref()
                .is_some_and(|uploaded| Rc::ptr_eq(uploaded, &shown));
            if fresh {
                if let Ok(renderer) = &mut state.renderer {
                    renderer.upload(graph);
                }
                state.uploaded = Some(shown.clone());
                // A new result renumbers everything: focus and the posed
                // problem are opened again from the session.
                state.focus = None;
                state.problem = None;
                state.opening = None;
            }
            state.posed.is_none()
        };
        let titles: Vec<String> = graph.scenes.iter().map(|s| s.title.clone()).collect();
        let index = self.session.scene_index();
        self.widgets.header.show_scenes(&titles, index);
        self.widgets.view_stack.set_visible_child_name("view");
        self.show_moves();
        self.repose();
        if first {
            let state = &mut self.state;
            state.orbit = Self::frame_all(state, &shown);
        }
        self.apply_wanted();
        self.apply_start_camera();
        // Whatever the session carried over, shown again; the editor stays
        // where people are typing.
        self.state.linking = true;
        self.selection_changed();
        self.state.linking = false;
        self.focus_changed();
        self.request_render();
        // The 3D view draws (and saves a screenshot) only when it shows.
        if !self.shows_3d() {
            self.capture_if_asked();
        }
    }

    /// Applies the command line's wishes once, after the first result.
    pub(in crate::ui) fn apply_wanted(&mut self) {
        let Some(wanted) = self.state.wanted.take() else {
            return;
        };
        let found = {
            let session = &self.session;
            session.drawable().map(|(design, graph)| {
                let select = wanted
                    .selection
                    .as_ref()
                    .and_then(|name| link::selection_named(design, name));
                let focus = wanted
                    .focus
                    .as_ref()
                    .and_then(|name| focus::find(design, name, None).ok());
                let scene = wanted
                    .scene
                    .as_ref()
                    .filter(|name| graph.scenes.iter().any(|s| &s.name == *name))
                    .cloned();
                (select, focus, scene)
            })
        };
        let (select, target, scene) = found.unwrap_or_default();
        let plan = if wanted.plan {
            Some(true)
        } else if wanted.in_3d {
            Some(false)
        } else {
            None
        };
        if let Some(scene) = scene {
            self.dispatch(Command::ShowScene(scene));
        }
        if let Some(explode) = wanted.explode {
            self.dispatch(Command::SetExplode(Explode::clamped(explode)));
        }
        if let Some(selection) = select {
            self.select_and_frame(Some(selection));
        }
        if let Some(target) = target {
            self.state.focus_start = Some((plan, wanted.step));
            self.dispatch(Command::Focus(target));
            self.state.focus_start = None;
        }
        if let Some(index) = wanted.problem {
            self.dispatch(Command::Select(Some(Selection::Problem(ProblemIndex(
                index,
            )))));
        }
        if wanted.view != View::Design {
            self.dispatch(Command::ShowView(wanted.view));
        }
        if wanted.xray {
            self.widgets.view_toolbar.xray.set_active(true);
        }
        if wanted.editor && wanted.file.is_none() {
            self.dispatch(Command::ShowEditor(Editor::Open));
        }
        if let Some(file) = &wanted.file {
            let location = SourceLocation::parse(self.session.folder(), file);
            if let Some(location) = location {
                self.dispatch(Command::GoToSource(location));
            }
        }
        if wanted.wish != Wish::default() {
            self.widgets.files.wish(wanted.wish);
        }
        if wanted.checking || wanted.building {
            self.state.hold_checks = true;
            self.dispatch(if wanted.building {
                Command::Build
            } else {
                Command::Check
            });
        }
        if let Some(query) = &wanted.finder {
            self.widgets
                .finder
                .present(&self.widgets.window, query.as_str());
        }
    }

    /// The editor opened or closed. Opening shows the Code panel wherever
    /// it is docked, with the selection's line; closing gives the keyboard
    /// back to the 3D view (the panel stays where it is).
    pub(in crate::ui) fn editor_changed(&mut self) {
        let open = self.session.editor() == Editor::Open;
        if open {
            self.widgets.workbench.present(workbench::PanelId::Code);
            self.sync_files();
            self.widgets.files.focus_text();
        } else {
            gtk::prelude::GtkWindowExt::set_focus(
                &self.widgets.window,
                Some(self.widgets.viewport.upcast_ref::<gtk::Widget>()),
            );
        }
        self.request_render();
    }

    /// The editor follows the selection while it is open: a place in a
    /// file is shown; a part, assembly or piece of hardware selected in
    /// the 3D view, the model tree or the finder moves the text to the
    /// line that places it.
    pub(in crate::ui) fn sync_files(&mut self) {
        let (open, selection, placed) = {
            let session = &self.session;
            let selection = session.selection().cloned();
            let placed = session
                .shown()
                .zip(selection.as_ref())
                .and_then(|(shown, selection)| link::placed_at(shown, selection));
            (session.editor() == Editor::Open, selection, placed)
        };
        let files = &self.widgets.files;
        match (&selection, placed) {
            (Some(Selection::Location(location)), _) if open => files.show_location(location),
            // A part, assembly or piece of hardware chosen anywhere opens
            // the file that places it, at that line, in the Code panel
            // (forward unless it would cover the 3D view; the keyboard
            // stays where it is).
            (Some(_), Some(placed)) if !self.state.linking => {
                files.reveal(&placed);
                self.widgets
                    .workbench
                    .bring_forward_beside_view(workbench::PanelId::Code);
            }
            _ => {}
        }
    }

    /// The editor's cursor rests on a name: select what it means in the
    /// design, without a history entry. A name the 3D view can't show
    /// leaves the selection as it is.
    pub(in crate::ui) fn text_linked(&mut self, found: files::CursorName) {
        let (analysis, symbol) = match found {
            files::CursorName::Name(analysis, symbol) => (analysis, symbol),
            // A broken name: the 3D view lets go, and the chip goes.
            files::CursorName::Unresolved => {
                let linked = self
                    .session
                    .selection()
                    .is_some_and(|s| !matches!(s, Selection::Location(_) | Selection::Problem(_)));
                if linked {
                    self.state.linking = true;
                    self.dispatch(Command::Link(None));
                    self.state.linking = false;
                }
                return;
            }
            files::CursorName::Other => return,
        };
        let selection = {
            let session = &self.session;
            session.drawable().and_then(|(design, _)| {
                link::selection_for(design, &analysis, symbol, session.selection())
            })
        };
        if let Some(selection) = selection {
            self.state.linking = true;
            self.dispatch(Command::Link(Some(selection)));
            self.state.linking = false;
        }
    }

    /// With `--window-screenshot`: saves the window shortly (once GTK has
    /// drawn it) and closes it.
    pub(in crate::ui) fn capture_if_asked(&mut self) {
        let Some(path) = self.state.window_screenshot.take() else {
            return;
        };
        let window = self.widgets.window.clone();
        // What `--type` typed is never saved, and closing doesn't ask.
        let files = self.widgets.files.clone();
        // An open finder would keep the window from closing.
        let finder = self.widgets.finder.dialog.clone();
        // The paintable follows the window's drawing from now on.
        let paintable = gtk::WidgetPaintable::new(Some(&window));
        let mut waited = 0;
        glib::timeout_add_local(Duration::from_millis(300), move || {
            // Wait until the window is on screen and drawn.
            waited += 1;
            // A tick makes the frame clock run, so GTK draws the window.
            window.queue_draw();
            window.add_tick_callback(|_, _| glib::ControlFlow::Break);
            if (!window.is_mapped() || waited < 3) && waited < 25 {
                return glib::ControlFlow::Continue;
            }
            // A view that hasn't drawn since it changed draws nothing yet:
            // try again on the next round.
            let Some(node) = window_node(&window, &paintable) else {
                if waited < 25 {
                    return glib::ControlFlow::Continue;
                }
                eprintln!("error: the window drew nothing");
                files.forget_unsaved();
                finder.force_close();
                close_for_good(&window);
                return glib::ControlFlow::Break;
            };
            if let Err(error) = save_node(&window, &node, &path) {
                eprintln!("error: {error:#}");
            }
            files.forget_unsaved();
            finder.force_close();
            close_for_good(&window);
            glib::ControlFlow::Break
        });
    }

    pub(in crate::ui) fn check_status_changed(&self) {
        {
            let session = &self.session;
            let status = session.check_status();
            self.widgets
                .header
                .show_pill(&status.pill(), session.problems().len(), &status.line());
            self.widgets
                .workbench
                .show_problem_count(session.problems().len());
            self.widgets.output.add_line(&status.line());
        }
        self.build_changed();
    }

    pub(in crate::ui) fn build_changed(&self) {
        let session = &self.session;
        self.widgets
            .header
            .show_build(session.build(), session.build_tooltip());
        if let Some(action) = self.widgets.window.lookup_action("build")
            && let Ok(action) = action.downcast::<gio::SimpleAction>()
        {
            action.set_enabled(session.build() == Build::Idle && session.buildable().is_ok());
        }
    }

    /// Opens another project folder.
    pub(in crate::ui) fn choose_folder(&mut self) {
        let dialog = gtk::FileDialog::builder()
            .title("Open a project folder")
            .modal(true)
            .build();
        let input = self.input();
        dialog.select_folder(
            Some(&self.widgets.window),
            gio::Cancellable::NONE,
            move |result| {
                let Ok(folder) = result else { return };
                let Some(path) = folder.path() else { return };
                input.emit(ProjectInput::GoToFolder(path).into());
            },
        );
    }

    /// Shows a project folder: the application brings forward the window
    /// already showing it, or answers with [`ProjectInput::OpenHere`]. A
    /// `.stave` file opens its folder.
    pub(in crate::ui) fn go_to_folder(&mut self, path: PathBuf) {
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
        // Nobody listening is not a mistake (a test's window).
        let _ = self.sender.output(WindowOutput::GoToFolder(folder));
    }

    /// Asks what to do with unsaved changes (save, throw away or stay),
    /// then sends `then`, unless people chose to stay.
    pub(in crate::ui) fn ask_about_unsaved(&mut self, then: impl Into<WindowInput>) {
        let then = then.into();
        let files = &self.widgets.files;
        let count = files.unsaved_count();
        if count == 0 {
            self.send(then);
            return;
        }
        let dialog = adw::AlertDialog::builder()
            .heading("Save your changes?")
            .body(if count == 1 {
                "One file has changes that aren't saved. If you don't save them, they are lost.".to_owned()
            } else {
                format!("{count} files have changes that aren't saved. If you don't save them, they are lost.")
            })
            .close_response("cancel")
            .default_response("save")
            .build();
        dialog.add_responses(&[
            ("cancel", "_Cancel"),
            ("discard", "_Don't Save"),
            ("save", "_Save"),
        ]);
        dialog.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
        dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);
        let input = self.input();
        let files = Rc::clone(files);
        let then = Cell::new(Some(then));
        dialog.connect_response(None, move |_, response| {
            let go_on = match response {
                "save" => files.save_all(),
                "discard" => true,
                _ => false,
            };
            if go_on && let Some(then) = then.take() {
                input.emit(then);
            }
        });
        dialog.present(Some(&self.widgets.window));
    }

    pub(in crate::ui) fn open_folder(&mut self, folder: PathBuf) {
        // The application keeps the recent projects, and which window
        // shows which folder.
        let _ = self.sender.output(WindowOutput::Opened(folder.clone()));
        self.session = Session::new(folder);
        self.editing_folder_changed();
        {
            let state = &mut self.state;
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
        self.show_status("Checking…", "");
        self.dispatch(Command::Check);
        self.view_changed();
        self.selection_changed();
    }

    /// Watches the project folder and checks again after a `.stave` file
    /// is saved (once the folder is quiet).
    pub(in crate::ui) fn watch(&mut self) {
        let folder = self.session.folder().to_path_buf();
        let (sender, receiver) = async_channel::unbounded::<()>();
        let watcher = Watcher::start(&folder, move || {
            // The window may be gone; then nobody needs to know.
            let _ = sender.send_blocking(());
        });
        match watcher {
            Ok(watcher) => {
                self.watcher = Some(watcher);
            }
            Err(error) => {
                self.watcher = None;
                eprintln!("warning: {error}");
                self.widgets.toasts.add_toast(
                    adw::Toast::builder()
                        .title("Not watching the folder for changes: check again with Ctrl+R")
                        .timeout(6)
                        .build(),
                );
                return;
            }
        }
        // Ends when the watcher goes (with the window, or for another
        // folder's), or once the window can't be told.
        let input = self.input();
        glib::spawn_future_local(async move {
            while receiver.recv().await.is_ok() {
                if input
                    .send(ProjectInput::Dispatch(Command::Check).into())
                    .is_err()
                {
                    break;
                }
            }
        });
    }
}
