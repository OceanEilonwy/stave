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

mod app;
mod chrome;
mod dock;
mod edit_card;
mod edit_demo;
mod edit_menu;
mod edit_panel;
mod edit_script;
mod edit_view;
mod edit_writer;
mod editor;
mod files;
mod finder;
mod floating;
mod focus_panel;
mod focus_stage;
#[cfg(test)]
mod gtk_test;
mod header;
mod icons;
mod inspector;
mod new_project;
mod option_search;
mod output;
mod panels;
mod relm4_shim;
mod remember;
mod sections;
mod slider;
mod start_page;
mod style;
mod tab_strip;
mod tree;
mod update;
mod view_layout;
mod view_tools;
mod viewport;
mod workbench;

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use anyhow::{Context as _, Result, bail};
use gtk::{gdk, gio, glib};
use relm4::{Component, ComponentController, ComponentParts, ComponentSender};
use stave_app::Contact;
use stave_app::check::{self, Checked, Watcher};
use stave_app::finder::{self as finding, Goal};
use stave_app::focus::{self, Focus, Target};
use stave_app::inspect;
use stave_app::link;
use stave_app::problems::{self, Mark, Marks};
use stave_app::session::{
    Build, Command, Editor, Event, Explode, LineNumber, ProblemIndex, Selection, Session,
    SourceLocation, View,
};
use stave_app::view::{self, Highlight};
use stave_model::solved::AssemblyId;
use stave_scene::{Look, NodeId, Orbit, Overlay, Posed};
use update::editing::EditingInput;
use update::selection::SelectionInput;

use crate::render::{Backdrop, Frame, Highlighted, Renderer};
use chrome::MotionBar;
use dock::Dock;
pub use edit_demo::Showcase;
use files::FilesPage;
pub use files::Wish;
use finder::{Choice, Finder};
use focus_stage::{Caption, Pictures, ProblemCard, StepCard, SwingCard};
use header::Header;
use inspector::Inspector;
use stave_app::problems::MotionCard;
use tree::TreeSidebar;
use view_tools::{CameraToolbar, PlanSwitch, ViewToolbar};
use viewport::Viewport;

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

/// The light or dark style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    /// Follow the system.
    #[default]
    System,
    /// Always light.
    Light,
    /// Always dark.
    Dark,
}

impl Theme {
    /// The theme a word names: `system`, `light`, `dark`.
    pub fn from_word(word: &str) -> Option<Theme> {
        match word {
            "system" => Some(Theme::System),
            "light" => Some(Theme::Light),
            "dark" => Some(Theme::Dark),
            _ => None,
        }
    }

    fn word(self) -> &'static str {
        match self {
            Theme::System => "system",
            Theme::Light => "light",
            Theme::Dark => "dark",
        }
    }

    fn scheme(self) -> adw::ColorScheme {
        match self {
            Theme::System => adw::ColorScheme::Default,
            Theme::Light => adw::ColorScheme::ForceLight,
            Theme::Dark => adw::ColorScheme::ForceDark,
        }
    }
}

