//! The libadwaita window (the V2-C canvas, option A): a title bar with
//! back and forward, the finder's field (Ctrl+K), the Design / Cut list
//! switcher, the problems chip and the build button; and the design: the
//! 3D view filling the window, the model tree floating at its top left,
//! the selection's card beside the part, problems pinned where they
//! happen, the dock at the bottom, and at the right edge the rail of open
//! files with the editor that slides in beside the view.
//!
//! All state that views share lives in a `stave_app::session::Session`:
//! the window sends it commands and redraws what the events it returns
//! name (`docs/gui.md`). The project is checked in the background on a
//! worker thread whenever a `.stave` file is saved, and the result
//! replaces the old one whole. What only the 3D view needs (the camera,
//! the renderer, focus mode's scrub, the "Open it" slider) stays here.
//!
//! Everything about the design comes from `stave-app` (checking, the
//! tree, the inspector, focus mode, problems) and `stave-scene` (poses,
//! camera, picking); this module only wires widgets to them.
//!
//! The window's code is split by what it does. This module holds the
//! window's state and the flow from commands to events (`Window::open`,
//! `dispatch`, `handle`, a check's result). `widgets` builds the widgets,
//! `events` wires their signals and `actions` the window's `win.*`
//! actions and main menu. `selection` keeps the panels on the selection;
//! `camera`, `drawing`, `motion`, `pins` and `focus_mode` each hold one
//! part of the 3D view; `capture` exports and takes screenshots; and
//! `options` says how a window is asked to open.

mod actions;
mod app;
mod camera;
mod capture;
mod chrome;
mod dock;
mod drawing;
mod edit_card;
mod edit_demo;
mod edit_menu;
mod edit_panel;
mod edit_script;
mod edit_view;
mod edit_writer;
mod editor;
mod events;
mod files;
mod finder;
mod floating;
mod focus_mode;
mod focus_panel;
mod focus_stage;
#[cfg(test)]
mod gtk_test;
mod header;
mod icons;
mod inspector;
mod motion;
mod new_project;
mod option_search;
mod options;
mod output;
mod panels;
mod pins;
mod remember;
mod sections;
mod selection;
mod slider;
mod start_page;
mod style;
mod tab_strip;
mod tree;
mod view_layout;
mod view_tools;
mod viewport;
mod widgets;
mod workbench;

use stave_app::i18n::{fill_in, tr};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use anyhow::{Context as _, Result, bail};
use gtk::{gio, glib};
use stave_app::Contact;
use stave_app::check::{self, Checked, Watcher};
use stave_app::focus::{self, Focus};
use stave_app::inspect;
use stave_app::link;
use stave_app::problems::Marks;
use stave_app::session::{
    Build, Command, Editor, Event, Explode, ProblemIndex, Selection, Session, SourceLocation, View,
};
use stave_app::view::Highlight;
use stave_model::solved::AssemblyId;
use stave_scene::{Look, Orbit, Overlay, Posed};

use crate::render::Renderer;
use capture::settle_for_screenshot;
use chrome::MotionBar;
use dock::Dock;
pub use edit_demo::Showcase;
use files::FilesPage;
pub use files::Wish;
use finder::Finder;
use focus_stage::{Caption, Pictures, ProblemCard, StepCard, SwingCard};
use header::Header;
use inspector::Inspector;
pub use options::{Options, StartCamera, Theme};
use pins::Pins;
use stave_app::problems::MotionCard;
use tree::TreeSidebar;
use view_tools::{CameraToolbar, PlanSwitch, ViewToolbar};
use viewport::Viewport;
use widgets::build_widgets;

/// Moving the mouse less than this many pixels between press and release
/// is a click, not a drag.
const CLICK_SLOP: f64 = 4.0;

/// A mouse's Back button (the thumb button nearer the wrist), as GDK
/// numbers it.
const MOUSE_BACK: u32 = 8;
/// A mouse's Forward button.
const MOUSE_FORWARD: u32 = 9;

/// How fast focus mode plays: the whole way in 2.5 seconds.
const PLAY_PER_SECOND: f64 = 0.4;

/// What one of the window's own actions does, given the window.
type ActionHandler = Box<dyn Fn(&Rc<Window>)>;

/// How fast pulling apart plays: the whole way in 3 seconds.
const EXPLODE_PER_SECOND: f64 = 1.0 / 3.0;

/// The front view looks a little down on the design (radians).
const FRONT_PITCH: f32 = 0.12;

