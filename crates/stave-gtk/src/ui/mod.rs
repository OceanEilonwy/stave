//! The libadwaita window (Main.dc.html): a title bar with back and
//! forward, the Parts / Cut list / Files switcher, the scene menu, the
//! status pill and the build button; the Parts view (the parts tree, the
//! 3D view and the inspector); and a status bar.
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
mod editor;
mod files;
mod focus_panel;
mod header;
mod icons;
mod inspector;
mod style;
mod tree;
mod viewport;

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use adw::prelude::*;
use anyhow::{Context as _, Result, bail};
use gtk::{gdk, gio, glib};
use stave_app::Contact;
use stave_app::check::{self, Checked, Watcher};
use stave_app::focus::{self, Focus, Target};
use stave_app::inspect;
use stave_app::link;
use stave_app::session::{
    Build, Command, Event, Explode, LineNumber, ProblemIndex, Selection, Session, Sidebar,
    SourceLocation, Tone, Verdict, View,
};
use stave_app::view::{self, Highlight};
use stave_model::solved::AssemblyId;
use stave_scene::{Look, NodeId, Orbit, Overlay, Posed};

use crate::render::{Backdrop, Frame, Renderer};
use chrome::{MotionBar, ProblemsList, StatusBar};
pub use files::Wish;
use files::{FilesPage, Mode};
use header::Header;
use inspector::Inspector;
use tree::TreeSidebar;
use viewport::Viewport;

/// Moving the mouse less than this many pixels between press and release
/// is a click, not a drag.
const CLICK_SLOP: f64 = 4.0;

