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
mod slider;
mod style;
mod tab_strip;
mod tree;
mod view_layout;
mod view_tools;
mod viewport;
mod workbench;

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use anyhow::{Context as _, Result, bail};
use gtk::{gdk, gio, glib};
use stave_app::Contact;
use stave_app::check::{self, Checked, Watcher};
use stave_app::finder::{self as finding, Goal};
use stave_app::focus::{self, Focus, Target};
use stave_app::inspect;
use stave_app::link;
use stave_app::problems::{Mark, Marks};
use stave_app::session::{
    Build, Command, Editor, Event, Explode, LineNumber, ProblemIndex, Selection, Session,
    SourceLocation, View,
};
use stave_app::view::{self, Highlight};
use stave_model::solved::AssemblyId;
use stave_scene::{Look, NodeId, Orbit, Overlay, Posed};

use crate::render::{Backdrop, Frame, Renderer};
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

/// The most problems pinned on the model at once.
const PIN_LIMIT: usize = 12;

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
pub struct Options {
    /// The project folder.
    pub folder: PathBuf,
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
    /// Light, dark or the system's.
    pub theme: Theme,
    /// Save the window to this PNG once drawn, then close.
    pub window_screenshot: Option<PathBuf>,
    /// The window's size, in logical pixels.
    pub size: (u32, u32),
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

/// Opens the window and runs until it is closed.
pub fn run(options: Options) -> Result<()> {
    let folder = options
        .folder
        .canonicalize()
        .with_context(|| format!("can't find the folder '{}'", options.folder.display()))?;
    if !folder.is_dir() {
        bail!("'{}' is not a folder", folder.display());
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
    let application = adw::Application::builder()
        .application_id("org.stave.Viewer")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    let options = Rc::new(RefCell::new(Some(Options { folder, ..options })));
    application.connect_activate(move |application| {
        if let Some(options) = options.borrow_mut().take()
            && let Err(error) = Window::open(application, options)
        {
            eprintln!("error: {error:#}");
            application.quit();
        }
    });
    let status = application.run_with_args::<&str>(&[]);
    if status != glib::ExitCode::SUCCESS {
        bail!("the viewer stopped with an error");
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

/// A problem to pin on the model, and where it shows (logical pixels in
/// the 3D view; `None` behind the camera).
struct Pin {
    index: ProblemIndex,
    mark: Mark,
    message: String,
    point: Option<(f32, f32)>,
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
    /// The latest check's marks, when its ids are the shown design's.
    marks: Marks,
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

impl Widgets {
    /// Offers the view's everyday controls (the scene menu, X-ray, Pull
    /// apart and the moving assemblies' chips), or takes them away while
    /// a problem is posed or a joint is in focus.
    fn offer_view_tools(&self, offered: bool) {
        self.dock.offer(offered);
        self.view_toolbar.offer(offered);
    }
}

struct Window {
    widgets: Widgets,
    session: RefCell<Session>,
    state: RefCell<State>,
    /// The problems pinned on the model, as overlay children of the
    /// design (made again whenever the view changes).
    pins: RefCell<Vec<gtk::Widget>>,
    /// Kept alive for as long as the window: dropping it stops watching.
    watcher: RefCell<Option<Watcher>>,
    /// Editing in the 3D view: the menu, the card and the ghost.
    editing: edit_view::EditView,
}

impl Window {
    fn open(application: &adw::Application, options: Options) -> Result<()> {
        if options.window_screenshot.is_some() {
            settle_for_screenshot();
        }
        adw::StyleManager::default().set_color_scheme(options.theme.scheme());
        if let Some(display) = gdk::Display::default() {
            style::install(&display);
        }
        let widgets = build_widgets(application, &options)?;
        let renderer = Renderer::new().map_err(|error| format!("{error:#}"));
        let window = Rc::new(Window {
            widgets,
            session: RefCell::new(Session::new(options.folder.clone())),
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
                marks: Marks::default(),
                edit_script: None,
            }),
            pins: RefCell::new(Vec::new()),
            watcher: RefCell::new(None),
            editing: edit_view::EditView::new(),
        });
        window.install_actions(options.theme);
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
        window.widgets.window.present();
        window.show_folder();
        window.dispatch(Command::Check);
        Ok(())
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
        glib::spawn_future_local(async move {
            let outcome = gio::spawn_blocking(move || check::run_with(&folder, &overlay))
                .await
                .unwrap_or_else(|_| check::Outcome::CantOpen {
                    message: "checking the design crashed".to_owned(),
                    elapsed: Duration::ZERO,
                });
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
        widgets.files.show_result(latest.as_deref());
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
        let marks = latest
            .as_deref()
            .map(stave_app::problems::marks)
            .unwrap_or_default();
        widgets.files.show_marks(marks.clone());
        let tree_marks = if showing_older {
            Marks::default()
        } else {
            marks
        };
        self.state.borrow_mut().marks = tree_marks.clone();
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
        widgets.tree.show_tree(inspect::parts_tree(design));
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
            let folder = self.session.borrow().folder().to_path_buf();
            let (path, line) = match file.rsplit_once(':') {
                Some((path, line)) => match line.parse::<u32>() {
                    Ok(line) => (path, line),
                    Err(_) => (file.as_str(), 1),
                },
                None => (file.as_str(), 1),
            };
            let path = if path.starts_with("std/") {
                path.to_owned()
            } else {
                folder.join(path).display().to_string()
            };
            if let Some(line) = LineNumber::new(line) {
                self.dispatch(Command::GoToSource(SourceLocation { path, line }));
            }
        }
        if wanted.wish != Wish::default() {
            self.widgets.files.wish(wanted.wish.clone());
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

    fn view_changed(self: &Rc<Self>) {
        let view = self.session.borrow().view();
        self.widgets.header.show_view(view);
        self.widgets.views.set_visible_child_name(view.word());
        self.sync_files();
        self.request_render();
    }

    /// `true` when the 3D view is on screen: the design.
    fn shows_3d(&self) -> bool {
        match self.session.borrow().view() {
            View::Design => true,
            View::CutList => false,
        }
    }

    /// The editor opened or closed. Opening shows the Code panel wherever
    /// it is docked, with the selection's line; closing gives the keyboard
    /// back to the 3D view (the panel stays where it is).
    fn editor_changed(self: &Rc<Self>) {
        let open = self.session.borrow().editor() == Editor::Open;
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
    fn sync_files(self: &Rc<Self>) {
        let (open, selection, placed) = {
            let session = self.session.borrow();
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
            (Some(_), Some(placed)) if !self.state.borrow().linking => {
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
    fn text_linked(self: &Rc<Self>, found: files::CursorName) {
        let (analysis, symbol) = match found {
            files::CursorName::Name(analysis, symbol) => (analysis, symbol),
            // A broken name: the 3D view lets go, and the chip goes.
            files::CursorName::Unresolved => {
                let linked =
                    self.session.borrow().selection().is_some_and(|s| {
                        !matches!(s, Selection::Location(_) | Selection::Problem(_))
                    });
                if linked {
                    self.state.borrow_mut().linking = true;
                    self.dispatch(Command::Link(None));
                    self.state.borrow_mut().linking = false;
                }
                return;
            }
            files::CursorName::Other => return,
        };
        let selection = {
            let session = self.session.borrow();
            session.drawable().and_then(|(design, _)| {
                link::selection_for(design, &analysis, symbol, session.selection())
            })
        };
        if let Some(selection) = selection {
            self.state.borrow_mut().linking = true;
            self.dispatch(Command::Link(Some(selection)));
            self.state.borrow_mut().linking = false;
        }
    }

    fn selection_changed(self: &Rc<Self>) {
        let (selection, inspection, problem, joints, focused) = {
            let session = self.session.borrow();
            let selection = session.selection().cloned();
            let inspection = session
                .shown()
                .zip(selection.as_ref())
                .and_then(|(shown, selection)| inspect::inspect(shown, selection));
            let problem = session.selected_problem().cloned();
            let details = if let (Some(Selection::Problem(index)), Some(latest)) =
                (&selection, session.latest())
            {
                stave_app::problems::details(latest, *index)
            } else {
                None
            };
            // The joints a selected part is in, listed under it in the
            // tree.
            let joints = selection
                .as_ref()
                .and_then(Selection::item)
                .zip(session.drawable())
                .map(|(item, (design, _))| focus::targets_of(design, item))
                .unwrap_or_default();
            (
                selection,
                inspection,
                problem.zip(details),
                joints,
                session.focus().is_some(),
            )
        };
        let widgets = &self.widgets;
        let linked = {
            let session = self.session.borrow();
            session
                .drawable()
                .zip(selection.as_ref())
                .and_then(|((design, _), selection)| {
                    let placed = link::placed_at(session.shown()?, selection)?;
                    link::chip(design, selection, &placed)
                })
        };
        widgets
            .workbench
            .status
            .set_label(linked.as_deref().unwrap_or(""));
        widgets.tree.show_selection(selection.as_ref(), joints);
        widgets.problems.show_selected(match &selection {
            Some(Selection::Problem(index)) => Some(index.0),
            _ => None,
        });
        self.sync_files();
        if focused {
            self.show_card();
            return;
        }
        let explode = self.session.borrow().explode().get();
        self.show_hint(&if explode > 0.0 && selection.is_none() {
            format!("Pulled apart · {}%", view_tools::percent(explode))
        } else {
            String::new()
        });
        // A problem is read in the Problems panel; Details keeps to the
        // design.
        widgets.inspector.show(inspection.as_ref());
        // Pose a problem where it happens, or offer to open the selection.
        match problem.and_then(|(problem, details)| Some((problem.contact?, details.motion))) {
            Some((contact, card)) => self.pose_problem(contact, card),
            None => {
                self.state.borrow_mut().problem = None;
                self.show_problem_controls(None);
                let moves_with = match &selection {
                    Some(Selection::Assembly(id)) => Some(*id),
                    _ => inspection.and_then(|i| i.moves_with),
                };
                self.update_opening(moves_with);
            }
        }
        self.show_card();
        self.show_moves();
        self.grips_follow_selection();
        self.request_render();
    }

    /// The Details panel always shows the selection's account; focus mode
    /// brings it forward, since its controls are there.
    fn show_card(self: &Rc<Self>) {
        if self.session.borrow().focus().is_some() {
            self.widgets.workbench.present(workbench::PanelId::Details);
        }
        self.update_face_button();
    }

    /// A short line at the top of the 3D view ("Focused on one joint · Esc
    /// to go back"); empty hides it.
    fn show_hint(&self, text: &str) {
        self.widgets.hint.set_label(text);
        self.widgets.hint_box.set_visible(!text.is_empty());
    }

    /// The dock's chips: every assembly that moves, with its mark, the
    /// one the motion slider shows marked current.
    fn show_moves(self: &Rc<Self>) {
        let (moving, current, sliding) = {
            let session = self.session.borrow();
            let state = self.state.borrow();
            let moving: Vec<(AssemblyId, String, Option<Mark>)> = session
                .drawable()
                .map(|(design, graph)| {
                    design
                        .assemblies
                        .iter()
                        .filter(|a| a.parent.is_none() && graph.motion_range(a.id).is_some())
                        .map(|a| (a.id, a.name.clone(), state.marks.assembly(a.id)))
                        .collect()
                })
                .unwrap_or_default();
            (
                moving,
                state.opening.map(|o| o.assembly),
                self.widgets.motion.root.is_visible(),
            )
        };
        // While the selection's own slider shows, the chips step aside
        // (V2C_Spatial.dc.html): there isn't room for both.
        self.widgets
            .dock
            .show_moves(if sliding { &[] } else { &moving }, current);
    }

    // ---- focus mode ----

    /// Opens or leaves focus mode to match the session.
    fn focus_changed(self: &Rc<Self>) {
        let wanted = self.session.borrow().focus();
        let current = self.state.borrow().focus.as_ref().map(|f| f.focus.target);
        if wanted == current {
            return;
        }
        match wanted {
            Some(target) => self.open_focus(target),
            None => self.leave_focus(),
        }
    }

    fn open_focus(self: &Rc<Self>, target: Target) {
        let found = {
            let session = self.session.borrow();
            session.shown().and_then(|shown| {
                let (design, graph) = shown.drawable()?;
                let focus = focus::open(&shown.opened, design, graph, target)?;
                let name = focus
                    .motion()
                    .and_then(|(assembly, _)| design.assemblies.get(assembly.index()))
                    .map(|assembly| assembly.name.clone());
                Some((focus, name))
            })
        };
        let Some((found, moving_name)) = found else {
            return;
        };
        let start = self.state.borrow_mut().focus_start.take();
        let (start_plan, start_step) = start.unwrap_or((None, None));
        let progress = start_step.unwrap_or(if found.assembles() { 0.0 } else { 0.4 });
        let plan = found.plan().is_some() && start_plan.unwrap_or(found.prefers_plan());
        {
            let mut state = self.state.borrow_mut();
            state.problem = None;
            state.opening = None;
            state.exploding = false;
            state.focus = Some(FocusState {
                focus: found,
                progress,
                options: focus::Options::default(),
                plan,
                playing: false,
                looks: Vec::new(),
                overlay: Overlay::default(),
            });
        }
        self.widgets.motion.show(None);
        self.show_problem_controls(None);
        self.layout_for_focus(moving_name.as_deref());
        self.hide_pins();
        let texts = self.repose();
        self.frame_focus();
        {
            let state = self.state.borrow();
            self.widgets
                .inspector
                .focus
                .show(state.focus.as_ref().map(|f| &f.focus));
        }
        // Setting the check boxes calls their handlers, which look at the
        // state: nothing may be borrowed here.
        self.state.borrow_mut().updating_controls = true;
        let panel = &self.widgets.inspector.focus;
        panel.xray.set_active(true);
        panel.tint.set_active(true);
        panel.fade.set_active(true);
        self.widgets.plan_switch.show(plan);
        self.state.borrow_mut().updating_controls = false;
        self.widgets.inspector.show_focus();
        let assembles = self
            .state
            .borrow()
            .focus
            .as_ref()
            .is_some_and(|f| f.focus.assembles());
        self.show_hint(if assembles {
            "Focused on one joint · Esc to go back"
        } else {
            "Focused on one piece of hardware · Esc to go back"
        });
        self.show_focus_texts(texts);
        self.show_progress();
        self.render_pictures();
        self.request_render();
    }

    /// Frames the joint or the hardware (from the direction focus asks
    /// for).
    fn frame_focus(&self) {
        let mut state = self.state.borrow_mut();
        let aspect = Self::aspect(&state);
        let framed = state
            .focus
            .as_ref()
            .and_then(|f| f.focus.zone())
            .zip(state.orbit);
        let view = state.focus.as_ref().and_then(|f| f.focus.view());
        if let Some((zone, orbit)) = framed {
            let orbit = match view {
                Some((yaw, pitch)) => Orbit {
                    yaw,
                    pitch,
                    ..orbit
                },
                None => orbit,
            };
            state.orbit = Some(orbit.framed(zone, aspect));
        }
    }

    /// Lays the 3D view's area out for focus mode (`Some` focus) or for
    /// the Parts view: a joint gets its step player over the view; hardware
    /// gets a page with a line over the view, three small views and the
    /// swing card.
    fn layout_for_focus(&self, moving_name: Option<&str>) {
        let state = self.state.borrow();
        let focus = state.focus.as_ref();
        let widgets = &self.widgets;
        let joint = focus.is_some_and(|f| f.focus.assembles());
        let hardware = focus.filter(|f| !f.focus.assembles());
        let page = hardware.is_some();
        if page {
            widgets.stage.add_css_class("page");
        } else {
            widgets.stage.remove_css_class("page");
        }
        widgets.step_card.root.set_visible(joint);
        let swing = hardware.and_then(|f| f.focus.motion());
        widgets.swing_card.root.set_visible(swing.is_some());
        if let Some((_, range)) = swing {
            widgets
                .swing_card
                .show_name(moving_name.unwrap_or("it"), range.turns);
        }
        widgets.caption.root.set_visible(page);
        widgets.pictures.root.set_visible(false);
        widgets
            .plan_switch
            .root
            .set_visible(focus.is_some_and(|f| f.focus.plan().is_some()));
        widgets.camera.root.set_visible(focus.is_none());
        widgets.offer_view_tools(focus.is_none());
        drop(state);
        self.show_caption();
    }

    /// The line over hardware focus's view: from above or in 3D.
    fn show_caption(&self) {
        let state = self.state.borrow();
        let Some(focus) = state.focus.as_ref() else {
            return;
        };
        let (text, hint) = match focus.focus.plan_caption().filter(|_| focus.plan) {
            Some(caption) => (caption.to_owned(), "back ↑ · front ↓"),
            None => (
                format!("In 3D around {}", focus.focus.title),
                "drag to turn · scroll to zoom",
            ),
        };
        self.widgets.caption.show(&text, hint);
    }

    fn leave_focus(self: &Rc<Self>) {
        self.state.borrow_mut().focus = None;
        self.widgets.inspector.focus.show(None);
        self.layout_for_focus(None);
        self.show_hint("");
        self.repose();
        self.selection_changed();
    }

    fn show_focus_texts(&self, texts: Option<(Option<String>, String)>) {
        if let Some((step, status)) = texts {
            self.widgets.step_card.show_frame(step.as_deref(), &status);
        }
    }

    /// Moves the step player's and the swing card's sliders to the
    /// progress, with the swing's reading and the small view nearest it
    /// ringed.
    fn show_progress(&self) {
        let state = self.state.borrow();
        let Some(focus) = state.focus.as_ref() else {
            return;
        };
        let progress = f64::from(focus.progress);
        let widgets = &self.widgets;
        widgets.step_card.scrub.set_value(progress);
        widgets.swing_card.scrub.set_value(progress);
        focus_stage::show_playing(
            &widgets.step_card.play,
            &widgets.step_card.play_icon,
            focus.playing,
            "Play the assembly",
        );
        focus_stage::show_playing(
            &widgets.swing_card.play,
            &widgets.swing_card.play_icon,
            focus.playing,
            "Play the swing",
        );
        if let Some((_, range)) = focus.focus.motion() {
            widgets
                .swing_card
                .show_value(&stave_scene::focus::amount_text(
                    range,
                    focus.progress * range.full,
                ));
        }
        let nearest = focus
            .focus
            .key_poses
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                (a.progress - focus.progress)
                    .abs()
                    .total_cmp(&(b.progress - focus.progress).abs())
            })
            .map(|(index, _)| index);
        widgets.pictures.show_current(nearest);
    }

    /// The scrub moved, or a toggle changed: poses again.
    fn update_focus(self: &Rc<Self>, change: impl FnOnce(&mut FocusState)) {
        {
            // Busy: the window itself is setting the controls.
            let Ok(mut state) = self.state.try_borrow_mut() else {
                return;
            };
            if state.updating_controls {
                return;
            }
            let Some(focus) = state.focus.as_mut() else {
                return;
            };
            change(focus);
        }
        let texts = self.repose();
        self.show_focus_texts(texts);
        self.show_progress();
        self.show_caption();
        self.request_render();
    }

    /// The play button: plays from where it is (from the start when it is
    /// at the end), or pauses.
    fn toggle_play_focus(self: &Rc<Self>) {
        let playing = {
            let Ok(mut state) = self.state.try_borrow_mut() else {
                return;
            };
            let Some(focus) = state.focus.as_mut() else {
                return;
            };
            focus.playing = !focus.playing;
            if focus.playing && focus.progress >= 1.0 {
                focus.progress = 0.0;
            }
            focus.playing
        };
        self.show_progress();
        if !playing {
            return;
        }
        let window = self.clone();
        let last: std::cell::Cell<Option<i64>> = std::cell::Cell::new(None);
        self.widgets.viewport.add_tick_callback(move |_, clock| {
            let now = clock.frame_time();
            let seconds = last.get().map_or(0.0, |before| {
                f64::from(i32::try_from(now - before).unwrap_or(0)) / 1e6
            });
            last.set(Some(now));
            let done = {
                let mut state = window.state.borrow_mut();
                let Some(focus) = state.focus.as_mut() else {
                    return glib::ControlFlow::Break;
                };
                if !focus.playing {
                    return glib::ControlFlow::Break;
                }
                focus.progress = (focus.progress + narrow(seconds * PLAY_PER_SECOND)).min(1.0);
                let done = focus.progress >= 1.0;
                if done {
                    focus.playing = false;
                }
                done
            };
            let texts = window.repose();
            window.show_focus_texts(texts);
            window.show_progress();
            window.request_render();
            if done {
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }

    /// Renders hardware focus's small views again shortly, once their
    /// size has stopped changing.
    fn schedule_pictures(self: &Rc<Self>) {
        let window = Rc::downgrade(self);
        let timer = glib::timeout_add_local_once(PICTURES_DELAY, move || {
            if let Some(window) = window.upgrade() {
                window.state.borrow_mut().pictures_timer = None;
                window.render_pictures();
            }
        });
        if let Some(old) = self.state.borrow_mut().pictures_timer.replace(timer) {
            old.remove();
        }
    }

    /// Renders hardware focus's small views offscreen, at fixed angles
    /// (closed, part way, fully open), in the view from above when there
    /// is one, each at its slot's size in device pixels. Before the slots
    /// have room they are only laid out: their first size renders them.
    fn render_pictures(self: &Rc<Self>) {
        let captions: Vec<String> = {
            let state = self.state.borrow();
            match state.focus.as_ref() {
                Some(focus) if !focus.focus.assembles() => focus
                    .focus
                    .key_poses
                    .iter()
                    .map(|pose| pose.caption.clone())
                    .collect(),
                Some(_) | None => Vec::new(),
            }
        };
        let window = Rc::downgrade(self);
        self.widgets.pictures.root.set_visible(!captions.is_empty());
        self.widgets.pictures.show(&captions, move |index| {
            let Some(window) = window.upgrade() else {
                return;
            };
            let progress = window
                .state
                .borrow()
                .focus
                .as_ref()
                .and_then(|f| f.focus.key_poses.get(index).map(|pose| pose.progress));
            if let Some(progress) = progress {
                window.update_focus(|focus| {
                    focus.progress = progress;
                    focus.playing = false;
                });
            }
        });
        let Some((width, height)) = self.widgets.pictures.device_size() else {
            return;
        };
        let backdrop = self.backdrop();
        let textures = {
            let session = self.session.borrow();
            let (Some((_, graph)), Some(entry)) = (session.drawable(), session.scene_entry())
            else {
                return;
            };
            let mut state = self.state.borrow_mut();
            let State {
                focus,
                renderer,
                orbit,
                ..
            } = &mut *state;
            let (Some(focus), Ok(renderer), Some(orbit)) = (focus.as_ref(), renderer, *orbit)
            else {
                return;
            };
            if focus.focus.assembles() {
                return;
            }
            let mut textures = Vec::new();
            for pose in &focus.focus.key_poses {
                let frame = focus
                    .focus
                    .frame(graph, entry, pose.progress, &focus.options);
                let aspect = crate::render::aspect(width, height);
                let (orbit, section) = match focus.focus.plan() {
                    Some(plan) => (
                        Orbit::plan(plan.bounds, aspect, plan.yaw),
                        Some(plan.section),
                    ),
                    None => (
                        focus
                            .focus
                            .zone()
                            .map_or(orbit, |zone| orbit.framed(zone, aspect)),
                        entry.section,
                    ),
                };
                let image = renderer.render(&Frame {
                    scene: graph,
                    posed: &frame.posed,
                    orbit,
                    selection: &[],
                    highlight: None,
                    section,
                    looks: Some(&frame.looks),
                    overlay: None,
                    lit: None,
                    tints: None,
                    tint_cut_faces: focus.options.tint_cut_faces,
                    backdrop,
                    width,
                    height,
                });
                let Ok(image) = image else { continue };
                if let Some(texture) = texture(image) {
                    textures.push(texture);
                }
            }
            textures
        };
        self.widgets.pictures.show_textures(textures);
        self.show_progress();
    }

    // ---- the 3D view ----

    fn aspect(state: &State) -> f32 {
        crate::render::aspect(state.size.0, state.size.1)
    }

    /// A camera that shows everything shown.
    fn frame_all(state: &State, shown: &Checked) -> Option<Orbit> {
        let (_, graph) = shown.drawable()?;
        let bounds = edit_view::drawn_bounds(graph, state.posed.as_ref()?)?;
        let aspect = Self::aspect(state);
        Some(match state.orbit {
            Some(orbit) => orbit.framed(bounds, aspect),
            None => Orbit::framing(bounds, aspect),
        })
    }

    /// The node drawn as selected.
    fn selected_node(&self) -> Option<NodeId> {
        let session = self.session.borrow();
        let item = session.selection()?.item()?;
        let (_, graph) = session.drawable()?;
        graph.nodes_of(item).into_iter().next()
    }

    /// Every node to draw selected: the selected item's, or those of every
    /// part and piece of hardware inside a selected assembly.
    fn selected_nodes(&self) -> Vec<NodeId> {
        let session = self.session.borrow();
        let Some((design, graph)) = session.drawable() else {
            return Vec::new();
        };
        let items: Vec<stave_scene::Item> = match session.selection() {
            Some(Selection::Assembly(id)) => {
                let inside = |mut assembly: Option<AssemblyId>| {
                    for _ in 0..=design.assemblies.len() {
                        match assembly {
                            Some(found) if found == *id => return true,
                            Some(found) => {
                                assembly =
                                    design.assemblies.get(found.index()).and_then(|a| a.parent);
                            }
                            None => return false,
                        }
                    }
                    false
                };
                design
                    .parts
                    .iter()
                    .filter(|p| inside(p.assembly))
                    .map(|p| stave_scene::Item::Part(p.id))
                    .chain(
                        design
                            .hardware
                            .iter()
                            .filter(|h| inside(h.assembly))
                            .map(|h| stave_scene::Item::Hardware(h.id)),
                    )
                    .collect()
            }
            Some(selection) => selection.item().into_iter().collect(),
            None => Vec::new(),
        };
        items
            .into_iter()
            .flat_map(|item| graph.nodes_of(item).into_iter().take(1))
            .collect()
    }

    /// F: frames the selection (every part of a selected assembly), or
    /// everything.
    fn frame(self: &Rc<Self>) {
        let nodes = self.selected_nodes();
        {
            let session = self.session.borrow();
            let Some(shown) = session.shown() else {
                return;
            };
            let mut state = self.state.borrow_mut();
            let aspect = Self::aspect(&state);
            let selected = (!nodes.is_empty())
                .then(|| {
                    let (_, graph) = shown.drawable()?;
                    let bounds = graph.bounds_of(state.posed.as_ref()?, &nodes)?;
                    let framed = state.orbit?.framed(bounds, aspect);
                    Some(Orbit {
                        distance: framed.distance * FRAME_ROOM,
                        ..framed
                    })
                })
                .flatten();
            state.orbit = selected.or_else(|| Self::frame_all(&state, shown));
        }
        self.request_render();
    }

    /// Choosing something frames it at once: a part, a piece of hardware
    /// or an assembly, or everything again when nothing is chosen. A
    /// problem frames itself where it is posed; focus mode frames its
    /// joint; a place in the text leaves the camera alone.
    fn frame_selection(self: &Rc<Self>) {
        let reframes = {
            let session = self.session.borrow();
            session.focus().is_none()
                && matches!(
                    session.selection(),
                    None | Some(
                        Selection::Part(_) | Selection::Hardware(_) | Selection::Assembly(_)
                    )
                )
        };
        if reframes {
            self.frame();
        }
    }

    /// Moves the camera where the command line asked, once the view has a
    /// size and the asked-for selection is made.
    fn apply_start_camera(self: &Rc<Self>) {
        let ready = {
            let state = self.state.borrow();
            state.size.0 > 1 && state.wanted.is_none() && state.start_camera.is_some()
        };
        if !ready {
            return;
        }
        let node = self.selected_node();
        {
            let session = self.session.borrow();
            let mut state = self.state.borrow_mut();
            let Some(camera) = state.start_camera.take() else {
                return;
            };
            let aspect = Self::aspect(&state);
            let Some(orbit) = state.orbit else {
                return;
            };
            let turned = Orbit {
                yaw: (orbit.yaw + camera.turn.to_radians()).rem_euclid(std::f32::consts::TAU),
                pitch: camera.pitch.map_or(orbit.pitch, f32::to_radians),
                ..orbit
            };
            let posed = state.posed.as_ref();
            let drawable = session.drawable();
            let bounds = drawable.and_then(|(_, graph)| {
                let posed = posed?;
                match node.filter(|_| camera.frame_selection) {
                    Some(id) => graph.bounds_of(posed, &[id]),
                    None => edit_view::drawn_bounds(graph, posed),
                }
            });
            let framed = bounds.map_or(turned, |bounds| turned.framed(bounds, aspect));
            let zoomed = Orbit {
                distance: framed.distance / camera.zoom.unwrap_or(1.0),
                ..framed
            };
            state.orbit = Some(zoomed);
        }
        self.request_render();
    }

    /// Offers the motion bar to open the selection's door or drawer.
    fn update_opening(self: &Rc<Self>, assembly: Option<AssemblyId>) {
        let found = {
            let session = self.session.borrow();
            assembly.and_then(|id| {
                let (design, graph) = session.drawable()?;
                let range = graph.motion_range(id)?;
                let name = design.assemblies.get(id.index())?.name.clone();
                Some((id, range, name))
            })
        };
        let amount = {
            let mut state = self.state.borrow_mut();
            if state.focus.is_some() {
                return;
            }
            let keep = state
                .opening
                .filter(|o| Some(o.assembly) == found.as_ref().map(|(id, _, _)| *id));
            state.opening = keep.or_else(|| {
                found.as_ref().map(|(id, range, _)| Opening {
                    assembly: *id,
                    amount: 0.0,
                    turns: range.turns,
                })
            });
            state.updating_controls = true;
            state.opening.map_or(0.0, |o| o.amount)
        };
        match &found {
            Some((_, range, name)) => {
                let (full, value, end) = if range.turns {
                    let degrees = range.full.to_degrees();
                    (
                        degrees,
                        amount.to_degrees(),
                        format!("{}°", degrees.round()),
                    )
                } else {
                    (range.full, amount, format!("{} mm", range.full.round()))
                };
                self.widgets.motion.show(Some((
                    name,
                    "closed",
                    &end,
                    f64::from(full),
                    f64::from(value),
                )));
            }
            None => self.widgets.motion.show(None),
        }
        self.state.borrow_mut().updating_controls = false;
        self.repose();
    }

    /// The motion bar moved: the selection opening.
    fn motion_moved(self: &Rc<Self>, value: f64) {
        {
            let Ok(mut state) = self.state.try_borrow_mut() else {
                return;
            };
            if state.updating_controls {
                return;
            }
            let value = narrow(value);
            if let Some(opening) = state.opening.as_mut() {
                opening.amount = if opening.turns {
                    value.to_radians()
                } else {
                    value
                };
            }
        }
        self.repose();
        self.request_render();
    }

    /// The problem card's slider moved: swings the problem's motion.
    fn problem_moved(self: &Rc<Self>, value: f64) {
        let shown = {
            let Ok(mut state) = self.state.try_borrow_mut() else {
                return;
            };
            let Some(problem) = state.problem.as_mut() else {
                return;
            };
            let value = narrow(value);
            let amount = if problem.turns {
                value.to_radians()
            } else {
                value
            };
            problem.amount = Some(amount);
            problem.card.clone().map(|card| (card, amount))
        };
        if let Some((card, amount)) = &shown {
            self.widgets.problem_card.show_value(card, *amount);
            self.show_posed_status(card, *amount);
        }
        self.repose();
        self.request_render();
    }

    /// "Posed at contact · door at 0.6°" at the top of the view.
    fn show_posed_status(&self, card: &MotionCard, amount: f32) {
        self.show_hint(&format!(
            "Posed at contact · {} at {}",
            card.name,
            card.amount_text(amount)
        ));
    }

    /// Shows the controls for a posed problem (its motion card and the
    /// shared-wood key in place of the view toolbar), or the usual ones.
    fn show_problem_controls(&self, problem: Option<(&MotionCard, f32, bool)>) {
        let widgets = &self.widgets;
        match problem {
            Some((card, amount, _)) => widgets.problem_card.show(Some(card), amount),
            None => widgets.problem_card.show(None, 0.0),
        }
        let posed = problem.is_some();
        let shared = problem.is_some_and(|(_, _, shared)| shared);
        let focused = self.state.borrow().focus.is_some();
        widgets.contact_key.set_visible(shared && !focused);
        widgets.offer_view_tools(!posed && !focused);
        if !posed && !focused {
            self.show_hint("");
        }
    }

    /// Poses the design where a problem happens and frames its parts.
    fn pose_problem(self: &Rc<Self>, contact: Contact, card: Option<MotionCard>) {
        let highlight = {
            let session = self.session.borrow();
            session
                .drawable()
                .map(|(_, graph)| view::highlight(graph, &contact))
        };
        let Some(highlight) = highlight else {
            return;
        };
        let shared = !highlight.volumes.is_empty();
        let turns = card.as_ref().is_none_or(|c| c.turns);
        {
            let mut state = self.state.borrow_mut();
            state.opening = None;
            state.problem = Some(ShownProblem {
                contact,
                highlight,
                card: card.clone(),
                amount: None,
                turns,
            });
        }
        self.widgets.motion.show(None);
        self.repose();
        {
            let session = self.session.borrow();
            let mut state = self.state.borrow_mut();
            let aspect = Self::aspect(&state);
            let framed = session.drawable().and_then(|(_, graph)| {
                let nodes = &state.problem.as_ref()?.highlight.nodes;
                let bounds = graph.bounds_of(state.posed.as_ref()?, nodes)?;
                Some(state.orbit?.framed(bounds, aspect))
            });
            if framed.is_some() {
                state.orbit = framed;
            }
        }
        match &card {
            Some(card) => {
                self.show_problem_controls(Some((card, card.shown, shared)));
                self.show_posed_status(card, card.shown);
            }
            None => {
                self.widgets.problem_card.show(None, 0.0);
                self.widgets.contact_key.set_visible(shared);
                self.widgets.offer_view_tools(false);
                if shared {
                    self.show_hint("Posed at contact");
                }
            }
        }
    }

    /// The 3D view's area behind its cards takes the stage colour, so a
    /// page laid out around the view (hardware focus) matches it.
    fn show_backdrop(&self, dark: bool) {
        for page in [&self.widgets.stage, &self.widgets.empty.root] {
            if dark {
                page.add_css_class("dark");
            } else {
                page.remove_css_class("dark");
            }
        }
        // What only the dark style changes in the sheet (the code blocks)
        // hangs off the window: a CSS media query doesn't see a style
        // chosen in the app's menu.
        if dark {
            self.widgets.window.add_css_class("stave-dark");
        } else {
            self.widgets.window.remove_css_class("stave-dark");
        }
    }

    /// Turns the camera to look from a direction, framing everything.
    fn look_from(self: &Rc<Self>, yaw: f32, pitch: f32) {
        {
            let session = self.session.borrow();
            let mut state = self.state.borrow_mut();
            let aspect = Self::aspect(&state);
            let bounds = session
                .drawable()
                .and_then(|(_, graph)| edit_view::drawn_bounds(graph, state.posed.as_ref()?));
            state.orbit = state.orbit.map(|orbit| {
                let turned = Orbit {
                    yaw: yaw.rem_euclid(std::f32::consts::TAU),
                    pitch,
                    ..orbit
                };
                bounds.map_or(turned, |bounds| turned.framed(bounds, aspect))
            });
        }
        self.request_render();
    }

    /// "Pull apart" switched on (all the way apart) or off (together).
    fn pull_apart(self: &Rc<Self>, on: bool) {
        {
            let Ok(mut state) = self.state.try_borrow_mut() else {
                return;
            };
            if state.updating_controls || state.pull_apart == on {
                return;
            }
            state.pull_apart = on;
            state.exploding = false;
        }
        self.dispatch(Command::SetExplode(Explode::clamped(if on {
            1.0
        } else {
            0.0
        })));
        self.show_view_toolbar();
    }

    /// Shows the view toolbar as the session and the window have it.
    fn show_view_toolbar(&self) {
        let explode = self.session.borrow().explode().get();
        let (on, playing) = {
            let mut state = self.state.borrow_mut();
            state.pull_apart |= explode > 0.0;
            state.updating_controls = true;
            (state.pull_apart, state.exploding)
        };
        self.widgets.view_toolbar.show(on, explode, playing);
        self.state.borrow_mut().updating_controls = false;
    }

    /// The pull apart play button: plays from together to apart (from the
    /// start when it is apart already), or pauses.
    fn toggle_play_explode(self: &Rc<Self>) {
        let playing = {
            let mut state = self.state.borrow_mut();
            state.exploding = !state.exploding;
            state.exploding
        };
        if playing && self.session.borrow().explode().get() >= 1.0 {
            self.dispatch(Command::SetExplode(Explode::clamped(0.0)));
        }
        self.show_view_toolbar();
        if !playing {
            return;
        }
        let window = self.clone();
        let last: std::cell::Cell<Option<i64>> = std::cell::Cell::new(None);
        self.widgets.viewport.add_tick_callback(move |_, clock| {
            if !window.state.borrow().exploding {
                return glib::ControlFlow::Break;
            }
            let now = clock.frame_time();
            let seconds = last.get().map_or(0.0, |before| {
                f64::from(i32::try_from(now - before).unwrap_or(0)) / 1e6
            });
            last.set(Some(now));
            let explode = window.session.borrow().explode().get();
            let next = (explode + narrow(seconds * EXPLODE_PER_SECOND)).min(1.0);
            let done = next >= 1.0;
            if done {
                window.state.borrow_mut().exploding = false;
            }
            window.dispatch(Command::SetExplode(Explode::clamped(next)));
            if done {
                window.show_view_toolbar();
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }

    /// Saves the view as posed as a glTF file, asking where.
    fn export_view(self: &Rc<Self>) {
        let name = {
            let session = self.session.borrow();
            let title = session
                .drawable()
                .map_or_else(|| "design".to_owned(), |(d, _)| d.project.title.clone());
            let slug: String = title
                .to_lowercase()
                .chars()
                .map(|c| if c.is_alphanumeric() { c } else { '_' })
                .collect();
            format!("{slug}_view.glb")
        };
        let dialog = gtk::FileDialog::builder()
            .title("Export View as glTF")
            .initial_name(name)
            .modal(true)
            .build();
        let window = self.clone();
        dialog.save(
            Some(&self.widgets.window),
            gio::Cancellable::NONE,
            move |result| {
                let Ok(file) = result else { return };
                let Some(path) = file.path() else { return };
                let saved = {
                    let session = window.session.borrow();
                    let state = window.state.borrow();
                    match (session.drawable(), state.posed.as_ref()) {
                        (Some((_, graph)), Some(posed)) => {
                            stave_app::export_view::save_glb(graph, posed, &path)
                                .map_err(|error| error.to_string())
                        }
                        _ => Err("there is nothing to save yet".to_owned()),
                    }
                };
                let title = match saved {
                    Ok(()) => format!("Saved {}", path.display()),
                    Err(error) => format!("Couldn't save the view: {error}"),
                };
                window
                    .widgets
                    .toasts
                    .add_toast(adw::Toast::builder().title(title).timeout(4).build());
            },
        );
    }

    fn explode_changed(self: &Rc<Self>) {
        self.show_view_toolbar();
        let explode = self.session.borrow().explode().get();
        let quiet = {
            let state = self.state.borrow();
            state.focus.is_none() && state.problem.is_none()
        };
        if quiet && self.session.borrow().selection().is_none() {
            self.show_hint(&if explode > 0.0 {
                format!("Pulled apart · {}%", view_tools::percent(explode))
            } else {
                String::new()
            });
        }
        self.repose();
        self.request_render();
    }

    fn scene_changed(self: &Rc<Self>) {
        let (titles, index) = {
            let session = self.session.borrow();
            let titles: Vec<String> = session
                .drawable()
                .map(|(_, g)| g.scenes.iter().map(|s| s.title.clone()).collect())
                .unwrap_or_default();
            (titles, session.scene_index())
        };
        self.widgets.header.show_scenes(&titles, index);
        self.state.borrow_mut().problem = None;
        self.show_problem_controls(None);
        self.repose();
        self.request_render();
    }

    /// Poses the design for what the window shows: focus mode, a problem
    /// at its contact, or the current scene pulled apart. In focus mode,
    /// returns its step and status for the panel.
    fn repose(&self) -> Option<(Option<String>, String)> {
        let session = self.session.borrow();
        let (_, graph) = session.drawable()?;
        let entry = session.scene_entry()?;
        let explode = session.explode().get();
        let mut state = self.state.borrow_mut();
        let State {
            posed,
            problem,
            opening,
            focus,
            ..
        } = &mut *state;
        if let Some(focus) = focus.as_mut() {
            // From above, the section cuts the receiving part open: it is
            // drawn solid.
            let options = focus::Options {
                xray: focus.options.xray && !focus.plan,
                ..focus.options
            };
            let frame = focus.focus.frame(graph, entry, focus.progress, &options);
            *posed = Some(frame.posed);
            focus.looks = frame.looks;
            focus.overlay = frame.overlay;
            return Some((frame.step, frame.status));
        }
        let at_contact = problem
            .as_ref()
            .and_then(|problem| view::pose_at_contact(graph, &problem.contact, problem.amount));
        *posed = Some(at_contact.unwrap_or_else(|| {
            let overrides: Vec<(AssemblyId, f32)> =
                opening.iter().map(|o| (o.assembly, o.amount)).collect();
            graph.pose_with(entry, &overrides, explode)
        }));
        None
    }

    fn pick(self: &Rc<Self>, x: f64, y: f64) {
        if self.edit_click(x, y) {
            return;
        }
        let hit = {
            let session = self.session.borrow();
            let state = self.state.borrow();
            let viewport = &self.widgets.viewport;
            session
                .drawable()
                .zip(state.orbit)
                .zip(state.posed.as_ref())
                .and_then(|(((_, graph), orbit), posed)| {
                    let ray = orbit.ray(
                        crate::convert::f32_of(x),
                        crate::convert::f32_of(y),
                        crate::convert::f32_of_int(viewport.width()),
                        crate::convert::f32_of_int(viewport.height()),
                    );
                    graph.pick(posed, ray)
                })
        };
        let selection = hit.map(|hit| Selection::of_item(hit.item));
        self.dispatch(Command::Select(selection));
    }

    fn update_orbit(self: &Rc<Self>, change: impl FnOnce(Orbit) -> Orbit) {
        {
            let mut state = self.state.borrow_mut();
            state.orbit = state.orbit.map(change);
        }
        self.request_render();
    }

    /// The stage colour for the current style.
    fn backdrop(&self) -> Backdrop {
        if adw::StyleManager::default().is_dark() {
            Backdrop::Dark
        } else {
            Backdrop::Light
        }
    }

    /// Draws on the next frame; several requests before then draw once.
    fn request_render(self: &Rc<Self>) {
        // The Model hidden behind an open file's tab draws nothing, so a
        // screenshot asked for is taken without it.
        if self.widgets.window.is_mapped() && !self.widgets.viewport.is_mapped() {
            self.capture_if_asked();
        }
        {
            let mut state = self.state.borrow_mut();
            if state.render_queued {
                return;
            }
            state.render_queued = true;
        }
        let window = self.clone();
        self.widgets.viewport.add_tick_callback(move |_, _| {
            window.render_now();
            glib::ControlFlow::Break
        });
    }

    /// How each node looks outside focus mode. Choosing something only
    /// outlines it: the rest stays solid, so clicking around never switches
    /// the view between solid and see-through. With X-ray on everything
    /// but the selection is see-through; a posed problem fades what it
    /// doesn't involve. `None` draws every node as usual.
    fn looks(&self, selection: &[NodeId]) -> Option<Vec<Look>> {
        let session = self.session.borrow();
        let state = self.state.borrow();
        let (_, graph) = session.drawable()?;
        let count = graph.node_ids().count();
        let kept: &[NodeId] = match state.problem.as_ref() {
            Some(problem) => &problem.highlight.nodes,
            None if state.xray => selection,
            None => return None,
        };
        let rest = if state.xray { Look::XRay } else { Look::Dim };
        let mut looks = vec![rest; count];
        for node in kept {
            if let Some(look) = looks.get_mut(node.index()) {
                *look = Look::Normal;
            }
        }
        Some(looks)
    }

    fn render_now(self: &Rc<Self>) {
        let selection = self.selected_nodes();
        let backdrop = self.backdrop();
        let looks = if self.state.borrow().focus.is_some() {
            None
        } else {
            self.looks(&selection)
        };
        let ghost = self.render_ghost(backdrop);
        let rendered = if let Some(rendered) = ghost {
            rendered
        } else {
            let session = self.session.borrow();
            let mut state = self.state.borrow_mut();
            state.render_queued = false;
            let (width, height) = state.size;
            let State {
                posed,
                orbit,
                renderer,
                problem,
                focus,
                ..
            } = &mut *state;
            let (Some((_, graph)), Some(orbit), Some(posed), Ok(renderer)) =
                (session.drawable(), *orbit, posed.as_ref(), renderer)
            else {
                return;
            };
            let focus = focus.as_ref();
            // Outside focus mode, a selected joint member shows through
            // the parts it sits in.
            let outside = if focus.is_none() {
                session.looks()
            } else {
                None
            };
            let plan = focus
                .filter(|f| f.plan)
                .and_then(|f| f.focus.plan().copied());
            let orbit = match plan {
                Some(plan) => {
                    Orbit::plan(plan.bounds, crate::render::aspect(width, height), plan.yaw)
                }
                None => orbit,
            };
            // The shared wood is drawn only where it was measured; moved by
            // the slider, the parts stay tinted.
            let highlight = problem.as_ref().map(|problem| match problem.amount {
                None => problem.highlight.clone(),
                Some(_) => Highlight {
                    nodes: problem.highlight.nodes.clone(),
                    volumes: Vec::new(),
                },
            });
            // The space the pointer or an edit is on, lit, and what covers
            // it see-through.
            let marks = if focus.is_none() {
                self.edit_marks(&session, graph, posed, &orbit)
            } else {
                edit_view::EditMarks::default()
            };
            let looks = if marks.covering.is_empty() {
                looks
            } else {
                let mut looks =
                    looks.unwrap_or_else(|| vec![Look::Normal; graph.node_ids().count()]);
                for node in &marks.covering {
                    if let Some(look) = looks.get_mut(node.index()) {
                        *look = Look::XRay;
                    }
                }
                Some(looks)
            };
            renderer.render(&Frame {
                scene: graph,
                posed,
                orbit,
                selection: if focus.is_some() { &[] } else { &selection },
                highlight: highlight.as_ref(),
                section: plan
                    .map(|p| p.section)
                    .or_else(|| session.scene_entry().and_then(|entry| entry.section)),
                looks: focus
                    .map(|f| f.looks.as_slice())
                    .or(outside.as_deref())
                    .or(looks.as_deref()),
                overlay: focus.map(|f| &f.overlay),
                tint_cut_faces: focus.is_none_or(|f| f.options.tint_cut_faces),
                lit: Some(&marks.lit),
                tints: None,
                backdrop,
                width,
                height,
            })
        };
        match rendered {
            Ok(image) => {
                if let Some(texture) = texture(image) {
                    self.widgets.viewport.show_frame(texture);
                }
                self.show_pins();
                self.place_ghost_verdict();
                self.place_grips();
                // Save the window once a frame of the view's real size is up.
                let settled = {
                    let state = self.state.borrow();
                    state.size.0 > 1 && state.size == self.widgets.viewport.device_size()
                };
                if settled {
                    // An edit script runs first; the frame it asks for is
                    // the one saved.
                    if self.run_edit_script() {
                        self.request_render();
                    } else {
                        self.capture_if_asked();
                    }
                }
            }
            Err(error) => {
                let message = format!("{error:#}");
                self.show_status("Can't draw in 3D", &message);
            }
        }
    }

    /// Where a set of nodes shows in the 3D view, in logical pixels from
    /// its top left (the middle of their bounds as posed); `None` when it
    /// can't be worked out (nothing drawn yet, behind the camera).
    fn screen_point(&self, nodes: &[NodeId]) -> Option<(f32, f32)> {
        let session = self.session.borrow();
        let state = self.state.borrow();
        let (_, graph) = session.drawable()?;
        let bounds = graph.bounds_of(state.posed.as_ref()?, nodes)?;
        self.project(&state, bounds.centre())
    }

    /// How much of the 3D view's right side the camera column takes (with
    /// the space beside it), in logical pixels: what the editing's cards
    /// and labels keep clear of at the right.
    fn right_column(&self) -> i32 {
        let camera = &self.widgets.camera.root;
        if camera.is_visible() {
            camera.width().saturating_add(EDGE)
        } else {
            0
        }
    }

    /// Where a point of the scene shows in the 3D view, in logical pixels.
    fn project(&self, state: &State, point: glam::Vec3) -> Option<(f32, f32)> {
        let viewport = &self.widgets.viewport;
        state.orbit?.project(
            point,
            crate::convert::f32_of_int(viewport.width()),
            crate::convert::f32_of_int(viewport.height()),
        )
    }

    /// Takes every pin off the model.
    fn hide_pins(&self) {
        for pin in self.pins.borrow_mut().drain(..) {
            self.widgets.design.remove_overlay(&pin);
        }
        self.widgets.edge_pin.set_visible(false);
    }

    /// Pins each problem where it happens (its shared wood, or the parts
    /// it is about), the first few; a problem whose place is off screen is
    /// named on the chip at the view's edge instead. A posed problem shows
    /// only its own pin; focus mode and an older design show none.
    fn show_pins(self: &Rc<Self>) {
        self.hide_pins();
        let found: Vec<Pin> = {
            let session = self.session.borrow();
            let state = self.state.borrow();
            // An edit's menu, card and ghost take the view.
            let editing = session.edit().is_some() || session.change().is_some();
            if state.focus.is_some()
                || editing
                || session.showing_older()
                || session.view() != View::Design
            {
                return;
            }
            let Some((_, graph)) = session.drawable() else {
                return;
            };
            let posed_problem = match session.selection() {
                Some(Selection::Problem(index)) => Some(*index),
                _ => None,
            };
            session
                .problems()
                .iter()
                .enumerate()
                .filter(|(index, _)| posed_problem.is_none_or(|p| p.0 == *index))
                .filter_map(|(index, problem)| {
                    let contact = problem.contact.as_ref()?;
                    let mark = Mark::of(problem.severity)?;
                    let highlight = view::highlight(graph, contact);
                    let point = match highlight.volumes.first() {
                        Some(volume) => self.project(&state, volume.centre()),
                        None => state.posed.as_ref().and_then(|posed| {
                            let bounds = graph.bounds_of(posed, &highlight.nodes)?;
                            self.project(&state, bounds.centre())
                        }),
                    };
                    Some(Pin {
                        index: ProblemIndex(index),
                        mark,
                        message: problem.message.clone(),
                        point,
                    })
                })
                .take(PIN_LIMIT)
                .collect()
        };
        let (width, height) = (
            crate::convert::f32_of_int(self.widgets.viewport.width()),
            crate::convert::f32_of_int(self.widgets.viewport.height()),
        );
        let mut off_screen: Vec<(ProblemIndex, Mark, String)> = Vec::new();
        for Pin {
            index,
            mark,
            message,
            point,
        } in found
        {
            let on_screen =
                point.filter(|(x, y)| (0.0..width).contains(x) && (0.0..height).contains(y));
            let Some((x, y)) = on_screen else {
                off_screen.push((index, mark, message));
                continue;
            };
            let pin = self.pin(index, mark, &message);
            // Clear of the cards and the parts list (edit_view).
            let wanted = ((x - 12.0).max(0.0), (y - 24.0).max(0.0));
            let size = edit_view::natural_size(pin.upcast_ref());
            let Some((dx, dy)) = self.clear_spot(wanted, size) else {
                off_screen.push((index, mark, message));
                continue;
            };
            pin.set_margin_start(crate::convert::int_of_f32((wanted.0 + dx).round()).max(0));
            pin.set_margin_top(crate::convert::int_of_f32((wanted.1 + dy).round()).max(0));
            self.widgets.design.add_overlay(&pin);
            self.pins.borrow_mut().push(pin.upcast());
        }
        if let Some((index, mark, message)) = off_screen.first() {
            let more = off_screen.len().saturating_sub(1);
            let words = match mark {
                Mark::Warning => "warning",
                Mark::Error | Mark::Involved => "error",
            };
            let text = if more == 0 {
                format!("Off screen: {words} · {message}")
            } else {
                format!("Off screen: {words} · {message} · {more} more")
            };
            self.widgets.edge_label.set_label(&text);
            self.widgets.edge_pin.set_visible(true);
            let index = *index;
            self.widgets
                .edge_pin
                .set_action_target_value(Some(&u64::try_from(index.0).unwrap_or(0).to_variant()));
        }
    }

    /// One pin: its mark's "!" and the problem's words, choosing it
    /// selects the problem (posed where it happens).
    fn pin(self: &Rc<Self>, index: ProblemIndex, mark: Mark, message: &str) -> gtk::Box {
        let warning = mark == Mark::Warning;
        let button = gtk::Button::builder()
            .label("!")
            .css_classes(if warning {
                vec!["stave-pin", "warning"]
            } else {
                vec!["stave-pin"]
            })
            .tooltip_text(message)
            .build();
        button.update_property(&[gtk::accessible::Property::Label(&format!(
            "Show the problem: {message}"
        ))]);
        let label = gtk::Label::builder()
            .label(message)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .max_width_chars(28)
            .css_classes(if warning {
                vec!["stave-pin-label", "warning"]
            } else {
                vec!["stave-pin-label"]
            })
            .build();
        let window = Rc::downgrade(self);
        button.connect_clicked(move |_| {
            if let Some(window) = window.upgrade() {
                window.dispatch(Command::Select(Some(Selection::Problem(index))));
            }
        });
        let pin = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(6)
            .halign(gtk::Align::Start)
            .valign(gtk::Align::Start)
            .build();
        pin.append(&button);
        pin.append(&label);
        pin
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

    /// With `--window-screenshot`: saves the window shortly (once GTK has
    /// drawn it) and closes it.
    fn capture_if_asked(&self) {
        let Some(path) = self.state.borrow_mut().window_screenshot.take() else {
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
        self.build_changed();
    }

    fn build_changed(&self) {
        let session = self.session.borrow();
        self.widgets
            .header
            .show_build(session.build(), session.build_tooltip());
        if let Some(action) = self.widgets.window.lookup_action("build")
            && let Ok(action) = action.downcast::<gio::SimpleAction>()
        {
            action.set_enabled(session.build() == Build::Idle && session.buildable().is_ok());
        }
    }

    // ---- events ----

    fn install_actions(self: &Rc<Self>, theme: Theme) {
        let window = &self.widgets.window;
        let add = |name: &str, handler: Box<dyn Fn()>| {
            let action = gio::SimpleAction::new(name, None);
            action.connect_activate(move |_, _| handler());
            window.add_action(&action);
        };
        let this = self.clone();
        add("new-project", Box::new(move || this.ask_new_project()));
        let this = self.clone();
        add("back", Box::new(move || this.dispatch(Command::Back)));
        let this = self.clone();
        add("forward", Box::new(move || this.dispatch(Command::Forward)));
        let this = self.clone();
        add("build", Box::new(move || this.dispatch(Command::Build)));
        let this = self.clone();
        add("check", Box::new(move || this.dispatch(Command::Check)));
        let this = self.clone();
        add("frame", Box::new(move || this.frame()));
        let this = self.clone();
        add("export-view", Box::new(move || this.export_view()));
        let this = self.clone();
        add(
            "next-problem",
            Box::new(move || this.dispatch(Command::NextProblem)),
        );
        let this = self.clone();
        add(
            "previous-problem",
            Box::new(move || this.dispatch(Command::PreviousProblem)),
        );
        let this = self.clone();
        add(
            "find",
            Box::new(move || this.widgets.finder.present(&this.widgets.window, "")),
        );
        let this = self.clone();
        add(
            "search",
            Box::new(move || {
                // In the text, its own search bar; anywhere else, the
                // finder.
                if !this.widgets.files.focus_search() {
                    this.widgets.finder.present(&this.widgets.window, "");
                }
            }),
        );
        let this = self.clone();
        add(
            "editor",
            // Shows the Code panel wherever it is docked, with the keyboard
            // in the text.
            Box::new(move || {
                if this.session.borrow().editor() == Editor::Open {
                    this.widgets.workbench.present(workbench::PanelId::Code);
                    this.widgets.files.focus_text();
                } else {
                    this.dispatch(Command::ShowEditor(Editor::Open));
                }
            }),
        );
        let show_panel = gio::SimpleAction::new("show-panel", Some(glib::VariantTy::STRING));
        let this = self.clone();
        show_panel.connect_activate(move |_, word| {
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
        let this = self.clone();
        move_panel.connect_activate(move |_, target| {
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
        let this = self.clone();
        add(
            "reset-layout",
            Box::new(move || this.widgets.workbench.reset()),
        );
        let this = self.clone();
        add(
            "model",
            Box::new(move || this.widgets.workbench.present(workbench::PanelId::Parts)),
        );
        let this = self.clone();
        add("save", Box::new(move || this.widgets.files.save()));
        let this = self.clone();
        add("format", Box::new(move || this.widgets.files.format()));
        let this = self.clone();
        add("open", Box::new(move || this.choose_folder()));
        let this = self.clone();
        add(
            "about",
            Box::new(move || {
                adw::AboutDialog::builder()
                    .application_name("Stave")
                    .comments("Woodworking design, checked like a compiler")
                    .version(env!("CARGO_PKG_VERSION"))
                    .license_type(gtk::License::Agpl30)
                    .build()
                    .present(Some(&this.widgets.window));
            }),
        );
        for view in View::ALL {
            let this = self.clone();
            add(
                &format!("view-{}", view.word()),
                Box::new(move || this.dispatch(Command::ShowView(view))),
            );
        }
        let this = self.clone();
        add(
            "escape",
            Box::new(move || {
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
        let this = self.clone();
        off_screen.connect_activate(move |_, parameter| {
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
        let style = gio::SimpleAction::new_stateful(
            "style",
            Some(glib::VariantTy::STRING),
            &theme.word().to_variant(),
        );
        style.connect_activate(|action, parameter| {
            let Some(word) = parameter.and_then(|p| p.get::<String>()) else {
                return;
            };
            if let Some(theme) = Theme::from_word(&word) {
                adw::StyleManager::default().set_color_scheme(theme.scheme());
                action.set_state(&word.to_variant());
            }
        });
        window.add_action(&style);
        if let Some(application) = window.application() {
            for (action, accels) in [
                ("win.back", &["<Alt>Left"][..]),
                ("win.forward", &["<Alt>Right"]),
                ("win.build", &["<Control>b"]),
                ("win.check", &["<Control>r"]),
                ("win.search", &["<Control>f"]),
                ("win.find", &["<Control>k"]),
                ("win.model", &["F9"]),
                ("win.open", &["<Control>o"]),
                ("win.editor", &["<Control>e"]),
                ("win.next-problem", &["F8"]),
                ("win.previous-problem", &["<Shift>F8"]),
                ("win.view-design", &["<Control>1"]),
                ("win.view-cut-list", &["<Control>2"]),
                ("win.save", &["<Control>s"]),
                ("win.format", &["<Control><Shift>i"]),
            ] {
                application.set_accels_for_action(action, accels);
            }
        }
    }

    /// Opens another project folder.
    fn choose_folder(self: &Rc<Self>) {
        let dialog = gtk::FileDialog::builder()
            .title("Open a project folder")
            .modal(true)
            .build();
        let window = self.clone();
        dialog.select_folder(
            Some(&self.widgets.window),
            gio::Cancellable::NONE,
            move |result| {
                let Ok(folder) = result else { return };
                let Some(path) = folder.path() else { return };
                let next = window.clone();
                window.ask_about_unsaved(move || next.open_folder(path));
            },
        );
    }

    /// Asks what to do with unsaved changes (save, throw away or stay),
    /// then goes on, unless people chose to stay.
    fn ask_about_unsaved(self: &Rc<Self>, then: impl FnOnce() + 'static) {
        let files = &self.widgets.files;
        let count = files.unsaved_count();
        if count == 0 {
            then();
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
        let window = self.clone();
        let then = RefCell::new(Some(then));
        dialog.connect_response(None, move |_, response| {
            let go_on = match response {
                "save" => window.widgets.files.save_all(),
                "discard" => true,
                _ => false,
            };
            if go_on && let Some(then) = then.borrow_mut().take() {
                then();
            }
        });
        dialog.present(Some(&self.widgets.window));
    }

    fn open_folder(self: &Rc<Self>, folder: PathBuf) {
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
        self.widgets.tree.show_tree(inspect::PartsTree::default());
        self.show_status("Checking…", "");
        self.dispatch(Command::Check);
        self.view_changed();
        self.selection_changed();
    }

    fn connect_events(self: &Rc<Self>) {
        let widgets = &self.widgets;
        let window = Rc::downgrade(self);
        widgets.problems.connect_choose(move |index| {
            if let Some(window) = window.upgrade() {
                window.dispatch(Command::Select(Some(Selection::Problem(ProblemIndex(
                    index,
                )))));
            }
        });
        let window = Rc::downgrade(self);
        widgets.problems.connect_source(move |location| {
            if let Some(window) = window.upgrade() {
                window.dispatch(Command::GoToSource(location));
            }
        });
        let window = Rc::downgrade(self);
        widgets.file_list.connect_open(move |path| {
            let (Some(window), Some(line)) = (window.upgrade(), LineNumber::new(1)) else {
                return;
            };
            window.dispatch(Command::GoToSource(SourceLocation {
                path: path.display().to_string(),
                line,
            }));
        });
        let header = &widgets.header;

        let window = self.clone();
        header.switcher.connect_active_name_notify(move |group| {
            let Some(view) = group.active_name().and_then(|name| View::from_word(&name)) else {
                return;
            };
            if window.session.borrow().view() != view {
                window.dispatch(Command::ShowView(view));
            }
        });

        let window = self.clone();
        header.scenes.connect_row_activated(move |_, row| {
            let name = {
                let session = window.session.borrow();
                usize::try_from(row.index())
                    .ok()
                    .and_then(|index| Some(session.drawable()?.1.scenes.get(index)?.name.clone()))
            };
            window.widgets.header.scene.popdown();
            if let Some(name) = name {
                window.dispatch(Command::ShowScene(name));
            }
        });

        let window = Rc::downgrade(self);
        widgets.tree.connect_select(move |selection| {
            if let Some(window) = window.upgrade() {
                window.select_and_frame(Some(selection));
            }
        });
        let window = Rc::downgrade(self);
        widgets.tree.connect_source(move |location| {
            if let Some(window) = window.upgrade() {
                window.dispatch(Command::GoToSource(location));
            }
        });
        let window = Rc::downgrade(self);
        widgets.tree.connect_focus(move |target| {
            if let Some(window) = window.upgrade() {
                window.dispatch(Command::Focus(target));
            }
        });
        let window = Rc::downgrade(self);
        widgets.dock.connect_move(move |assembly| {
            if let Some(window) = window.upgrade() {
                window.dispatch(Command::Select(Some(Selection::Assembly(assembly))));
            }
        });
        let window = self.clone();
        widgets.view_toolbar.xray.connect_toggled(move |button| {
            window.state.borrow_mut().xray = button.is_active();
            window.request_render();
        });
        self.connect_finder();
        let window = Rc::downgrade(self);
        widgets.inspector.connect_focus(move |target| {
            if let Some(window) = window.upgrade() {
                window.dispatch(Command::Focus(target));
            }
        });
        let window = Rc::downgrade(self);
        widgets.inspector.connect_source(move |location| {
            if let Some(window) = window.upgrade() {
                window.dispatch(Command::GoToSource(location));
            }
        });
        let window = self.clone();
        widgets
            .motion
            .scale
            .connect_value_changed(move |scale| window.motion_moved(scale.value()));

        // Focus mode's panel and its controls in the 3D view's area.
        let focus_panel = &widgets.inspector.focus;
        let window = self.clone();
        focus_panel
            .done
            .connect_clicked(move |_| window.dispatch(Command::LeaveFocus));
        let window = Rc::downgrade(self);
        focus_panel.connect_source(move |location| {
            if let Some(window) = window.upgrade() {
                window.dispatch(Command::GoToSource(location));
            }
        });
        for scrub in [&widgets.step_card.scrub, &widgets.swing_card.scrub] {
            let window = Rc::downgrade(self);
            scrub.connect_changed(move |value| {
                let Some(window) = window.upgrade() else {
                    return;
                };
                let value = narrow(value).clamp(0.0, 1.0);
                window.update_focus(|focus| {
                    focus.progress = value;
                    focus.playing = false;
                });
            });
        }
        for play in [&widgets.step_card.play, &widgets.swing_card.play] {
            let window = self.clone();
            play.connect_clicked(move |_| window.toggle_play_focus());
        }
        let window = self.clone();
        focus_panel.xray.connect_active_notify(move |check| {
            let on = check.is_active();
            window.update_focus(|focus| focus.options.xray = on);
        });
        let window = self.clone();
        focus_panel.tint.connect_active_notify(move |check| {
            let on = check.is_active();
            window.update_focus(|focus| focus.options.tint_cut_faces = on);
        });
        let window = self.clone();
        focus_panel.fade.connect_active_notify(move |check| {
            let on = check.is_active();
            window.update_focus(|focus| focus.options.fade_others = on);
        });
        let window = self.clone();
        widgets
            .plan_switch
            .from_above
            .connect_toggled(move |button| {
                let on = button.is_active();
                window.update_focus(|focus| focus.plan = on);
            });

        // A problem's motion card.
        let window = Rc::downgrade(self);
        widgets.problem_card.scrub.connect_changed(move |value| {
            if let Some(window) = window.upgrade() {
                window.problem_moved(value);
            }
        });

        // The view toolbar: Pull apart, its play button and its slider.
        let toolbar = &widgets.view_toolbar;
        let window = self.clone();
        toolbar.pull.connect_toggled(move |button| {
            let on = button.is_active();
            window.pull_apart(on);
        });
        let window = self.clone();
        toolbar
            .play
            .connect_clicked(move |_| window.toggle_play_explode());
        let window = Rc::downgrade(self);
        toolbar.slider.connect_changed(move |value| {
            if let Some(window) = window.upgrade() {
                window.state.borrow_mut().exploding = false;
                window.dispatch(Command::SetExplode(Explode::clamped(narrow(value))));
            }
        });

        // The camera buttons.
        let camera = &widgets.camera;
        camera.fit.set_action_name(Some("win.frame"));
        let window = self.clone();
        camera
            .front
            .connect_clicked(move |_| window.look_from(0.0, FRONT_PITCH));
        let window = self.clone();
        camera.turn.connect_clicked(move |_| {
            let (yaw, pitch) = window
                .state
                .borrow()
                .orbit
                .map_or((0.0, FRONT_PITCH), |orbit| (orbit.yaw, orbit.pitch));
            window.look_from(yaw + std::f32::consts::PI, pitch);
        });

        // The style changed: the 3D view's backdrop follows.
        let window = Rc::downgrade(self);
        adw::StyleManager::default().connect_dark_notify(move |manager| {
            if let Some(window) = window.upgrade() {
                window.show_backdrop(manager.is_dark());
                window.request_render();
                window.render_pictures();
                window
                    .widgets
                    .files
                    .restyle(adw::StyleManager::default().is_dark());
            }
        });
        self.show_backdrop(adw::StyleManager::default().is_dark());

        self.connect_files();
        self.connect_viewport();
    }

    /// The editor tells the window where the cursor is, where to go,
    /// when typing has paused and when a tab on the rail was chosen.
    fn connect_files(self: &Rc<Self>) {
        let files = &self.widgets.files;
        let window = Rc::downgrade(self);
        files.connect_link(move |found| {
            if let Some(window) = window.upgrade() {
                window.text_linked(found);
            }
        });
        let window = Rc::downgrade(self);
        files.connect_go(move |from, to| {
            let Some(window) = window.upgrade() else {
                return;
            };
            // Where the cursor was becomes the place Back returns to.
            if let Some(from) = from {
                window.dispatch(Command::Link(Some(Selection::Location(from))));
            }
            window.dispatch(Command::GoToSource(to));
        });
        let window = Rc::downgrade(self);
        files.connect_check(move || {
            if let Some(window) = window.upgrade() {
                window.dispatch(Command::Check);
            }
        });
        let window = Rc::downgrade(self);
        files.connect_show_in_3d(move |analysis, symbol| {
            let Some(window) = window.upgrade() else {
                return;
            };
            let selection = {
                let session = window.session.borrow();
                session.drawable().and_then(|(design, _)| {
                    link::selection_for(design, &analysis, symbol, session.selection())
                })
            };
            match selection {
                Some(selection) => window.select_and_frame(Some(selection)),
                None => window.widgets.toasts.add_toast(
                    adw::Toast::builder()
                        .title("It isn't in the design the 3D view shows")
                        .timeout(4)
                        .build(),
                ),
            }
        });
        let window = Rc::downgrade(self);
        files.connect_open(move || {
            if let Some(window) = window.upgrade() {
                window.dispatch(Command::ShowEditor(Editor::Open));
            }
        });
        // The open files as tabs beside the Model, wherever the Code
        // panel is docked.
        let page = Rc::downgrade(files);
        tab_strip::set_page_tabs(workbench::PanelId::Code.word(), move || {
            page.upgrade()
                .map(|page| page.page_tabs())
                .unwrap_or_default()
        });
        let window = Rc::downgrade(self);
        files.connect_notice(move |text| {
            if let Some(window) = window.upgrade() {
                window
                    .widgets
                    .toasts
                    .add_toast(adw::Toast::builder().title(text).timeout(4).build());
            }
        });
        // Closing the window asks about unsaved changes first.
        let window = Rc::downgrade(self);
        self.widgets.window.connect_close_request(move |_| {
            let Some(window) = window.upgrade() else {
                return glib::Propagation::Proceed;
            };
            window.widgets.workbench.save();
            if !window.widgets.files.has_unsaved() {
                return glib::Propagation::Proceed;
            }
            let closing = window.clone();
            window.ask_about_unsaved(move || {
                closing.widgets.files.forget_unsaved();
                closing.widgets.window.close();
            });
            glib::Propagation::Stop
        });
    }

    /// The finder finds through `stave_app::finder` and goes where a hit
    /// says: a part is selected (and framed), a problem posed, a place in
    /// the text opened in the editor.
    fn connect_finder(self: &Rc<Self>) {
        let window = Rc::downgrade(self);
        self.widgets.finder.connect_search(move |query| {
            let Some(window) = window.upgrade() else {
                return Vec::new();
            };
            let analysis = window.widgets.files.analysis();
            let session = window.session.borrow();
            finding::search(
                finding::Sources {
                    shown: session.shown().map(|c| &**c),
                    latest: session.latest().map(|c| &**c),
                    analysis: analysis.as_deref(),
                },
                query,
            )
        });
        let window = Rc::downgrade(self);
        self.widgets.finder.connect_hit(move |hit, choice| {
            let Some(window) = window.upgrade() else {
                return;
            };
            match (hit.goal, choice) {
                // Resting on a part shows it, without going there.
                (
                    Goal::Select(
                        selection @ (Selection::Part(_)
                        | Selection::Hardware(_)
                        | Selection::Assembly(_)),
                    ),
                    Choice::Preview,
                ) => {
                    window.state.borrow_mut().linking = true;
                    window.dispatch(Command::Link(Some(selection)));
                    window.state.borrow_mut().linking = false;
                    window.frame();
                }
                (Goal::Select(_) | Goal::Open(_), Choice::Preview) => {}
                (Goal::Select(selection), Choice::Go) => {
                    window.select_and_frame(Some(selection));
                }
                (Goal::Select(selection), Choice::OpenText) => {
                    let placed = {
                        let session = window.session.borrow();
                        session
                            .shown()
                            .and_then(|shown| link::placed_at(shown, &selection))
                    };
                    window.select_and_frame(Some(selection));
                    if let Some(placed) = placed {
                        window.dispatch(Command::GoToSource(placed));
                    }
                }
                (Goal::Open(location), Choice::Go | Choice::OpenText) => {
                    window.dispatch(Command::GoToSource(location));
                }
            }
        });
    }

    fn connect_viewport(self: &Rc<Self>) {
        let window = Rc::downgrade(self);
        self.widgets.pictures.connect_resized(move || {
            if let Some(window) = window.upgrade() {
                window.schedule_pictures();
            }
        });
        let viewport = &self.widgets.viewport;
        let window = self.clone();
        viewport.connect_resized(move |width, height| {
            let changed = {
                let mut state = window.state.borrow_mut();
                let changed = state.size != (width, height);
                let first = state.size.0 <= 1 && width > 1;
                state.size = (width, height);
                // The first camera was framed before the view had a size.
                if first && let Some(shown) = window.session.borrow().shown() {
                    state.orbit = Self::frame_all(&state, shown);
                }
                (changed, first)
            };
            let (changed, first) = changed;
            // ...and anything chosen by then is framed again at the real
            // size.
            if first && window.state.borrow().focus.is_none() {
                window.frame_selection();
            }
            if first && window.state.borrow().focus.is_some() {
                window.frame_focus();
            }
            if first {
                window.apply_start_camera();
            }
            if changed {
                window.request_render();
            }
        });

        // Left drag turns; a left click selects.
        let orbit_drag = gtk::GestureDrag::builder()
            .button(gdk::BUTTON_PRIMARY)
            .build();
        let window = self.clone();
        orbit_drag.connect_drag_begin(move |_, _, _| {
            window.state.borrow_mut().drag_applied = (0.0, 0.0);
            window.widgets.viewport.grab_focus();
        });
        let window = self.clone();
        orbit_drag.connect_drag_update(move |_, x, y| {
            let (dx, dy) = window.drag_step(x, y);
            window.update_orbit(|orbit| orbit.orbited(dx, dy));
        });
        let window = self.clone();
        orbit_drag.connect_drag_end(move |gesture, x, y| {
            if x.hypot(y) < CLICK_SLOP
                && let Some((start_x, start_y)) = gesture.start_point()
            {
                window.pick(start_x + x, start_y + y);
            }
        });
        viewport.add_controller(orbit_drag);

        // Right or middle drag moves the view sideways.
        for button in [gdk::BUTTON_SECONDARY, gdk::BUTTON_MIDDLE] {
            let pan_drag = gtk::GestureDrag::builder().button(button).build();
            let window = self.clone();
            pan_drag.connect_drag_begin(move |_, _, _| {
                window.state.borrow_mut().drag_applied = (0.0, 0.0);
            });
            let window = self.clone();
            pan_drag.connect_drag_update(move |_, x, y| {
                let (dx, dy) = window.drag_step(x, y);
                let height = crate::convert::f32_of_int(window.widgets.viewport.height());
                window.update_orbit(|orbit| orbit.panned(dx, dy, height));
            });
            viewport.add_controller(pan_drag);
        }

        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        let window = self.clone();
        scroll.connect_scroll(move |_, _, dy| {
            window.update_orbit(|orbit| orbit.zoomed(crate::convert::f32_of(-dy)));
            glib::Propagation::Stop
        });
        viewport.add_controller(scroll);

        // Escape and F anywhere in the window but a text field.
        let keys = gtk::EventControllerKey::new();
        let window = self.clone();
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            if !modifiers.is_empty()
                || gtk::prelude::GtkWindowExt::focus(&window.widgets.window)
                    .is_some_and(|w| w.is::<gtk::Text>() || w.is::<gtk::TextView>())
            {
                return glib::Propagation::Proceed;
            }
            match key {
                gdk::Key::Escape => {
                    let _ = WidgetExt::activate_action(&window.widgets.window, "win.escape", None);
                    glib::Propagation::Stop
                }
                gdk::Key::f | gdk::Key::F => {
                    window.frame();
                    glib::Propagation::Stop
                }
                // Delete removes the selection, saying first what goes.
                gdk::Key::Delete | gdk::Key::KP_Delete if window.remove_selected() => {
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
        self.widgets.window.add_controller(keys);

        // A mouse's Back and Forward buttons (8 and 9) anywhere in the
        // window, before the widget under the pointer sees them.
        let history = gtk::GestureClick::builder()
            .button(0)
            .propagation_phase(gtk::PropagationPhase::Capture)
            .build();
        let window = self.clone();
        history.connect_pressed(move |gesture, _, _, _| {
            let action = match gesture.current_button() {
                MOUSE_BACK => "win.back",
                MOUSE_FORWARD => "win.forward",
                _ => return,
            };
            gesture.set_state(gtk::EventSequenceState::Claimed);
            let _ = WidgetExt::activate_action(&window.widgets.window, action, None);
        });
        self.widgets.window.add_controller(history);
    }

    /// The part of a drag not applied yet.
    fn drag_step(&self, x: f64, y: f64) -> (f32, f32) {
        let mut state = self.state.borrow_mut();
        let (done_x, done_y) = state.drag_applied;
        state.drag_applied = (x, y);
        (
            crate::convert::f32_of(x - done_x),
            crate::convert::f32_of(y - done_y),
        )
    }

    // ---- watching ----

    /// Watches the project folder and checks again after a `.stave` file
    /// is saved (once the folder is quiet).
    fn watch(self: &Rc<Self>) {
        let folder = self.session.borrow().folder().to_path_buf();
        let (sender, receiver) = async_channel::unbounded::<()>();
        let watcher = Watcher::start(&folder, move || {
            // The window may be gone; then nobody needs to know.
            let _ = sender.send_blocking(());
        });
        match watcher {
            Ok(watcher) => {
                self.watcher.replace(Some(watcher));
            }
            Err(error) => {
                self.watcher.replace(None);
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
        let window = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            while receiver.recv().await.is_ok() {
                let Some(window) = window.upgrade() else {
                    break;
                };
                window.dispatch(Command::Check);
            }
        });
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
    let texture =
        gdk::MemoryTexture::new(width, height, gdk::MemoryFormat::R8g8b8a8, &bytes, stride);
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
fn main_menu() -> gio::Menu {
    /// A section of `(label, action)` items.
    fn section(items: &[(&str, &str)]) -> gio::Menu {
        let section = gio::Menu::new();
        for (label, action) in items {
            section.append(Some(label), Some(action));
        }
        section
    }
    let project = gio::Menu::new();
    project.append_section(
        None,
        &section(&[
            ("New Project…", "win.new-project"),
            ("Open Project…", "win.open"),
        ]),
    );
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
    view.append_section(
        Some("Style"),
        &section(&[
            ("Follow System Style", "win.style::system"),
            ("Light", "win.style::light"),
            ("Dark", "win.style::dark"),
        ]),
    );
    let help = section(&[("About Stave", "win.about")]);
    let menu = gio::Menu::new();
    menu.append_submenu(Some("Project"), &project);
    menu.append_submenu(Some("Edit"), &edit);
    menu.append_submenu(Some("View"), &view);
    menu.append_submenu(Some("Help"), &help);
    menu
}

fn build_widgets(application: &adw::Application, options: &Options) -> Result<Widgets> {
    let folder_name = options.folder.file_name().map_or_else(
        || options.folder.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    let header = Header::new(&folder_name, &main_menu());

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
    let orient_camera = camera.orientation_setter(&view_toolbar);
    let orient_moves = dock.moves_orientation_setter();
    let orient_motion = motion.orientation_setter();
    let saver = Rc::downgrade(&workbench);
    let floats = floating::Floats::new(
        &figure,
        floating,
        workbench.tools(),
        Box::new(move |group, orientation| match group {
            view_layout::Group::Scene => {}
            view_layout::Group::Camera => orient_camera(orientation),
            view_layout::Group::Moves => orient_moves(orientation),
            view_layout::Group::Motion => orient_motion(orientation),
        }),
        Box::new(move |group, spot| {
            if let Some(workbench) = saver.upgrade() {
                workbench.dock_tool(group, spot);
            }
        }),
    );
    let refresher = Rc::downgrade(&floats);
    workbench.connect_tools(move || {
        if let Some(floats) = refresher.upgrade() {
            floats.refresh();
        }
    });

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

    let (width, height) = options.size;
    let width = i32::try_from(width).unwrap_or(1440);
    let height = i32::try_from(height).unwrap_or(900);
    let window = adw::ApplicationWindow::builder()
        .application(application)
        .title(format!("{folder_name} — Stave"))
        .default_width(width)
        .default_height(height)
        .width_request(360)
        .height_request(400)
        .content(&toolbar)
        .css_classes(["stave-window"])
        .build();
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
        window,
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
        _floats: floats,
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