/// How long hardware focus's small views wait for their size to settle
/// before rendering again at it.
const PICTURES_DELAY: Duration = Duration::from_millis(150);

/// How much room a chosen part is framed with: the camera stands this many
/// times farther back than the tightest fit, so what is round it shows.
const FRAME_ROOM: f32 = 1.6;

/// The space kept clear round what the editing puts on the 3D view (its
/// menu, cards, handles and labels), in logical pixels.
const EDGE: i32 = 14;

/// Runs Stave until its last window is closed. If Stave is already
/// running, the folders are handed to it instead (except for
/// `--window-screenshot`, which always runs on its own).
pub fn run(options: Options) -> Result<()> {
    let folders = options
        .folders
        .iter()
        .map(|folder| app::project_folder(folder))
        .collect::<Result<Vec<PathBuf>>>()?;
    if options.window_screenshot.is_some() && folders.len() != 1 {
        bail!("--window-screenshot needs one project folder");
    }
    gtk::init().context("can't start GTK (is there a display?)")?;
    // Some desktops (KDE among them) write the old GTK 3 dark-theme switch
    // into settings.ini; libadwaita warns about it and ignores it. Light
    // and dark come from AdwStyleManager's colour scheme alone (the style
    // menu, or the system's preference through the settings portal), so
    // the old switch is turned off before libadwaita starts.
    if let Some(settings) = gtk::Settings::default()
        && settings.is_gtk_application_prefer_dark_theme()
    {
        settings.set_gtk_application_prefer_dark_theme(false);
    }
    adw::init().context("can't start GTK (is there a display?)")?;
    let mut flags = gio::ApplicationFlags::HANDLES_OPEN;
    if options.window_screenshot.is_some() {
        flags |= gio::ApplicationFlags::NON_UNIQUE;
    }
    let application = adw::Application::builder()
        .application_id(app::APP_ID)
        .flags(flags)
        .build();
    let shell = app::Shell::new(&application, options);
    let this = shell.clone();
    application.connect_startup(move |_| this.startup());
    let this = shell.clone();
    application.connect_activate(move |_| this.activate());
    application.connect_open(move |_, files, _| {
        for path in files.iter().filter_map(gio::File::path) {
            shell.open_folder(&path);
        }
    });
    // GApplication reads the folders as files to open: in this process
    // if it is the first Stave, or handed to the one already running.
    let mut arguments = vec!["stave-gtk".to_owned()];
    arguments.extend(
        folders
            .iter()
            .map(|folder| gio::File::for_path(folder).uri().to_string()),
    );
    let status = application.run_with_args(&arguments);
    if status != glib::ExitCode::SUCCESS {
        bail!("Stave stopped with an error");
    }
    Ok(())
}

/// What the command line asks the window to start with, applied once the
/// first result is in.
struct Wanted {
    scene: Option<String>,
    selection: Option<String>,
    focus: Option<String>,
    plan: bool,
    in_3d: bool,
    step: Option<f32>,
    problem: Option<usize>,
    explode: Option<f32>,
    view: View,
    editor: bool,
    finder: Option<String>,
    xray: bool,
    checking: bool,
    building: bool,
    file: Option<String>,
    wish: Wish,
}

/// A problem posed in the view.
struct ShownProblem {
    contact: Contact,
    highlight: Highlight,
    /// The motion card, for a door or drawer that hits.
    card: Option<MotionCard>,
    /// The motion's amount set by the slider (radians or millimetres);
    /// `None` for where the shared wood was measured.
    amount: Option<f32>,
    /// The slider works in degrees (a hinge) rather than millimetres.
    turns: bool,
}

/// Focus mode: one joint or piece of hardware.
struct FocusState {
    focus: Focus,
    /// 0 to 1: apart to together, or closed to fully open.
    progress: f32,
    options: focus::Options,
    /// Looking down from above, flat.
    plan: bool,
    /// Playing: the progress moves on each frame.
    playing: bool,
    /// The last frame's looks and overlay.
    looks: Vec<Look>,
    overlay: Overlay,
}

/// What the floating motion bar opens.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Opening {
    assembly: AssemblyId,
    /// Radians or millimetres.
    amount: f32,
    turns: bool,
}