/// How fast focus mode plays: the whole way in 2.5 seconds.
const PLAY_PER_SECOND: f64 = 0.4;

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
    /// The problem to start showing (its index in the problems list).
    pub problem: Option<usize>,
    /// The explode amount to start with.
    pub explode: Option<f32>,
    /// The view to start in.
    pub view: View,
    /// Open the problems list.
    pub problems: bool,
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
    /// The file to open in the Files view, as `doors.stave:9`.
    pub file: Option<String>,
    /// What the Files view shows once the names are in (screenshots).
    pub wish: Wish,
    /// The Files view shows the text alone.
    pub text_only: bool,
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
    adw::init().context("can't start GTK (is there a display?)")?;
    let application = adw::Application::builder()
        .application_id("org.stave.Viewer")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    let options = Rc::new(RefCell::new(Some(Options { folder, ..options })));
    application.connect_activate(move |application| {
        if let Some(options) = options.borrow_mut().take() {
            Window::open(application, options);
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
    problem: Option<usize>,
    explode: Option<f32>,
    view: View,
    problems: bool,
    checking: bool,
    building: bool,
    file: Option<String>,
    wish: Wish,
    text_only: bool,
}

/// A problem posed in the view.
struct ShownProblem {
    contact: Contact,
    highlight: Highlight,
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
    /// Don't run checks (a screenshot of the checking state).
    hold_checks: bool,
    /// The selection is changing because the editor's cursor moved: the
    /// editor stays where it is.
    linking: bool,
}

/// The window's widgets.
struct Widgets {
    window: adw::ApplicationWindow,
    header: Header,
    toasts: adw::ToastOverlay,
    banner: adw::Banner,
    views: gtk::Stack,
    outer: adw::OverlaySplitView,
    inner: adw::OverlaySplitView,
    sidebar: gtk::Stack,
    tree: TreeSidebar,
    problems: ProblemsList,
    inspector: Inspector,
    view_stack: gtk::Stack,
    status_page: adw::StatusPage,
    viewport: Viewport,
    motion: MotionBar,
    files: Rc<FilesPage>,
    status: StatusBar,
}

struct Window {
    widgets: Widgets,
    session: RefCell<Session>,
    state: RefCell<State>,
    /// Kept alive for as long as the window: dropping it stops watching.
    watcher: RefCell<Option<Watcher>>,
}

impl Window {
    fn open(application: &adw::Application, options: Options) {
        if options.window_screenshot.is_some() {
            settle_for_screenshot();
        }
        adw::StyleManager::default().set_color_scheme(options.theme.scheme());
        if let Some(display) = gdk::Display::default() {
            style::install(&display);
        }
        let widgets = build_widgets(application, &options);
        let renderer = Renderer::new().map_err(|error| format!("{error:#}"));
        let window = Rc::new(Window {
            widgets,
            session: RefCell::new(Session::new(options.folder.clone())),
            state: RefCell::new(State {
                wanted: Some(Wanted {
                    scene: options.scene,
                    selection: options.select,
                    focus: options.focus,
                    problem: options.problem,
                    explode: options.explode,
                    view: options.view,
                    problems: options.problems,
                    checking: options.checking,
                    building: options.building,
                    file: options.file,
                    wish: options.wish,
                    text_only: options.text_only,
                }),
                window_screenshot: options.window_screenshot,
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
                hold_checks: false,
                linking: false,
            }),
            watcher: RefCell::new(None),
        });
        window.install_actions(options.theme);
        window.connect_events();
        window.watch();
        window.widgets.window.present();
        window.show_folder();
        window.dispatch(Command::Check);
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
        if events.contains(&Event::ResultChanged) {
            self.result_changed();
        }
        for event in events {
            match event {
                Event::ResultChanged => {}
                Event::ViewChanged => self.view_changed(),
                Event::SelectionChanged => self.selection_changed(),
                Event::FocusChanged => self.focus_changed(),
                Event::HistoryChanged => {
                    let session = self.session.borrow();
                    self.widgets
                        .header
                        .show_history(session.can_go_back(), session.can_go_forward());
                }
                Event::SceneChanged | Event::ExplodeChanged => self.scene_changed(),
                Event::SidebarChanged => self.sidebar_changed(),
                Event::CheckStatusChanged => self.check_status_changed(),
                Event::BuildChanged => self.build_changed(),
                Event::CheckStarted(ticket) => self.run_check(ticket),
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
        let name = folder.file_name().map_or_else(
            || folder.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        self.widgets.header.show_title(&name, &shown_path(&folder));
        self.widgets
            .window
            .set_title(Some(&format!("{name} — Stave")));
        self.widgets.files.set_folder(&folder);
    }

    fn result_changed(self: &Rc<Self>) {
        let (shown, latest, folder, showing_older) = {
            let session = self.session.borrow();
            (
                session.shown().cloned(),
                session.latest().cloned(),
                session.folder().to_path_buf(),
                session.showing_older(),
            )
        };
        let widgets = &self.widgets;
        let problems = latest
            .as_ref()
            .map_or(&[][..], |latest| latest.problems.as_slice());
        widgets.problems.show(problems);
        widgets.files.show_result(latest.as_deref());
        let Some(shown) = shown.filter(|c| c.drawable().is_some()) else {
            // Nothing solved yet: say why.
            let (title, description) = match latest.as_ref() {
                Some(latest) if latest.summary.errors > 0 => (
                    "Nothing to show yet",
                    format!(
                        "The design has {}. The status pill lists them.",
                        plural(latest.summary.errors, "error", "errors")
                    ),
                ),
                Some(_) => (
                    "Nothing to show",
                    "This folder is a library: it has nothing to place.".to_owned(),
                ),
                None => match self.session.borrow().check_status().verdict() {
                    Some(Verdict::CantOpen(message)) => ("Can't open the folder", message.clone()),
                    _ => ("Checking…", String::new()),
                },
            };
            self.show_status(title, &description);
            // The views that don't need the 3D view (the Files view) still
            // open as asked.
            self.apply_wanted();
            return;
        };
        if showing_older {
            widgets
                .banner
                .set_title("The design has errors. Showing the last version that worked.");
            widgets.banner.set_revealed(true);
        } else {
            widgets.banner.set_revealed(false);
        }
        let Some((design, graph)) = shown.drawable() else {
            return;
        };
        widgets
            .header
            .show_title(&design.project.title, &shown_path(&folder));
        widgets
            .window
            .set_title(Some(&format!("{} — Stave", design.project.title)));
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
        self.repose();
        if first {
            let mut state = self.state.borrow_mut();
            state.orbit = Self::frame_all(&state, &shown);
        }
        self.apply_wanted();
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
                let select = wanted.selection.as_ref().and_then(|name| {
                    design
                        .parts
                        .iter()
                        .find(|p| &p.name == name)
                        .map(|p| Selection::Part(p.id))
                        .or_else(|| {
                            design
                                .hardware
                                .iter()
                                .find(|h| &h.name == name)
                                .map(|h| Selection::Hardware(h.id))
                        })
                        .or_else(|| {
                            design
                                .assemblies
                                .iter()
                                .find(|a| &a.name == name)
                                .map(|a| Selection::Assembly(a.id))
                        })
                });
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
        if let Some(scene) = scene {
            self.dispatch(Command::ShowScene(scene));
        }
        if let Some(explode) = wanted.explode {
            self.dispatch(Command::SetExplode(Explode::clamped(explode)));
        }
        if let Some(selection) = select {
            self.dispatch(Command::Select(Some(selection)));
        }
        if let Some(target) = target {
            self.dispatch(Command::Focus(target));
        }
        if let Some(index) = wanted.problem {
            self.dispatch(Command::Select(Some(Selection::Problem(ProblemIndex(
                index,
            )))));
        }
        if wanted.problems {
            self.dispatch(Command::ShowSidebar(Sidebar::Problems));
        }
        if wanted.view != View::Parts {
            self.dispatch(Command::ShowView(wanted.view));
        }
        if wanted.text_only {
            self.widgets.files.set_mode(Mode::Text);
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
    }

    fn view_changed(self: &Rc<Self>) {
        let view = self.session.borrow().view();
        self.widgets.header.show_view(view);
        self.widgets.views.set_visible_child_name(view.word());
        self.place_viewport();
        self.sync_files();
    }

    /// `true` when the 3D view is on screen: the Parts view, or the Files
    /// view beside the text.
    fn shows_3d(&self) -> bool {
        match self.session.borrow().view() {
            View::Parts => true,
            View::CutList => false,
            View::Files => self.widgets.files.mode() == Mode::Both,
        }
    }

    /// Puts the 3D view where it shows: in the Parts view, or beside the
    /// text in the Files view (Both). It is one widget, moved between the
    /// two, so the camera and everything posed stay as they are.
    fn place_viewport(self: &Rc<Self>) {
        let view_stack = &self.widgets.view_stack;
        let in_files =
            self.session.borrow().view() == View::Files && self.widgets.files.mode() == Mode::Both;
        let slot = &self.widgets.files.view_slot;
        let in_slot = view_stack
            .parent()
            .is_some_and(|parent| parent == *slot.upcast_ref::<gtk::Widget>());
        if in_files && !in_slot {
            self.widgets.inner.set_content(None::<&gtk::Widget>);
            slot.set_child(Some(view_stack));
            self.request_render();
        } else if !in_files && in_slot {
            slot.set_child(None::<&gtk::Widget>);
            self.widgets.inner.set_content(Some(view_stack));
            self.request_render();
        }
    }

    /// The Files view follows the selection: a place in a file is shown;
    /// a part, assembly or piece of hardware selected elsewhere moves the
    /// text to the line that places it, and the chip names it.
    fn sync_files(self: &Rc<Self>) {
        let (view, selection, placed, chip) = {
            let session = self.session.borrow();
            let selection = session.selection().cloned();
            let placed = session
                .shown()
                .zip(selection.as_ref())
                .and_then(|(shown, selection)| link::placed_at(shown, selection));
            let chip = session
                .drawable()
                .zip(selection.as_ref())
                .zip(placed.as_ref())
                .and_then(|(((design, _), selection), placed)| {
                    link::chip(design, selection, placed)
                });
            (session.view(), selection, placed, chip)
        };
        if view != View::Files {
            return;
        }
        let files = &self.widgets.files;
        match (&selection, placed) {
            (Some(Selection::Location(location)), _) => files.show_location(location),
            (Some(_), Some(placed)) => {
                if self.state.borrow().linking {
                    files.set_chip(chip.as_deref());
                } else {
                    files.reveal(&placed, chip.as_deref());
                }
            }
            _ => files.set_chip(None),
        }
    }

    /// The editor's cursor rests on a name: select what it means in the
    /// design, without a history entry. A name the 3D view can't show
    /// leaves the selection as it is.
    fn text_linked(
        self: &Rc<Self>,
        found: Option<(Arc<stave_ide::Analysis>, stave_ide::SymbolId)>,
    ) {
        let Some((analysis, symbol)) = found else {
            return;
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
        let (selection, inspection, problem, name, focused) = {
            let session = self.session.borrow();
            let selection = session.selection().cloned();
            let inspection = session
                .shown()
                .zip(selection.as_ref())
                .and_then(|(shown, selection)| inspect::inspect(shown, selection));
            let problem = session.selected_problem().cloned();
            let name = selection.as_ref().and_then(|selection| {
                let (design, _) = session.drawable()?;
                selection_name(design, selection)
            });
            (
                selection,
                inspection,
                problem,
                name,
                session.focus().is_some(),
            )
        };
        let widgets = &self.widgets;
        widgets.tree.show_selection(selection.as_ref());
        if !focused {
            widgets
                .status
                .show_selected(&name.map_or_else(String::new, |name| format!("Selected: {name}")));
        }
        self.sync_files();
        // A narrow window shows the inspector over the view while
        // something is selected.
        if widgets.inner.is_collapsed() {
            widgets.inner.set_show_sidebar(selection.is_some());
        }
        if focused {
            return;
        }
        match &problem {
            Some(problem) => widgets.inspector.show_problem(problem),
            None => widgets.inspector.show(inspection.as_ref()),
        }
        // Pose a problem where it happens, or offer to open the selection.
        match problem.and_then(|p| p.contact) {
            Some(contact) => self.pose_problem(contact),
            None => {
                self.state.borrow_mut().problem = None;
                self.update_opening(inspection.and_then(|i| i.moves_with));
            }
        }
        self.request_render();
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
                focus::open(&shown.opened, design, graph, target)
            })
        };
        let Some(found) = found else {
            return;
        };
        let progress = if found.assembles() { 0.0 } else { 0.4 };
        let plan = found.prefers_plan();
        {
            let mut state = self.state.borrow_mut();
            state.problem = None;
            state.opening = None;
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
        let texts = self.repose();
        {
            // Frame the joint or the hardware.
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
            state.updating_controls = true;
        }
        {
            let panel = &self.widgets.inspector.focus;
            {
                let state = self.state.borrow();
                panel.show(state.focus.as_ref().map(|f| &f.focus));
            }
            // Setting the controls calls their handlers, which look at the
            // state: nothing may be borrowed here.
            panel.scrub.set_value(f64::from(progress));
            panel.play.set_active(false);
            panel.xray.set_active(true);
            panel.tint.set_active(true);
            panel.fade.set_active(true);
            panel.from_above.set_active(plan);
        }
        self.state.borrow_mut().updating_controls = false;
        self.widgets.inspector.show_focus();
        self.widgets
            .status
            .show_selected("Focused on one joint or piece of hardware · Esc to go back");
        self.show_focus_texts(texts);
        self.render_pictures();
        self.request_render();
    }

    fn leave_focus(self: &Rc<Self>) {
        self.state.borrow_mut().focus = None;
        self.widgets.inspector.focus.show(None);
        self.repose();
        self.selection_changed();
    }

    fn show_focus_texts(&self, texts: Option<(Option<String>, String)>) {
        if let Some((step, status)) = texts {
            self.widgets
                .inspector
                .focus
                .show_frame(step.as_deref(), &status);
        }
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
        self.request_render();
    }

    /// Play toggled: moves the progress on every frame until it is done.
    fn play_focus(self: &Rc<Self>, playing: bool) {
        {
            let Ok(mut state) = self.state.try_borrow_mut() else {
                return;
            };
            if state.updating_controls {
                return;
            }
            let Some(focus) = state.focus.as_mut() else {
                return;
            };
            let already = focus.playing;
            focus.playing = playing;
            if playing && focus.progress >= 1.0 {
                focus.progress = 0.0;
            }
            if already || !playing {
                return;
            }
        }
        let window = self.clone();
        let last: std::cell::Cell<Option<i64>> = std::cell::Cell::new(None);
        self.widgets.viewport.add_tick_callback(move |_, clock| {
            let now = clock.frame_time();
            let seconds = last.get().map_or(0.0, |before| {
                f64::from(i32::try_from(now - before).unwrap_or(0)) / 1e6
            });
            last.set(Some(now));
            let (progress, done) = {
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
                (focus.progress, done)
            };
            let texts = window.repose();
            window.state.borrow_mut().updating_controls = true;
            let panel = &window.widgets.inspector.focus;
            panel.scrub.set_value(f64::from(progress));
            if done {
                panel.play.set_active(false);
            }
            window.state.borrow_mut().updating_controls = false;
            window.show_focus_texts(texts);
            window.request_render();
            if done {
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }

    /// Renders the focus's small pictures offscreen for the panel.
    fn render_pictures(&self) {
        let backdrop = self.backdrop();
        let session = self.session.borrow();
        let (Some((_, graph)), Some(entry)) = (session.drawable(), session.scene_entry()) else {
            return;
        };
        let mut state = self.state.borrow_mut();
        let State {
            focus,
            renderer,
            orbit,
            ..
        } = &mut *state;
        let (Some(focus), Ok(renderer), Some(orbit)) = (focus.as_ref(), renderer, *orbit) else {
            return;
        };
        let (width, height) = (240, 170);
        let mut pictures = Vec::new();
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
                overlay: Some(&frame.overlay),
                tint_cut_faces: focus.options.tint_cut_faces,
                backdrop,
                width,
                height,
            });
            let Ok(image) = image else { continue };
            if let Some(texture) = texture(image) {
                pictures.push((texture, pose.caption.clone()));
            }
        }
        drop(state);
        drop(session);
        self.widgets.inspector.focus.show_pictures(pictures);
    }

    // ---- the 3D view ----

    fn aspect(state: &State) -> f32 {
        crate::render::aspect(state.size.0, state.size.1)
    }

    /// A camera that shows everything shown.
    fn frame_all(state: &State, shown: &Checked) -> Option<Orbit> {
        let (_, graph) = shown.drawable()?;
        let bounds = graph.bounds(state.posed.as_ref()?)?;
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

    /// F: frames the selection, or everything.
    fn frame(self: &Rc<Self>) {
        let node = self.selected_node();
        {
            let session = self.session.borrow();
            let Some(shown) = session.shown() else {
                return;
            };
            let mut state = self.state.borrow_mut();
            let aspect = Self::aspect(&state);
            let selected = node.and_then(|id| {
                let (_, graph) = shown.drawable()?;
                let bounds = graph.bounds_of(state.posed.as_ref()?, &[id])?;
                Some(state.orbit?.framed(bounds, aspect))
            });
            state.orbit = selected.or_else(|| Self::frame_all(&state, shown));
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

    /// The motion bar moved: a problem's swing, or the selection opening.
    fn motion_moved(self: &Rc<Self>, value: f64) {
        {
            let Ok(mut state) = self.state.try_borrow_mut() else {
                return;
            };
            if state.updating_controls {
                return;
            }
            let value = narrow(value);
            if let Some(problem) = state.problem.as_mut() {
                problem.amount = Some(if problem.turns {
                    value.to_radians()
                } else {
                    value
                });
            } else if let Some(opening) = state.opening.as_mut() {
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

    /// Poses the design where a problem happens and frames its parts.
    fn pose_problem(self: &Rc<Self>, contact: Contact) {
        let prepared = {
            let session = self.session.borrow();
            session.drawable().map(|(design, graph)| {
                let highlight = view::highlight(graph, &contact);
                let range = view::contact_motion_range(graph, &contact);
                let shown_amount = contact
                    .motion
                    .and_then(|motion| graph.amount(motion.assembly, motion.shown));
                let name = contact
                    .motion
                    .and_then(|motion| design.assemblies.get(motion.assembly.index()))
                    .map(|a| a.name.clone());
                (highlight, range, shown_amount, name)
            })
        };
        let Some((highlight, range, shown_amount, name)) = prepared else {
            return;
        };
        let turns = range.is_some_and(|r| r.turns);
        {
            let mut state = self.state.borrow_mut();
            state.opening = None;
            state.problem = Some(ShownProblem {
                contact,
                highlight,
                amount: None,
                turns,
            });
        }
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
            state.updating_controls = true;
        }
        match range.zip(shown_amount) {
            Some((range, amount)) => {
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
                let label = format!("Swing {}", name.as_deref().unwrap_or("it"));
                self.widgets.motion.show(Some((
                    &label,
                    "closed",
                    &end,
                    f64::from(full),
                    f64::from(value),
                )));
            }
            None => self.widgets.motion.show(None),
        }
        self.state.borrow_mut().updating_controls = false;
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

    fn render_now(self: &Rc<Self>) {
        let selection = self.selected_nodes();
        let backdrop = self.backdrop();
        let rendered = {
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
            renderer.render(&Frame {
                scene: graph,
                posed,
                orbit,
                selection: if focus.is_some() { &[] } else { &selection },
                highlight: highlight.as_ref(),
                section: plan
                    .map(|p| p.section)
                    .or_else(|| session.scene_entry().and_then(|entry| entry.section)),
                looks: focus.map(|f| f.looks.as_slice()),
                overlay: focus.map(|f| &f.overlay),
                tint_cut_faces: focus.is_none_or(|f| f.options.tint_cut_faces),
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
                // Save the window once a frame of the view's real size is up.
                let settled = {
                    let state = self.state.borrow();
                    state.size.0 > 1 && state.size == self.widgets.viewport.device_size()
                };
                if settled {
                    self.capture_if_asked();
                }
            }
            Err(error) => {
                let message = format!("{error:#}");
                self.show_status("Can't draw in 3D", &message);
            }
        }
    }

    fn show_status(&self, title: &str, description: &str) {
        self.widgets.status_page.set_title(title);
        self.widgets.status_page.set_description(Some(description));
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
                window.close();
                return glib::ControlFlow::Break;
            };
            if let Err(error) = save_node(&window, &node, &path) {
                eprintln!("error: {error:#}");
            }
            window.close();
            glib::ControlFlow::Break
        });
    }

    // ---- the rest of the session's events ----

    fn sidebar_changed(&self) {
        let session = self.session.borrow();
        let sidebar = session.sidebar();
        self.widgets.sidebar.set_visible_child_name(match sidebar {
            Sidebar::Parts => "parts",
            Sidebar::Problems => "problems",
        });
        if sidebar == Sidebar::Problems {
            self.widgets.outer.set_show_sidebar(true);
        }
        self.widgets
            .header
            .show_pill(&session.check_status().pill(), sidebar == Sidebar::Problems);
    }

    fn check_status_changed(&self) {
        {
            let session = self.session.borrow();
            let status = session.check_status();
            self.widgets
                .header
                .show_pill(&status.pill(), session.sidebar() == Sidebar::Problems);
            self.widgets.status.show_check(status);
        }
        self.build_changed();
    }

    fn build_changed(&self) {
        let session = self.session.borrow();
        let has_errors = session.check_status().pill().tone == Tone::Errors;
        self.widgets.header.show_build(session.build(), has_errors);
        if let Some(action) = self.widgets.window.lookup_action("build")
            && let Ok(action) = action.downcast::<gio::SimpleAction>()
        {
            action.set_enabled(session.build() == Build::Idle && !has_errors);
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
        add(
            "problems",
            Box::new(move || {
                let open = this.session.borrow().sidebar() == Sidebar::Problems;
                this.dispatch(Command::ShowSidebar(if open {
                    Sidebar::Parts
                } else {
                    Sidebar::Problems
                }));
            }),
        );
        let this = self.clone();
        add(
            "search",
            Box::new(move || {
                // The Files view searches files and names; the others the
                // parts tree.
                if this.session.borrow().view() == View::Files {
                    this.widgets.files.focus_search();
                    return;
                }
                this.dispatch(Command::ShowSidebar(Sidebar::Parts));
                this.widgets.outer.set_show_sidebar(true);
                this.widgets.tree.focus_search();
            }),
        );
        let this = self.clone();
        add("save", Box::new(move || this.widgets.files.save()));
        let this = self.clone();
        add("format", Box::new(move || this.widgets.files.format()));
        let this = self.clone();
        add(
            "sidebar",
            Box::new(move || {
                let outer = &this.widgets.outer;
                outer.set_show_sidebar(!outer.shows_sidebar());
            }),
        );
        let this = self.clone();
        add("open", Box::new(move || this.choose_folder()));
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
                let focused = this.session.borrow().focus().is_some();
                if focused {
                    this.dispatch(Command::LeaveFocus);
                } else {
                    this.dispatch(Command::Select(None));
                }
            }),
        );
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
                ("win.sidebar", &["F9"]),
                ("win.open", &["<Control>o"]),
                ("win.problems", &["<Control>e"]),
                ("win.view-parts", &["<Control>1"]),
                ("win.view-cut-list", &["<Control>2"]),
                ("win.view-files", &["<Control>3"]),
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
        let header = &widgets.header;
        header.back.set_action_name(Some("win.back"));
        header.forward.set_action_name(Some("win.forward"));
        header.open.set_action_name(Some("win.open"));
        header.pill.set_action_name(Some("win.problems"));

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
                window.dispatch(Command::Select(Some(selection)));
            }
        });
        let window = Rc::downgrade(self);
        widgets.problems.connect_problem(move |index| {
            if let Some(window) = window.upgrade() {
                window.dispatch(Command::Select(Some(Selection::Problem(ProblemIndex(
                    index,
                )))));
            }
        });
        let window = self.clone();
        widgets
            .problems
            .close
            .connect_clicked(move |_| window.dispatch(Command::ShowSidebar(Sidebar::Parts)));
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

        // Focus mode's panel.
        let focus_panel = &widgets.inspector.focus;
        let window = self.clone();
        focus_panel
            .done
            .connect_clicked(move |_| window.dispatch(Command::LeaveFocus));
        let window = self.clone();
        focus_panel.scrub.connect_value_changed(move |scale| {
            let value = narrow(scale.value()).clamp(0.0, 1.0);
            window.update_focus(|focus| {
                focus.progress = value;
                focus.playing = false;
            });
        });
        let window = self.clone();
        focus_panel
            .play
            .connect_toggled(move |button| window.play_focus(button.is_active()));
        let window = self.clone();
        focus_panel.xray.connect_active_notify(move |row| {
            let on = row.is_active();
            window.update_focus(|focus| focus.options.xray = on);
        });
        let window = self.clone();
        focus_panel.tint.connect_active_notify(move |row| {
            let on = row.is_active();
            window.update_focus(|focus| focus.options.tint_cut_faces = on);
        });
        let window = self.clone();
        focus_panel.fade.connect_active_notify(move |row| {
            let on = row.is_active();
            window.update_focus(|focus| focus.options.fade_others = on);
        });
        let window = self.clone();
        focus_panel.from_above.connect_toggled(move |button| {
            let on = button.is_active();
            window.update_focus(|focus| focus.plan = on);
        });

        // The style changed: the 3D view's backdrop follows.
        let window = Rc::downgrade(self);
        adw::StyleManager::default().connect_dark_notify(move |_| {
            if let Some(window) = window.upgrade() {
                window.request_render();
                window.render_pictures();
                window
                    .widgets
                    .files
                    .restyle(adw::StyleManager::default().is_dark());
            }
        });

        self.connect_files();
        self.connect_viewport();
    }

    /// The Files view tells the window where the cursor is, where to go,
    /// when typing has paused and which way it shows the 3D view.
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
            if window.widgets.files.mode() == Mode::Text {
                window.widgets.files.set_mode(Mode::Both);
            }
            match selection {
                Some(selection) => window.dispatch(Command::Select(Some(selection))),
                None => window.widgets.toasts.add_toast(
                    adw::Toast::builder()
                        .title("It isn't in the design the 3D view shows")
                        .timeout(4)
                        .build(),
                ),
            }
        });
        let window = Rc::downgrade(self);
        files.connect_mode(move |_| {
            if let Some(window) = window.upgrade() {
                window.place_viewport();
                window.sync_files();
            }
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
        // The title bar's sidebar button shows the files in the Files view.
        let window = Rc::downgrade(self);
        self.widgets
            .header
            .sidebar
            .connect_active_notify(move |button| {
                let Some(window) = window.upgrade() else {
                    return;
                };
                if window.session.borrow().view() == View::Files {
                    window
                        .widgets
                        .files
                        .root
                        .set_show_sidebar(button.is_active());
                }
            });
        // Closing the window asks about unsaved changes first.
        let window = Rc::downgrade(self);
        self.widgets.window.connect_close_request(move |_| {
            let Some(window) = window.upgrade() else {
                return glib::Propagation::Proceed;
            };
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

    fn connect_viewport(self: &Rc<Self>) {
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
                changed
            };
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
                _ => glib::Propagation::Proceed,
            }
        });
        self.widgets.window.add_controller(keys);
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
                self.widgets.status.show_watching("Watching for changes");
            }
            Err(error) => {
                self.watcher.replace(None);
                eprintln!("warning: {error}");
                self.widgets
                    .status
                    .show_watching("Not watching for changes");
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

/// The name of what is selected, for the status bar.
fn selection_name(
    design: &stave_model::solved::SolvedDesign,
    selection: &Selection,
) -> Option<String> {
    match selection {
        Selection::Part(id) => design.part(*id).map(|p| p.name.clone()),
        Selection::Hardware(id) => design.hardware.get(id.index()).map(|h| h.name.clone()),
        Selection::Assembly(id) => design.assemblies.get(id.index()).map(|a| a.name.clone()),
        Selection::Joint { joint, .. } => design.joints.get(joint.index()).map(|j| j.name.clone()),
        Selection::Location(location) => Some(format!(
            "{}:{}",
            Path::new(&location.path).file_name().map_or_else(
                || location.path.clone(),
                |n| n.to_string_lossy().into_owned()
            ),
            location.line
        )),
        Selection::Problem(ProblemIndex(index)) => {
            Some(format!("problem {}", index.saturating_add(1)))
        }
    }
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

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

fn main_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    let project = gio::Menu::new();
    project.append(Some("Open Project…"), Some("win.open"));
    project.append(Some("Check Again"), Some("win.check"));
    project.append(Some("Frame Everything"), Some("win.frame"));
    menu.append_section(None, &project);
    let style = gio::Menu::new();
    style.append(Some("Follow System Style"), Some("win.style::system"));
    style.append(Some("Light"), Some("win.style::light"));
    style.append(Some("Dark"), Some("win.style::dark"));
    menu.append_section(Some("Style"), &style);
    menu
}

fn build_widgets(application: &adw::Application, options: &Options) -> Widgets {
    let folder_name = options.folder.file_name().map_or_else(
        || options.folder.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    let header = Header::new(&folder_name, &main_menu());

    // The Parts view: the tree, the 3D view, the inspector.
    let tree = TreeSidebar::new();
    let problems = ProblemsList::new();
    let sidebar = gtk::Stack::new();
    sidebar.add_named(&tree.root, Some("parts"));
    sidebar.add_named(&problems.root, Some("problems"));
    sidebar.set_visible_child_name("parts");

    let viewport = Viewport::default();
    let motion = MotionBar::new();
    let overlay = gtk::Overlay::builder()
        .child(&viewport)
        .css_classes(["stave-viewport"])
        .build();
    overlay.add_overlay(&motion.root);
    // The slots lane 2 fills: the view's own toolbar (Pull apart and its
    // slider) at the top left, the camera buttons at the top right.
    for (name, halign) in [
        ("view-toolbar", gtk::Align::Start),
        ("camera-toolbar", gtk::Align::End),
    ] {
        let slot = gtk::Box::builder()
            .name(name)
            .halign(halign)
            .valign(gtk::Align::Start)
            .margin_top(12)
            .margin_start(12)
            .margin_end(12)
            .visible(false)
            .build();
        overlay.add_overlay(&slot);
    }
    let status_page = adw::StatusPage::builder()
        .title("Checking…")
        .css_classes(["stave-viewport"])
        .build();
    let view_stack = gtk::Stack::new();
    view_stack.add_named(&overlay, Some("view"));
    view_stack.add_named(&status_page, Some("status"));
    view_stack.set_visible_child_name("status");

    let inspector = Inspector::new();
    let inner = adw::OverlaySplitView::builder()
        .content(&view_stack)
        .sidebar(&inspector.root)
        .sidebar_position(gtk::PackType::End)
        .min_sidebar_width(300.0)
        .max_sidebar_width(360.0)
        .sidebar_width_fraction(0.25)
        .show_sidebar(true)
        .build();
    let outer = adw::OverlaySplitView::builder()
        .content(&inner)
        .sidebar(&sidebar)
        .min_sidebar_width(260.0)
        .max_sidebar_width(300.0)
        .sidebar_width_fraction(0.2)
        .show_sidebar(true)
        .build();
    header
        .sidebar
        .bind_property("active", &outer, "show-sidebar")
        .bidirectional()
        .sync_create()
        .build();

    let files = FilesPage::new();
    files.keep_chip_above(&motion.root);
    let views = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .vexpand(true)
        .build();
    views.add_named(&outer, Some(View::Parts.word()));
    views.add_named(&chrome::cut_list_page(), Some(View::CutList.word()));
    views.add_named(&files.root, Some(View::Files.word()));
    views.set_visible_child_name(View::Parts.word());

    let banner = adw::Banner::new("");
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&banner);
    content.append(&views);
    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&content));

    let status = StatusBar::new();
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header.bar);
    toolbar.add_bottom_bar(&status.root);
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

    // Narrower windows fold the parts tree away first, then the inspector
    // (it slides over the view while something is selected), then words.
    let medium = breakpoint(1100.0);
    medium.add_setter(&outer, "collapsed", Some(&true.to_value()));
    medium.add_setter(&outer, "show-sidebar", Some(&false.to_value()));
    medium.add_setter(&files.root, "collapsed", Some(&true.to_value()));
    medium.add_setter(&files.root, "show-sidebar", Some(&false.to_value()));
    window.add_breakpoint(medium);
    for (width, tiny) in [(860.0, false), (600.0, true)] {
        let narrow = breakpoint(width);
        narrow.add_setter(&outer, "collapsed", Some(&true.to_value()));
        narrow.add_setter(&outer, "show-sidebar", Some(&false.to_value()));
        narrow.add_setter(&inner, "collapsed", Some(&true.to_value()));
        narrow.add_setter(&inner, "show-sidebar", Some(&false.to_value()));
        narrow.add_setter(&files.root, "collapsed", Some(&true.to_value()));
        narrow.add_setter(&files.root, "show-sidebar", Some(&false.to_value()));
        narrow.add_setter(
            files.paned(),
            "orientation",
            Some(&gtk::Orientation::Vertical.to_value()),
        );
        for label in header.switcher_labels() {
            narrow.add_setter(label, "visible", Some(&false.to_value()));
        }
        narrow.add_setter(header.scene_label(), "visible", Some(&false.to_value()));
        narrow.add_setter(header.subtitle(), "visible", Some(&false.to_value()));
        narrow.add_setter(&header.open, "visible", Some(&false.to_value()));
        if tiny {
            narrow.add_setter(&header.title_box, "visible", Some(&false.to_value()));
            narrow.add_setter(&header.forward, "visible", Some(&false.to_value()));
            narrow.add_setter(
                &header.switcher,
                "css-classes",
                Some(&["stave-switcher", "compact"].to_value()),
            );
        }
        window.add_breakpoint(narrow);
    }

    Widgets {
        window,
        header,
        toasts,
        banner,
        views,
        outer,
        inner,
        sidebar,
        tree,
        problems,
        inspector,
        view_stack,
        status_page,
        viewport,
        motion,
        files,
        status,
    }
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