/// How to open the window.
#[derive(Default)]
pub struct Options {
    /// The project folders, each in a window of its own; none shows the
    /// start page.
    pub folders: Vec<PathBuf>,
    /// The scene to start in.
    pub scene: Option<String>,
    /// The item to start with selected, by name.
    pub select: Option<String>,
    /// The joint or hardware to start focused on, by name.
    pub focus: Option<String>,
    /// Focus mode starts looking from above.
    pub plan: bool,
    /// Focus mode starts in 3D, even where it would look from above.
    pub in_3d: bool,
    /// How far along focus mode starts: 0 apart to 1 together, or 0
    /// closed to 1 fully open.
    pub step: Option<f32>,
    /// The problem to start showing (its index in the problems list).
    pub problem: Option<usize>,
    /// The explode amount to start with.
    pub explode: Option<f32>,
    /// The view to start in.
    pub view: View,
    /// Open the editor beside the 3D view.
    pub editor: bool,
    /// Open the finder with this typed (screenshots).
    pub finder: Option<String>,
    /// Start with X-ray on.
    pub xray: bool,
    /// Lay the panels out from this file instead of the user's own.
    pub layout: Option<PathBuf>,
    /// Light, dark or the system's; `None` for the style chosen last
    /// time.
    pub theme: Option<Theme>,
    /// Save the window to this PNG once drawn, then close.
    pub window_screenshot: Option<PathBuf>,
    /// The window's size, in logical pixels; `None` for the size it had
    /// last time.
    pub size: Option<(u32, u32)>,
    /// Start maximised (as it was last time).
    pub maximized: bool,
    /// For the screenshot: hold a second check running, to show the
    /// checking state.
    pub checking: bool,
    /// For the screenshot: press the build button and hold its check
    /// running, to show the building state.
    pub building: bool,
    /// The file to open in the editor, as `doors.stave:9`.
    pub file: Option<String>,
    /// What the editor shows once the names are in (screenshots).
    pub wish: Wish,
    /// Where the camera starts (screenshots of a close view).
    pub camera: StartCamera,
    /// Show the editing menu or card from the contract's fixture.
    pub edit_fixture: Option<edit_demo::Showcase>,
    /// Editing steps to run once the design is drawn (screenshots).
    pub edit_script: Option<PathBuf>,
    /// Open the new project dialog.
    pub new_project: bool,
}