/// What only the 3D view needs.
struct State {
    wanted: Option<Wanted>,
    /// Where to save the window once drawn.
    window_screenshot: Option<PathBuf>,
    renderer: Result<Renderer, String>,
    /// The result whose scene the renderer holds.
    uploaded: Option<Rc<Checked>>,
    posed: Option<Posed>,
    orbit: Option<Orbit>,
    /// The view's size in device pixels.
    size: (u32, u32),
    render_queued: bool,
    /// The drag offset already applied.
    drag_applied: (f64, f64),
    problem: Option<ShownProblem>,
    opening: Option<Opening>,
    focus: Option<FocusState>,
    /// Ignore control changes made by the window itself.
    updating_controls: bool,
    /// "Pull apart" is on (its slider shows), even at 0.
    pull_apart: bool,
    /// Pulling apart is playing.
    exploding: bool,
    /// How focus mode should open, from the command line, once.
    focus_start: Option<(Option<bool>, Option<f32>)>,
    /// Don't run checks (a screenshot of the checking state).
    hold_checks: bool,
    /// The selection is changing because the editor's cursor moved: the
    /// editor stays where it is.
    linking: bool,
    /// Where the camera starts, applied once the view has a size and the
    /// command line's selection is made.
    start_camera: Option<StartCamera>,
    /// Hardware focus's small views render again once their size has
    /// settled (not on every frame of a resize).
    pictures_timer: Option<glib::SourceId>,
    /// X-ray: everything but the selection see-through.
    xray: bool,
    /// What a partner link in the Parts panel points at, while the pointer
    /// is over it: outlined like the selection.
    hovered: Vec<stave_scene::Item>,
    /// The latest check's marks, when its ids are the shown design's.
    marks: Rc<Marks>,
    /// Editing steps to run once the design is drawn (`--edit-script`).
    edit_script: Option<PathBuf>,
}

/// The window's widgets.
struct Widgets {
    window: adw::ApplicationWindow,
    header: Header,
    toasts: adw::ToastOverlay,
    views: gtk::Stack,
    /// The 3D view with everything that floats over it.
    design: gtk::Overlay,
    tree: TreeSidebar,
    inspector: Inspector,
    /// The Details panel's content: the selection's account (the
    /// inspector), then its editing controls (the "Add onto" button and
    /// the item's changes).
    details: gtk::Box,
    /// The Edit panel's content: the card in progress, or how to start.
    edit_holder: gtk::Box,
    /// Over the window's panels: the search over a setting's options.
    sheet: gtk::Overlay,
    dock: Rc<Dock>,
    /// The dock and its panels round the 3D view.
    workbench: Rc<workbench::Workbench>,
    /// The Problems panel.
    problems: Rc<panels::ProblemsPanel>,
    /// The Files panel.
    file_list: Rc<panels::FilesList>,
    /// A short line at the top of the view: focus mode, posed problems.
    hint: gtk::Label,
    hint_box: gtk::Box,
    /// The problems pinned off screen: one chip at the view's edge.
    edge_pin: gtk::Button,
    edge_label: gtk::Label,
    finder: Rc<Finder>,
    view_stack: gtk::Stack,
    empty: view_tools::EmptyView,
    viewport: Viewport,
    motion: MotionBar,
    stage: gtk::Box,
    view_toolbar: ViewToolbar,
    camera: CameraToolbar,
    contact_key: gtk::Box,
    plan_switch: PlanSwitch,
    step_card: StepCard,
    swing_card: SwingCard,
    caption: Caption,
    pictures: Pictures,
    problem_card: ProblemCard,
    files: Rc<FilesPage>,
    /// The Output panel: what the checks said.
    output: output::OutputPanel,
    /// The controls floating on the 3D view, and their docking: held so
    /// the placing and dragging (which only hold it weakly) live on.
    _floats: Rc<floating::Floats>,
}

struct Window {
    widgets: Widgets,
    session: RefCell<Session>,
    state: RefCell<State>,
    /// The problems pinned on the model, as overlay children of the
    /// design (made again when the result or the posed problem changes,
    /// moved on every frame).
    pins: RefCell<Pins>,
    /// Kept alive for as long as the window: dropping it stops watching.
    watcher: RefCell<Option<Watcher>>,
    /// Editing in the 3D view: the menu, the card and the ghost.
    editing: edit_view::EditView,
    /// The application: the other windows, recent projects, what is
    /// remembered.
    shell: Rc<app::Shell>,
}

impl Window {
    fn open(
        application: &adw::Application,
        folder: PathBuf,
        options: Options,
        shell: Rc<app::Shell>,
    ) -> Result<Rc<Window>> {
        if options.window_screenshot.is_some() {
            settle_for_screenshot();
        }
        let widgets = build_widgets(application, &folder, &shell, &options)?;
        let renderer = Renderer::new()
            .map(|mut renderer| {
                // The live view's frames go to GTK as dmabufs where both
                // sides can, and by readback otherwise.
                let display = gtk::prelude::WidgetExt::display(&widgets.window);
                let accepted = crate::render::display_modifiers(&display);
                let used = renderer.use_dmabufs(accepted);
                if crate::render::profile::enabled() {
                    match used {
                        Ok(()) => eprintln!("stave-profile: frames go to GTK as dmabufs"),
                        Err(error) => {
                            eprintln!("stave-profile: frames go by readback: {error:#}");
                        }
                    }
                }
                renderer
            })
            .map_err(|error| format!("{error:#}"));
        let window = Rc::new(Window {
            widgets,
            session: RefCell::new(Session::new(folder)),
            state: RefCell::new(State {
                wanted: Some(Wanted {
                    scene: options.scene,
                    selection: options.select,
                    focus: options.focus,
                    plan: options.plan,
                    in_3d: options.in_3d,
                    step: options.step,
                    problem: options.problem,
                    explode: options.explode,
                    view: options.view,
                    editor: options.editor,
                    finder: options.finder,
                    xray: options.xray,
                    checking: options.checking,
                    building: options.building,
                    file: options.file,
                    wish: options.wish,
                }),
                window_screenshot: options.window_screenshot,
                start_camera: (options.camera != StartCamera::default()).then_some(options.camera),
                pictures_timer: None,
                renderer,
                uploaded: None,
                posed: None,
                orbit: None,
                size: (1, 1),
                render_queued: false,
                drag_applied: (0.0, 0.0),
                problem: None,
                opening: None,
                focus: None,
                updating_controls: false,
                pull_apart: false,
                exploding: false,
                focus_start: None,
                hold_checks: false,
                linking: false,
                xray: false,
                hovered: Vec::new(),
                marks: Rc::default(),
                edit_script: None,
            }),
            pins: RefCell::new(Pins::default()),
            watcher: RefCell::new(None),
            editing: edit_view::EditView::new(),
            shell,
        });
        window.install_actions();
        window.connect_events();
        window.connect_editing();
        if let Some(showcase) = options.edit_fixture {
            // The fixture's card takes the Edit panel's place of the
            // session's own.
            if edit_demo::show(
                &window.widgets.design,
                &window.widgets.edit_holder,
                showcase,
            ) {
                window.widgets.workbench.present(workbench::PanelId::Edit);
            }
        }
        window.state.borrow_mut().edit_script = options.edit_script;
        if options.new_project {
            window.ask_new_project();
        }
        window.watch();
        // The keyboard starts in the 3D view, not in the first focusable
        // widget GTK finds, which can be a problem's selectable `stave
        // check` text in the card (it would show a text cursor).
        gtk::prelude::GtkWindowExt::set_focus(
            &window.widgets.window,
            Some(window.widgets.viewport.upcast_ref::<gtk::Widget>()),
        );
        if options.maximized {
            window.widgets.window.maximize();
        }
        window.widgets.window.present();
        window.show_folder();
        window.dispatch(Command::Check);
        Ok(window)
    }

    // ---- the session ----

    /// Sends a command to the session and redraws what changed.
    fn dispatch(self: &Rc<Self>, command: Command) {
        let events = self.session.borrow_mut().apply(command);
        self.handle(events);
    }

    /// Redraws what the events name. A new result is taken first, so the
    /// other views see it.
    fn handle(self: &Rc<Self>, events: Vec<Event>) {
        let result = events.contains(&Event::ResultChanged);
        if result {
            self.result_changed();
        }
        for event in events {
            match event {
                Event::ResultChanged => {}
                Event::ViewChanged => self.view_changed(),
                Event::SelectionChanged => self.selection_changed(),
                Event::FocusChanged => self.focus_changed(),
                Event::HistoryChanged => {
                    // Back and Forward are in the View menu and on
                    // Alt+← / Alt+→; each is offered only when it can go.
                    let session = self.session.borrow();
                    for (name, enabled) in [
                        ("back", session.can_go_back()),
                        ("forward", session.can_go_forward()),
                    ] {
                        if let Some(action) = self.widgets.window.lookup_action(name)
                            && let Ok(action) = action.downcast::<gio::SimpleAction>()
                        {
                            action.set_enabled(enabled);
                        }
                    }
                }
                Event::SceneChanged => self.scene_changed(),
                Event::ExplodeChanged => self.explode_changed(),
                Event::EditorChanged => self.editor_changed(),
                Event::CheckStatusChanged => self.check_status_changed(),
                Event::BuildChanged => self.build_changed(),
                Event::CheckStarted(ticket) => self.run_check(ticket),
                Event::Edit(event) => self.edit_event(event),
                Event::Notice(notice) => {
                    self.widgets.toasts.add_toast(
                        adw::Toast::builder()
                            .title(notice.text())
                            .timeout(4)
                            .build(),
                    );
                }
            }
        }
    }