/// Where the 3D view's camera starts, from the command line.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct StartCamera {
    /// Degrees about the up axis from the default view.
    pub turn: f32,
    /// Degrees above the horizontal, instead of the default.
    pub pitch: Option<f32>,
    /// Frame the selection rather than everything.
    pub frame_selection: bool,
    /// Move this many times closer once framed.
    pub zoom: Option<f32>,
}

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
    // The application's component: GApplication's signals only queue its
    // messages. Activating and opening hold the application until the
    // message is handled, so it doesn't end before its first window.
    let shell = app::Shell::builder()
        .launch(app::ShellInit {
            application: application.clone(),
            options,
        })
        .detach();
    let input = shell.sender().clone();
    application.connect_startup(move |_| input.emit(app::ShellInput::Startup));
    let input = shell.sender().clone();
    application.connect_activate(move |application| {
        input.emit(app::ShellInput::Activate(application.hold()));
    });
    let input = shell.sender().clone();
    application.connect_open(move |application, files, _| {
        let folders = files.iter().filter_map(gio::File::path).collect();
        input.emit(app::ShellInput::Open(folders, application.hold()));
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
    // The application, its windows and the start page go with it.
    drop(shell);
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

/// A problem pinned on the model: its widget, made once per result and
/// selection, and the place it is anchored to, projected again on every
/// frame.
struct Pin {
    widget: gtk::Box,
    index: ProblemIndex,
    mark: Mark,
    message: String,
    anchor: PinAnchor,
}

/// Where a pin is anchored in the scene.
enum PinAnchor {
    /// The middle of the problem's first shared volume (already posed).
    Volume(glam::Vec3),
    /// The middle of the bounds of the parts it is about, as posed now.
    Nodes(Vec<NodeId>),
}

/// The pins on the model and what they were made for: they are made again
/// only when the result, the shown design or the posed problem changes.
#[derive(Default)]
struct Pins {
    made_for: Option<PinsKey>,
    pins: Vec<Pin>,
    /// The off-screen chip as last shown (first problem, how many more), so
    /// its words are written only when they change.
    edge: Option<(ProblemIndex, usize)>,
}

/// What a set of pins was made for.
struct PinsKey {
    latest: Option<Rc<Checked>>,
    shown: Rc<Checked>,
    posed_problem: Option<ProblemIndex>,
}

impl PinsKey {
    fn same(&self, other: &Self) -> bool {
        let latest = match (&self.latest, &other.latest) {
            (Some(a), Some(b)) => Rc::ptr_eq(a, b),
            (None, None) => true,
            (Some(_), None) | (None, Some(_)) => false,
        };
        latest && Rc::ptr_eq(&self.shown, &other.shown) && self.posed_problem == other.posed_problem
    }
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
    /// The motion bar shows (it was last told to show something).
    motion_shown: bool,
    focus: Option<FocusState>,
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
    /// settled (not on every frame of a resize): the number of the last
    /// ask, so only its timer renders.
    pictures_asked: u64,
    /// Focus mode's playing, frame by frame.
    focus_clock: Clock,
    /// Pulling apart's playing, frame by frame.
    explode_clock: Clock,
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

/// The clock of something that plays frame by frame: the frame time it
/// last moved at, and whether the next frame is asked for already.
#[derive(Default)]
struct Clock {
    last: Option<i64>,
    ticking: bool,
}

impl Clock {
    /// The seconds since the last frame it moved at (none at the first),
    /// moving it to `now` (in microseconds, as the frame clock gives it).
    fn seconds_to(&mut self, now: i64) -> f64 {
        let seconds = self.last.map_or(0.0, |before| {
            f64::from(i32::try_from(now - before).unwrap_or(0)) / 1e6
        });
        self.last = Some(now);
        seconds
    }
}

/// The window's widgets.
pub(in crate::ui) struct Widgets {
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
    workbench: workbench::Workbench,
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
    finder: Finder,
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
    /// The controls floating on the 3D view, and their docking.
    floats: floating::Floats,
}

impl Widgets {
    /// Offers the view's everyday controls (the scene menu, X-ray, Pull
    /// apart and the moving assemblies' chips), or takes them away while
    /// a problem is posed or a joint is in focus.
    fn offer_view_tools(&self, offered: bool) {
        self.dock.offer(offered);
        self.view_toolbar.offer(offered);
    }
}

/// The project window: a relm4 component whose model is everything the
/// window keeps. The application owns its [`Controller`](relm4::Controller)
/// for exactly as long as its GTK window is open (until GTK takes the
/// window out of the application). Only
/// [`Window::update`] changes it; the widgets, timers and background work
/// hold a [`ComponentSender`] and send it [`WindowInput`]s, which are
/// queued and handled one at a time.
pub(in crate::ui) struct Window {
    widgets: Widgets,
    session: Session,
    state: State,
    /// The problems pinned on the model, as overlay children of the
    /// design (made again when the result or the posed problem changes,
    /// moved on every frame).
    pins: Pins,
    /// Kept for as long as the window: dropping it stops watching.
    watcher: Option<Watcher>,
    /// Editing in the 3D view: the menu, the card and the ghost.
    editing: edit_view::EditView,
    /// Where the window's handlers, timers and background work send its
    /// messages.
    sender: ComponentSender<Window>,
    /// The handler on the style manager, which outlives the window:
    /// disconnected when the window shuts down.
    style_handler: Option<glib::SignalHandlerId>,
}

/// What a project window is opened with.
pub(in crate::ui) struct WindowInit {
    /// Its widgets, built (and their window) before it is launched.
    pub(super) widgets: Widgets,
    /// The project folder it shows.
    pub(super) folder: PathBuf,
    /// The command line's wishes.
    pub(super) options: Options,
}

/// What a project window is told: each group of its behaviour has its
/// own messages (`update/`).
#[derive(Debug)]
pub(in crate::ui) enum WindowInput {
    /// What is selected and focused.
    Selection(update::selection::SelectionInput),
    /// The project: folders, files, checks and the window's actions.
    Project(update::project::ProjectInput),
    /// Editing in the 3D view.
    Editing(update::editing::EditingInput),
    /// The 3D view: camera, drawing, pulling apart.
    Rendering(update::rendering::RenderingInput),
}

/// What a project window tells the application.
#[derive(Debug)]
pub(in crate::ui) enum WindowOutput {
    /// Show this folder: the window already showing it comes forward, or
    /// this window is asked to open it
    /// ([`ProjectInput::OpenHere`](update::project::ProjectInput::OpenHere)).
    GoToFolder(PathBuf),
    /// The window now shows this folder.
    Opened(PathBuf),
    /// The window is closing: its size and whether it is maximised, to
    /// remember. The application is held until this is handled.
    Closing {
        /// Its size, as `default_size` gives it.
        size: (i32, i32),
        /// It is maximised.
        maximized: bool,
        /// Keeps the application running until the size is saved.
        hold: gio::ApplicationHoldGuard,
    },
}

impl Component for Window {
    type Init = WindowInit;
    type Input = WindowInput;
    type Output = WindowOutput;
    type CommandOutput = ();
    type Root = adw::ApplicationWindow;
    type Widgets = ();

    fn init_root() -> adw::ApplicationWindow {
        adw::ApplicationWindow::builder()
            .width_request(360)
            .height_request(400)
            .css_classes(["stave-window"])
            .build()
    }

    fn init(
        init: WindowInit,
        root: adw::ApplicationWindow,
        sender: ComponentSender<Window>,
    ) -> ComponentParts<Window> {
        let WindowInit {
            widgets,
            folder,
            options,
        } = init;
        let renderer = Renderer::new().map_err(|error| format!("{error:#}"));
        let mut window = Window {
            widgets,
            session: Session::new(folder),
            state: State {
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
                pictures_asked: 0,
                focus_clock: Clock::default(),
                explode_clock: Clock::default(),
                renderer,
                uploaded: None,
                posed: None,
                orbit: None,
                size: (1, 1),
                render_queued: false,
                drag_applied: (0.0, 0.0),
                problem: None,
                opening: None,
                motion_shown: false,
                focus: None,
                pull_apart: false,
                exploding: false,
                focus_start: None,
                hold_checks: false,
                linking: false,
                xray: false,
                hovered: Vec::new(),
                marks: Rc::default(),
                edit_script: options.edit_script,
            },
            pins: Pins::default(),
            watcher: None,
            editing: edit_view::EditView::new(),
            sender,
            style_handler: None,
        };
        update::project::wire(&window);
        update::selection::wire(&mut window);
        update::rendering::wire(&mut window);
        update::editing::wire(&mut window);
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
        if options.new_project {
            window.ask_new_project();
        }
        window.watch();
        // The keyboard starts in the 3D view, not in the first focusable
        // widget GTK finds, which can be a problem's selectable `stave
        // check` text in the card (it would show a text cursor).
        gtk::prelude::GtkWindowExt::set_focus(
            &root,
            Some(window.widgets.viewport.upcast_ref::<gtk::Widget>()),
        );
        if options.maximized {
            root.maximize();
        }
        root.present();
        window.show_folder();
        window.dispatch(Command::Check);
        ComponentParts {
            model: window,
            widgets: (),
        }
    }

    fn update(
        &mut self,
        message: WindowInput,
        _sender: ComponentSender<Window>,
        _root: &adw::ApplicationWindow,
    ) {
        match message {
            WindowInput::Selection(message) => self.update_selection(message),
            WindowInput::Project(message) => self.update_project(message),
            WindowInput::Editing(message) => self.update_editing(message),
            WindowInput::Rendering(message) => self.update_rendering(message),
        }
    }

    fn shutdown(&mut self, _widgets: &mut (), _output: relm4::Sender<WindowOutput>) {
        if let Some(handler) = self.style_handler.take() {
            adw::StyleManager::default().disconnect(handler);
        }
    }
}

impl Window {
    /// A clone of the window's input sender, for a handler, a timer or
    /// background work to send messages with. Sending fails quietly once
    /// the window has gone.
    fn input(&self) -> relm4::Sender<WindowInput> {
        self.sender.input_sender().clone()
    }

    /// Queues a message for the window, handled after the current one.
    fn send(&self, message: impl Into<WindowInput>) {
        // Only fails once the window has gone, when nobody needs to know.
        let _ = self.sender.input_sender().send(message.into());
    }

    /// Sends a command to the session and redraws what changed.
    fn dispatch(&mut self, command: Command) {
        let events = self.session.apply(command);
        self.handle(events);
    }

    /// Redraws what the events name. A new result is taken first, so the
    /// other views see it.
    fn handle(&mut self, events: Vec<Event>) {
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
                    let session = &self.session;
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

    /// A short message at the bottom of the window.
    fn toast(&self, title: &str) {
        self.widgets
            .toasts
            .add_toast(adw::Toast::builder().title(title).timeout(4).build());
    }
}

/// Closes the window after a screenshot and stops the application, so
/// nothing left open (a dialog) keeps it running.
fn close_for_good(window: &adw::ApplicationWindow) {
    let application = window.application();
    window.close();
    if let Some(application) = application {
        application.quit();
    }
}

/// A rendered image as a texture for GTK.
fn texture(image: crate::render::Image) -> Option<gdk::Texture> {
    let stride = usize::try_from(image.width).ok()?.checked_mul(4)?;
    let width = i32::try_from(image.width).ok()?;
    let height = i32::try_from(image.height).ok()?;
    let bytes = glib::Bytes::from_owned(image.rgba);
    // Alpha is 1 everywhere, so the bytes are premultiplied already: the
    // format GTK draws without converting.
    let texture = gdk::MemoryTexture::new(
        width,
        height,
        gdk::MemoryFormat::R8g8b8a8Premultiplied,
        &bytes,
        stride,
    );
    Some(texture.upcast())
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

/// The window's drawing, when it has drawn.
fn window_node(
    window: &adw::ApplicationWindow,
    paintable: &gtk::WidgetPaintable,
) -> Option<gtk::gsk::RenderNode> {
    let snapshot = gtk::Snapshot::new();
    // In device pixels (GDK_SCALE=2 doubles the picture), so a HiDPI
    // window's sharpness can be checked.
    let scale = window
        .native()
        .and_then(|native| native.surface())
        .map_or_else(
            || f64::from(window.scale_factor()),
            |surface| surface.scale(),
        );
    let scale = crate::convert::f32_of(scale);
    snapshot.scale(scale, scale);
    paintable.snapshot(
        &snapshot,
        f64::from(window.width()),
        f64::from(window.height()),
    );
    snapshot.to_node()
}

/// Renders the window's drawing into a PNG.
fn save_node(
    window: &adw::ApplicationWindow,
    node: &gtk::gsk::RenderNode,
    path: &Path,
) -> Result<()> {
    // A software renderer of our own: it works on every display backend.
    let renderer = gtk::gsk::CairoRenderer::new();
    renderer
        .realize_for_display(&gtk::prelude::WidgetExt::display(window))
        .context("can't start a renderer for the window")?;
    let texture = renderer.render_texture(node, None);
    renderer.unrealize();
    texture
        .save_to_png(path)
        .with_context(|| format!("can't write '{}'", path.display()))?;
    eprintln!("wrote {}", path.display());
    Ok(())
}

/// Screenshots look the same on every machine: GNOME's default font at
/// 96 dpi and the Adwaita icons, whatever the desktop's settings.
fn settle_for_screenshot() {
    if let Some(settings) = gtk::Settings::default() {
        settings.set_gtk_font_name(Some("Adwaita Sans 11"));
        settings.set_gtk_xft_dpi(96 * 1024);
        settings.set_gtk_icon_theme_name(Some("Adwaita"));
        settings.set_gtk_decoration_layout(Some(":close"));
        settings.set_gtk_enable_animations(false);
    }
}

/// The title bar's menus: Project, Edit, View and Help, each a submenu
/// (the menu bar shows them side by side; a narrow window's one menu
/// button lists them).
fn main_menu(recent: &gio::Menu) -> gio::Menu {
    /// A section of `(label, action)` items.
    fn section(items: &[(&str, &str)]) -> gio::Menu {
        let section = gio::Menu::new();
        for (label, action) in items {
            section.append(Some(label), Some(action));
        }
        section
    }
    let project = gio::Menu::new();
    let opening = section(&[
        ("New Project…", "win.new-project"),
        ("Open Project…", "win.open"),
    ]);
    opening.append_submenu(Some("Open Recent"), recent);
    project.append_section(None, &opening);
    project.append_section(
        None,
        &section(&[
            ("Save", "win.save"),
            ("Check Again", "win.check"),
            ("Build Plan", "win.build"),
        ]),
    );
    project.append_section(
        None,
        &section(&[("Export View as glTF…", "win.export-view")]),
    );
    let edit = gio::Menu::new();
    edit.append_section(
        None,
        &section(&[
            ("Find…", "win.find"),
            ("Find in the Text", "win.search"),
            ("Format the Text", "win.format"),
        ]),
    );
    edit.append_section(
        None,
        &section(&[
            ("Next Problem", "win.next-problem"),
            ("Previous Problem", "win.previous-problem"),
        ]),
    );
    let view = gio::Menu::new();
    view.append_section(
        None,
        &section(&[
            ("Design", "win.view-design"),
            ("Cut List", "win.view-cut-list"),
        ]),
    );
    view.append_section(
        None,
        &section(&[
            ("Back", "win.back"),
            ("Forward", "win.forward"),
            ("Frame Everything", "win.frame"),
        ]),
    );
    // Any panel can be closed; these bring it back where it was.
    let panels = gio::Menu::new();
    for id in workbench::PanelId::ALL {
        panels.append(
            Some(id.title()),
            Some(&format!("win.show-panel::{}", id.word())),
        );
    }
    panels.append(Some("Reset Layout"), Some("win.reset-layout"));
    view.append_section(Some("Panels"), &panels);
    view.append_section(Some("Style"), &app::style_section());
    let menu = gio::Menu::new();
    menu.append_submenu(Some("Project"), &project);
    menu.append_submenu(Some("Edit"), &edit);
    menu.append_submenu(Some("View"), &view);
    menu.append_submenu(Some("Help"), &app::help_section());
    menu
}

/// Builds a project window's widgets into `window` (the component's
/// root, made by [`Window::init_root`]), before the component is
/// launched: a failure leaves nothing to launch.
pub(in crate::ui) fn build_widgets(
    window: &adw::ApplicationWindow,
    application: &adw::Application,
    folder: &Path,
    recent: &gio::Menu,
    options: &Options,
) -> Result<Widgets> {
    if options.window_screenshot.is_some() {
        settle_for_screenshot();
    }
    let folder_name = app::folder_name(folder);
    let header = Header::new(&folder_name, &main_menu(recent));

    // The 3D view, its controls floating on it (`view_layout` keeps them
    // apart): the scene menu at the top left; the camera buttons, X-ray
    // and Pull apart down the top right, Pull apart's slider popping out
    // to the left of its button; a short line and the shared-wood key
    // along the top; the moving assemblies' chips at the bottom left and
    // the selection's motion slider at the bottom right; a joint's step
    // player, a problem's motion card and the off-screen chip above the
    // foot, in the middle; focus mode's From above / In 3D switch at the
    // top right.
    let viewport = Viewport::default();
    let motion = MotionBar::new();
    let view_toolbar = ViewToolbar::new();
    let camera = CameraToolbar::new(&view_toolbar);
    let contact_key = view_tools::contact_key();
    let plan_switch = PlanSwitch::new();
    let step_card = StepCard::new();
    let problem_card = ProblemCard::new();
    let dock = Dock::new(&header.scene, &motion);
    let hint = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .max_width_chars(48)
        .build();
    let hint_box = gtk::Box::builder()
        .css_classes(["stave-chip"])
        .visible(false)
        .build();
    hint_box.append(&hint);
    let edge_label = gtk::Label::new(None);
    edge_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    edge_label.set_max_width_chars(60);
    let edge_content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    edge_content.append(&gtk::Image::from_icon_name("go-down-symbolic"));
    edge_content.append(&edge_label);
    let edge_pin = gtk::Button::builder()
        .child(&edge_content)
        .css_classes(["stave-edge-pin"])
        .action_name("win.show-problem")
        .tooltip_text("Show the problem off screen")
        .visible(false)
        .build();
    let figure = gtk::Overlay::builder()
        .child(&viewport)
        .vexpand(true)
        .overflow(gtk::Overflow::Hidden)
        .css_classes(["stave-viewport", "stave-figure"])
        .build();
    // Hardware focus lays the view out as a page (Hinge.dc.html): a line
    // over it, three small views and the swing card under it.
    let caption = Caption::new();
    let pictures = Pictures::new();
    let swing_card = SwingCard::new();
    let stage = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .css_classes(["stave-stage"])
        .build();
    stage.append(&caption.root);
    stage.append(&figure);
    stage.append(&pictures.root);
    stage.append(&swing_card.root);
    let empty = view_tools::EmptyView::new();
    let view_stack = gtk::Stack::new();
    view_stack.add_named(&stage, Some("view"));
    view_stack.add_named(&empty.root, Some("status"));
    view_stack.set_visible_child_name("status");

    // The panels round the view: the parts tree, the files, the
    // selection's account, the open files and the problems.
    let tree = TreeSidebar::new();
    let inspector = Inspector::new();
    let details_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    details_box.append(&inspector.root);
    let details = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&details_box)
        .css_classes(["stave-details"])
        .build();
    // The Edit panel's content comes with the editing (`edit_view`).
    let edit_holder = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(true)
        .vexpand(true)
        .build();
    let files = FilesPage::new();
    let file_list = panels::FilesList::new();
    let problems = panels::ProblemsPanel::new();
    let output = output::OutputPanel::new();

    // The 3D View panel: the view, with the problems pinned on the model
    // (they come later) over it.
    let design = gtk::Overlay::builder()
        .child(&view_stack)
        .hexpand(true)
        .vexpand(true)
        .overflow(gtk::Overflow::Hidden)
        .build();
    let view_panel = design.clone();

    let workbench = workbench::Workbench::new(
        vec![
            (workbench::PanelId::View, view_panel.upcast()),
            (workbench::PanelId::Parts, tree.root.clone().upcast()),
            (workbench::PanelId::Files, file_list.root.clone().upcast()),
            (workbench::PanelId::Details, details.upcast()),
            (workbench::PanelId::Edit, edit_holder.clone().upcast()),
            (workbench::PanelId::Code, files.root.clone().upcast()),
            (workbench::PanelId::Problems, problems.root.clone().upcast()),
            (workbench::PanelId::Output, output.root.clone().upcast()),
        ],
        match &options.layout {
            Some(path) => workbench::Remember::From(path.clone()),
            None if options.window_screenshot.is_some() => workbench::Remember::Nothing,
            None => workbench::Remember::Yours,
        },
    )?;

    // The view's controls float on it; the tool groups dock where the
    // layout says, and a group dragged to a new spot is remembered with
    // the layout.
    let floating: Vec<(view_layout::Floating, gtk::Widget)> = vec![
        (view_layout::Floating::Camera, camera.root.clone().upcast()),
        (
            view_layout::Floating::PullSlider,
            view_toolbar.popout.clone().upcast(),
        ),
        (
            view_layout::Floating::PlanSwitch,
            plan_switch.root.clone().upcast(),
        ),
        (view_layout::Floating::Scene, dock.scene.clone().upcast()),
        (view_layout::Floating::Hint, hint_box.clone().upcast()),
        (
            view_layout::Floating::ContactKey,
            contact_key.clone().upcast(),
        ),
        (view_layout::Floating::Moves, dock.moves.clone().upcast()),
        (view_layout::Floating::Motion, motion.root.clone().upcast()),
        (
            view_layout::Floating::StepCard,
            step_card.root.clone().upcast(),
        ),
        (
            view_layout::Floating::ProblemCard,
            problem_card.root.clone().upcast(),
        ),
        (view_layout::Floating::EdgePin, edge_pin.clone().upcast()),
    ];
    // The groups are laid out for their spots, and follow the workbench
    // when it moves them, by the window's messages (`update/rendering.rs`).
    let floats = floating::Floats::new(&figure, floating, workbench.tool_spots());

    let views = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .vexpand(true)
        .build();
    views.add_named(&workbench.root, Some(View::Design.word()));
    views.add_named(&chrome::cut_list_page(), Some(View::CutList.word()));
    views.set_visible_child_name(View::Design.word());

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&views);
    // Over the whole window: the search over a setting's options.
    let sheet = gtk::Overlay::builder().child(&content).build();
    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&sheet));

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header.bar);
    toolbar.set_content(Some(&toasts));

    let (width, height) = options.size.unwrap_or((1200, 800));
    let width = i32::try_from(width).unwrap_or(1200);
    let height = i32::try_from(height).unwrap_or(800);
    window.set_application(Some(application));
    window.set_title(Some(&format!("{folder_name} — Stave")));
    window.set_default_size(width, height);
    window.set_content(Some(&toolbar));
    if options.window_screenshot.is_some() {
        // A screenshot is exactly the size asked for, whatever the
        // display's own size.
        window.set_size_request(width, height);
    }

    // Narrower windows drop the project's name first, then the switcher's
    // words, then fold the menu bar into one menu button. Only the last
    // breakpoint that matches applies, so each repeats the wider ones.
    // The side edges narrow too, so the Model keeps room.
    for (width, level) in [(1000.0, 0), (860.0, 1), (760.0, 2), (600.0, 3)] {
        let narrow = breakpoint(width);
        workbench.narrow_edges(&narrow);
        if level >= 1 {
            narrow.add_setter(&header.title, "visible", Some(&false.to_value()));
        }
        if level >= 2 {
            for label in header.switcher_labels() {
                narrow.add_setter(label, "visible", Some(&false.to_value()));
            }
        }
        if level >= 3 {
            narrow.add_setter(&header.menu_bar, "visible", Some(&false.to_value()));
            narrow.add_setter(&header.menu_button, "visible", Some(&true.to_value()));
            narrow.add_setter(
                &header.switcher,
                "css-classes",
                Some(&["stave-switcher", "compact"].to_value()),
            );
        }
        window.add_breakpoint(narrow);
    }

    let finder = Finder::new();

    Ok(Widgets {
        window: window.clone(),
        header,
        toasts,
        views,
        design,
        tree,
        inspector,
        details: details_box,
        edit_holder,
        sheet,
        dock,
        workbench,
        problems,
        file_list,
        hint,
        hint_box,
        edge_pin,
        edge_label,
        finder,
        view_stack,
        empty,
        viewport,
        motion,
        stage,
        view_toolbar,
        camera,
        contact_key,
        plan_switch,
        step_card,
        swing_card,
        caption,
        pictures,
        problem_card,
        files,
        output,
        floats,
    })
}

/// A breakpoint for windows at most `width` sp wide.
fn breakpoint(width: f64) -> adw::Breakpoint {
    adw::Breakpoint::new(adw::BreakpointCondition::new_length(
        adw::BreakpointConditionLengthType::MaxWidth,
        width,
        adw::LengthUnit::Sp,
    ))
}

/// A control's `f64` value as the `f32` the scene works in.
fn narrow(value: f64) -> f32 {
    crate::convert::f32_of(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::gtk_test::{on_gtk_thread, settle};

    #[test]
    fn a_closed_window_says_so_and_is_freed_once_its_controller_goes() {
        on_gtk_thread(closed_window_is_freed);
    }

    /// The application owns a window's controller while it is open: it
    /// lives while held, closing takes it out of the application (which
    /// is when the application drops its controller), and dropping the
    /// controller then frees the window, its model and its runtime (the
    /// 960f06d and 0603e47 bugs: a closed window kept alive, an open one
    /// freed).
    fn closed_window_is_freed() {
        let application = adw::Application::new(
            Some("org.stave.WindowTest.Freed"),
            gio::ApplicationFlags::NON_UNIQUE,
        );
        // A window joins only an application that has started.
        if let Err(error) = application.register(gio::Cancellable::NONE) {
            panic!("the test's application registers: {error}");
        }
        let folder = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/wall_shelf"
        ))
        .canonicalize()
        .unwrap();
        // A layout of its own, which is never written: the people's own
        // layout is left alone.
        let options = Options {
            layout: Some(std::env::temp_dir().join("stave-window-test-no-layout")),
            ..Options::default()
        };
        let builder = Window::builder();
        let root = builder.widget().clone();
        let widgets =
            build_widgets(&root, &application, &folder, &gio::Menu::new(), &options).unwrap();
        let closed = Rc::new(Cell::new(false));
        let heard = Rc::clone(&closed);
        // What the application listens for to drop the controller.
        let removed = application.connect_window_removed(move |_, _| heard.set(true));
        let controller = builder
            .launch(WindowInit {
                widgets,
                folder,
                options,
            })
            .detach();
        let window = root.downgrade();
        drop(root);
        let input = controller.sender().clone();
        settle();
        assert!(window.upgrade().is_some(), "the window lives while open");
        assert!(!closed.get());
        controller.widget().close();
        settle();
        assert!(closed.get(), "the window has left the application");
        application.disconnect(removed);
        drop(controller);
        settle();
        assert!(window.upgrade().is_none(), "the window is freed");
        assert!(
            input
                .send(WindowInput::Rendering(
                    update::rendering::RenderingInput::Frame
                ))
                .is_err(),
            "the window's runtime, and with it its model, is gone"
        );
    }
}