    /// Selects something and frames it: for the parts list, the finder,
    /// "show in 3D" and the command line. Clicking in the 3D view selects
    /// without moving the camera (`pick`). The 3D View comes to the front
    /// (brought back if it was closed), so what was chosen can be seen.
    fn select_and_frame(self: &Rc<Self>, selection: Option<Selection>) {
        self.widgets.workbench.present(workbench::PanelId::View);
        self.dispatch(Command::Select(selection));
        self.frame_selection();
    }

    /// Runs a check on a worker thread and hands its outcome back.
    fn run_check(self: &Rc<Self>, ticket: stave_app::session::CheckTicket) {
        if self.state.borrow().hold_checks {
            return;
        }
        let folder = self.session.borrow().folder().to_path_buf();
        // The project as the editor shows it, unsaved changes included.
        let overlay = self.widgets.files.overlay_for_check();
        let window = Rc::downgrade(self);
        crate::render::profile::edit_started();
        glib::spawn_future_local(async move {
            let outcome = gio::spawn_blocking(move || check::run_with(&folder, &overlay))
                .await
                .unwrap_or_else(|_| check::Outcome::CantOpen {
                    message: "checking the design crashed".to_owned(),
                    elapsed: Duration::ZERO,
                });
            crate::render::profile::edit_checked();
            let Some(window) = window.upgrade() else {
                return;
            };
            let events = window.session.borrow_mut().finish_check(ticket, outcome);
            window.handle(events);
        });
    }

    /// The folder in the title until the design is in.
    fn show_folder(self: &Rc<Self>) {
        let folder = self.session.borrow().folder().to_path_buf();
        self.show_title();
        self.widgets.files.set_folder(&folder);
        self.widgets.file_list.set_folder(&folder);
    }

    /// The project's or library's own title (the folder's name until one
    /// is read), with the folder under it.
    fn show_title(&self) {
        let (title, folder) = {
            let session = self.session.borrow();
            (session.title(), session.folder().to_path_buf())
        };
        self.widgets.header.show_title(&title, &shown_path(&folder));
        self.widgets
            .window
            .set_title(Some(&format!("{title} — Stave")));
        self.describe_view();
    }

    fn result_changed(self: &Rc<Self>) {
        self.grips_result_changed();
        let (shown, latest, showing_older) = {
            let session = self.session.borrow();
            (
                session.shown().cloned(),
                session.latest().cloned(),
                session.showing_older(),
            )
        };
        self.show_title();
        let widgets = &self.widgets;
        widgets.files.show_result(latest.clone());
        {
            let session = self.session.borrow();
            let selected = match session.selection() {
                Some(Selection::Problem(index)) => Some(index.0),
                _ => None,
            };
            let reports = latest
                .as_deref()
                .map(stave_app::problems::reports)
                .unwrap_or_default();
            widgets.problems.show(&reports, selected, showing_older);
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
        widgets.files.show_marks(Rc::clone(&marks));
        let tree_marks = if showing_older { Rc::default() } else { marks };
        self.state.borrow_mut().marks = Rc::clone(&tree_marks);
        widgets.tree.show_marks(tree_marks);
        let empty = edit_view::empty_state(&self.session.borrow());
        if let Some(empty) = empty {
            // The sidebar and the inspector say why too (for a library,
            // what it defines and how to use it), not "Nothing selected".
            widgets.inspector.show_idle(Some(empty.inspector()));
            self.show_empty(&empty.title, &empty.description, empty.offers_open);
            widgets.tree.show_empty(empty);
            self.show_card();
            // What doesn't need the 3D view (the editor) still opens as
            // asked.
            self.apply_wanted();
            return;
        }
        widgets.inspector.show_idle(None);
        let Some(shown) = shown else {
            return;
        };
        // An older design on show is said by the red problems chip and a
        // line over the Problems panel's list, not a banner.
        let Some((design, graph)) = shown.drawable() else {
            return;
        };
        widgets.tree.show_tree(
            inspect::parts_tree(design),
            stave_app::part_joints::PartJoints::new(design),
        );
        let first = {
            let mut state = self.state.borrow_mut();
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
        let index = self.session.borrow().scene_index();
        widgets.header.show_scenes(&titles, index);
        widgets.view_stack.set_visible_child_name("view");
        self.show_moves();
        self.repose();
        if first {
            let mut state = self.state.borrow_mut();
            state.orbit = Self::frame_all(&state, &shown);
        }
        self.apply_wanted();
        self.apply_start_camera();
        // Whatever the session carried over, shown again; the editor stays
        // where people are typing.
        self.state.borrow_mut().linking = true;
        self.selection_changed();
        self.state.borrow_mut().linking = false;
        self.focus_changed();
        self.request_render();
        // The 3D view draws (and saves a screenshot) only when it shows.
        if !self.shows_3d() {
            self.capture_if_asked();
        }
    }

    /// Applies the command line's wishes once, after the first result.
    fn apply_wanted(self: &Rc<Self>) {
        let Some(wanted) = self.state.borrow_mut().wanted.take() else {
            return;
        };
        let found = {
            let session = self.session.borrow();
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
            self.state.borrow_mut().focus_start = Some((plan, wanted.step));
            self.dispatch(Command::Focus(target));
            self.state.borrow_mut().focus_start = None;
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
            let location = SourceLocation::parse(self.session.borrow().folder(), file);
            if let Some(location) = location {
                self.dispatch(Command::GoToSource(location));
            }
        }
        if wanted.wish != Wish::default() {
            self.widgets.files.wish(wanted.wish);
        }
        if wanted.checking || wanted.building {
            self.state.borrow_mut().hold_checks = true;
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

    // ---- focus mode ----

    // ---- the 3D view ----

    /// A short message at the bottom of the window.
    fn toast(&self, title: &str) {
        self.widgets
            .toasts
            .add_toast(adw::Toast::builder().title(title).timeout(4).build());
    }

    fn show_status(&self, title: &str, description: &str) {
        self.show_empty(title, description, false);
    }

    /// Shows the 3D view's empty state instead of the view.
    fn show_empty(&self, title: &str, description: &str, offers_open: bool) {
        self.widgets.empty.show(title, description, offers_open);
        self.widgets.view_stack.set_visible_child_name("status");
        self.capture_if_asked();
    }

    // ---- the rest of the session's events ----

    fn check_status_changed(&self) {
        {
            let session = self.session.borrow();
            let status = session.check_status();
            self.widgets
                .header
                .show_pill(&status.pill(), session.problems().len(), &status.line());
            self.widgets
                .workbench
                .show_problem_count(session.problems().len());
            self.widgets.output.add_line(&status.line());
        }
        self.describe_view();
        self.build_changed();
    }

    fn build_changed(&self) {
        let session = self.session.borrow();
        self.widgets
            .header
            .show_build(session.build(), &session.build_tooltip());
        if let Some(action) = self.widgets.window.lookup_action("build")
            && let Ok(action) = action.downcast::<gio::SimpleAction>()
        {
            action.set_enabled(session.build() == Build::Idle && session.buildable().is_ok());
        }
    }

    // ---- events ----

    // ---- watching ----
}

/// A folder as people read it: relative to where Stave was started when it
/// is inside it, under `~` when it is in the home folder.
fn shown_path(folder: &Path) -> String {
    if let Ok(here) = std::env::current_dir()
        && let Ok(inside) = folder.strip_prefix(&here)
        && !inside.as_os_str().is_empty()
    {
        return inside.display().to_string();
    }
    if let Some(home) = std::env::var_os("HOME")
        && let Ok(inside) = folder.strip_prefix(&home)
    {
        return format!("~/{}", inside.display());
    }
    folder.display().to_string()
}

/// The 3D view's description for screen readers: what is selected, and
/// what the last check found ("door_l selected. 1 error.").
fn view_description(selected: Option<&str>, problems: &str) -> String {
    match selected {
        Some(title) => fill_in(
            &tr("{title} selected. {problems}."),
            &[("title", &title), ("problems", &problems)],
        ),
        None => fill_in(
            &tr("Nothing selected. {problems}."),
            &[("problems", &problems)],
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_3d_view_says_what_is_selected_and_found() {
        assert_eq!(
            view_description(Some("door_l"), "1 error"),
            "door_l selected. 1 error."
        );
        assert_eq!(
            view_description(None, "No problems"),
            "Nothing selected. No problems."
        );
    }
}
