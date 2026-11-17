//! The libadwaita window: a header bar with the project title and a scene
//! menu, the 3D view, and a side panel with the selection and the
//! project's problems. The folder is watched; saving a file reloads the
//! design.
//!
//! Clicking a problem the checks can show poses the design where it
//! happens (`view::pose_at_contact`), tints its parts red and draws the
//! shared wood in red (`view::highlight`); a slider moves its motion. The
//! explode control pulls any scene apart (`SceneGraph::pose_with`), and
//! "Pull apart" eases it there and back.
//!
//! Everything about the design comes from `stave-app` (loading,
//! descriptions, problems) and `stave-scene` (poses, camera, picking);
//! this module only wires widgets to them.

mod panel;
mod viewport;

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use anyhow::{Context as _, Result, bail};
use gtk::{gdk, gio, glib};
use notify::Watcher as _;
use stave_app::Contact;
use stave_app::view::{self, Highlight, Item, Problem};
use stave_model::solved::SolvedDesign;
use stave_scene::{NodeId, Orbit, Posed, SceneGraph};

use crate::render::{Frame, Renderer};
use panel::Panel;
use viewport::Viewport;

/// How long to wait after a file changes before reloading, so an editor's
/// save (often several writes) reloads once.
const RELOAD_DELAY: Duration = Duration::from_millis(250);

/// Moving the mouse less than this many pixels between press and release
/// is a click, not a drag.
const CLICK_SLOP: f64 = 4.0;

/// How fast "Pull apart" moves the explode control: the whole way in about
/// 0.7 seconds.
const EXPLODE_PER_SECOND: f64 = 1.4;

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

/// How to open the window.
pub struct Options {
    /// The project folder.
    pub folder: PathBuf,
    /// The scene to start in.
    pub scene: Option<String>,
    /// The item to start with selected, by name.
    pub select: Option<String>,
    /// Save the window to this PNG once drawn, then close.
    pub window_screenshot: Option<PathBuf>,
    /// The problem to start showing (its index in the problems list).
    pub problem: Option<usize>,
    /// The explode amount to start with.
    pub explode: Option<f32>,
}

/// Opens the window and runs until it is closed.
pub fn run(options: Options) -> Result<()> {
    let Options {
        folder,
        scene,
        select,
        window_screenshot,
        problem,
        explode,
    } = options;
    let folder = folder
        .canonicalize()
        .with_context(|| format!("can't find the folder '{}'", folder.display()))?;
    if !folder.is_dir() {
        bail!("'{}' is not a folder", folder.display());
    }
    adw::init().context("can't start GTK (is there a display?)")?;
    let application = adw::Application::builder()
        .application_id("org.stave.Viewer")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    application.connect_activate(move |application| {
        Window::open(
            application,
            folder.clone(),
            Wanted {
                scene: scene.clone(),
                selection: select.clone(),
                problem,
                explode,
            },
            window_screenshot.clone(),
        );
    });
    let status = application.run_with_args::<&str>(&[]);
    if status != glib::ExitCode::SUCCESS {
        bail!("the viewer stopped with an error");
    }
    Ok(())
}

/// What the command line asks the window to start with.
struct Wanted {
    scene: Option<String>,
    selection: Option<String>,
    problem: Option<usize>,
    explode: Option<f32>,
}

/// The window's widgets.
struct Widgets {
    window: adw::ApplicationWindow,
    title: adw::WindowTitle,
    scenes: gtk::DropDown,
    viewport: Viewport,
    stack: gtk::Stack,
    status: adw::StatusPage,
    banner: adw::Banner,
    toasts: adw::ToastOverlay,
    split: adw::OverlaySplitView,
    panel: Panel,
}

/// The last design that solved, ready to draw.
struct Shown {
    design: SolvedDesign,
    scene: SceneGraph,
    scene_index: usize,
    posed: Posed,
}

/// What the window remembers between events.
struct State {
    folder: PathBuf,
    /// The scene asked for on the command line, until the first load.
    wanted_scene: Option<String>,
    /// The item asked for on the command line, until the first load.
    wanted_selection: Option<String>,
    /// The problem and explode amount asked for, until the first load.
    wanted_problem: Option<usize>,
    wanted_explode: Option<f32>,
    /// Where to save the window once drawn.
    window_screenshot: Option<PathBuf>,
    shown: Option<Shown>,
    orbit: Option<Orbit>,
    selection: Option<NodeId>,
    renderer: Result<Renderer, String>,
    /// The view's size in device pixels.
    size: (u32, u32),
    render_queued: bool,
    /// The drag offset already applied.
    drag_applied: (f64, f64),
    /// Ignore scene menu changes made while filling it.
    filling_scenes: bool,
    loads: usize,
    /// The problems of the last load, in the panel's order.
    problems: Vec<Problem>,
    /// The problem posed in the view, if any.
    problem: Option<ShownProblem>,
    /// How far apart the parts are pulled, 0 to 1.
    explode: f32,
    /// Where "Pull apart" is easing the explode control to.
    explode_target: Option<f32>,
    /// Ignore slider changes made by the window itself.
    updating_controls: bool,
}

struct Window {
    widgets: Widgets,
    state: RefCell<State>,
    /// Kept alive for as long as the window: dropping it stops watching.
    watcher: RefCell<Option<notify::RecommendedWatcher>>,
}

/// What a load produced, from the worker thread.
type Loaded = Result<(stave_app::Opened, Option<SceneGraph>), String>;

impl Window {
    fn open(
        application: &adw::Application,
        folder: PathBuf,
        wanted: Wanted,
        window_screenshot: Option<PathBuf>,
    ) {
        let widgets = build_widgets(application, &folder);
        let renderer = Renderer::new().map_err(|error| format!("{error:#}"));
        let window = Rc::new(Window {
            widgets,
            state: RefCell::new(State {
                folder,
                wanted_scene: wanted.scene,
                wanted_selection: wanted.selection,
                wanted_problem: wanted.problem,
                wanted_explode: wanted.explode,
                window_screenshot,
                shown: None,
                orbit: None,
                selection: None,
                renderer,
                size: (1, 1),
                render_queued: false,
                drag_applied: (0.0, 0.0),
                filling_scenes: false,
                loads: 0,
                problems: Vec::new(),
                problem: None,
                explode: 0.0,
                explode_target: None,
                updating_controls: false,
            }),
            watcher: RefCell::new(None),
        });
        window.connect_events();
        window.watch();
        window.widgets.window.present();
        let loading = window.clone();
        glib::spawn_future_local(async move { loading.load().await });
    }

    // ---- loading ----

    async fn load(self: &Rc<Self>) {
        let folder = self.state.borrow().folder.clone();
        let result: Loaded = gio::spawn_blocking(move || {
            stave_app::open(&folder)
                .map(|opened| {
                    let scene = opened.design.as_ref().map(view::scene);
                    (opened, scene)
                })
                .map_err(|error| format!("{error}"))
        })
        .await
        .unwrap_or_else(|_| Err("loading the design crashed".to_owned()));
        self.apply(result);
    }

    fn apply(self: &Rc<Self>, loaded: Loaded) {
        let widgets = &self.widgets;
        let first = {
            let mut state = self.state.borrow_mut();
            state.loads += 1;
            state.loads == 1
        };
        let (opened, scene) = match loaded {
            Ok(loaded) => loaded,
            Err(message) => {
                widgets.panel.show_problems(&[]);
                self.show_status("Can't open the folder", &message);
                return;
            }
        };
        let folder = self.state.borrow().folder.clone();
        let problems = view::problems(&opened, &folder);
        widgets.panel.show_problems(&problems);
        {
            // A reload renumbers the problems: stop showing the old one.
            let mut state = self.state.borrow_mut();
            state.problems = problems.clone();
            state.problem = None;
        }
        widgets.panel.show_problem(None, None);
        let errors = opened.error_count();
        match (opened.design, scene) {
            (Some(design), Some(scene)) => {
                widgets.title.set_title(&design.project.title);
                self.show_design(design, scene);
                widgets.banner.set_revealed(false);
                widgets.stack.set_visible_child_name("view");
                if !first {
                    let message = if errors == 0 && problems.is_empty() {
                        "Reloaded".to_owned()
                    } else {
                        format!(
                            "Reloaded, with {} to look at",
                            plural(problems.len(), "problem", "problems")
                        )
                    };
                    widgets
                        .toasts
                        .add_toast(adw::Toast::builder().title(message).timeout(2).build());
                }
            }
            _ => {
                let what = if errors == 0 {
                    "This folder is a library: it has nothing to place.".to_owned()
                } else {
                    format!(
                        "The design has {}. The side panel says what is wrong and where.",
                        plural(errors, "error", "errors")
                    )
                };
                widgets.split.set_show_sidebar(true);
                let showing_older = self.state.borrow().shown.is_some();
                if showing_older {
                    widgets.banner.set_title(&format!(
                        "{} Showing the last version that worked.",
                        what.split_once('.')
                            .map_or(what.as_str(), |(first, _)| first)
                    ));
                    widgets.banner.set_revealed(true);
                } else {
                    self.show_status("Nothing to show yet", &what);
                }
            }
        }
    }

    fn show_status(&self, title: &str, description: &str) {
        self.widgets.status.set_title(title);
        self.widgets.status.set_description(Some(description));
        self.widgets.stack.set_visible_child_name("status");
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
        glib::timeout_add_local(Duration::from_millis(400), move || {
            // Wait until the window is on screen and drawn.
            waited += 1;
            // A tick makes the frame clock run, so GTK draws the window.
            window.queue_draw();
            window.add_tick_callback(|_, _| glib::ControlFlow::Break);
            if (!window.is_mapped() || waited < 3) && waited < 25 {
                return glib::ControlFlow::Continue;
            }
            if let Err(error) = save_window(&window, &paintable, &path) {
                eprintln!("error: {error:#}");
            }
            window.close();
            glib::ControlFlow::Break
        });
    }

    /// Shows a freshly solved design, keeping the scene, the camera and
    /// the selection (by name) when they still exist.
    fn show_design(self: &Rc<Self>, design: SolvedDesign, scene: SceneGraph) {
        let mut state = self.state.borrow_mut();
        let previous_scene = state.wanted_scene.take().or_else(|| {
            let shown = state.shown.as_ref()?;
            Some(shown.scene.scenes.get(shown.scene_index)?.name.clone())
        });
        let previous_selection = state.wanted_selection.take().or_else(|| {
            let shown = state.shown.as_ref()?;
            Some(shown.scene.node(state.selection?)?.name.clone())
        });
        let first_load = state.shown.is_none();
        let scene_index = previous_scene
            .and_then(|name| scene.scenes.iter().position(|s| s.name == name))
            .unwrap_or(0);
        if first_load {
            state.explode = scene
                .scenes
                .get(scene_index)
                .map_or(0.0, |entry| if entry.explode { 1.0 } else { 0.0 });
        }
        let posed = scene.scenes.get(scene_index).map_or_else(
            || scene.rest(),
            |entry| scene.pose_with(entry, &[], state.explode),
        );
        state.selection = previous_selection.and_then(|name| {
            scene
                .node_ids()
                .find(|id| scene.node(*id).is_some_and(|node| node.name == name))
        });
        if let Ok(renderer) = &mut state.renderer {
            renderer.upload(&scene);
        }
        let titles: Vec<String> = scene.scenes.iter().map(|s| s.title.clone()).collect();
        state.shown = Some(Shown {
            design,
            scene,
            scene_index,
            posed,
        });
        if state.orbit.is_none() {
            state.orbit = Self::frame_all(&state);
        }
        state.filling_scenes = true;
        drop(state);
        let titles: Vec<&str> = titles.iter().map(String::as_str).collect();
        self.widgets
            .scenes
            .set_model(Some(&gtk::StringList::new(&titles)));
        self.widgets
            .scenes
            .set_selected(u32::try_from(scene_index).unwrap_or(0));
        self.state.borrow_mut().filling_scenes = false;
        let (problem, explode) = {
            let mut state = self.state.borrow_mut();
            (state.wanted_problem.take(), state.wanted_explode.take())
        };
        if let Some(explode) = explode {
            let mut state = self.state.borrow_mut();
            state.explode = explode.clamp(0.0, 1.0);
            Self::repose(&mut state);
        }
        self.sync_explode_control();
        if let Some(index) = problem {
            self.show_problem(index);
        }
        self.update_panel();
        if let Err(message) = &self.state.borrow().renderer {
            self.show_status("Can't draw in 3D", message);
        }
        self.request_render();
    }

    // ---- camera and selection ----

    fn aspect(state: &State) -> f32 {
        crate::render::aspect(state.size.0, state.size.1)
    }

    /// A camera that shows everything shown in the current scene.
    fn frame_all(state: &State) -> Option<Orbit> {
        let shown = state.shown.as_ref()?;
        let bounds = shown.scene.bounds(&shown.posed)?;
        let aspect = Self::aspect(state);
        Some(match state.orbit {
            Some(orbit) => orbit.framed(bounds, aspect),
            None => Orbit::framing(bounds, aspect),
        })
    }

    /// F: frames the selection, or everything.
    fn frame(self: &Rc<Self>) {
        let mut state = self.state.borrow_mut();
        let aspect = Self::aspect(&state);
        let selected = state.selection.and_then(|id| {
            let shown = state.shown.as_ref()?;
            let bounds = shown.scene.bounds_of(&shown.posed, &[id])?;
            Some(state.orbit?.framed(bounds, aspect))
        });
        state.orbit = selected.or_else(|| Self::frame_all(&state));
        drop(state);
        self.request_render();
    }

    fn select(self: &Rc<Self>, node: Option<NodeId>) {
        self.state.borrow_mut().selection = node;
        self.update_panel();
        self.request_render();
    }

    fn update_panel(&self) {
        let state = self.state.borrow();
        let description = state.shown.as_ref().and_then(|shown| {
            let node = shown.scene.node(state.selection?)?;
            view::describe(&shown.design, node.item)
        });
        self.widgets.panel.show_selection(description.as_ref());
    }

    fn pick(self: &Rc<Self>, x: f64, y: f64) {
        let hit = {
            let state = self.state.borrow();
            let viewport = &self.widgets.viewport;
            state
                .shown
                .as_ref()
                .zip(state.orbit)
                .and_then(|(shown, orbit)| {
                    let ray = orbit.ray(
                        crate::convert::f32_of(x),
                        crate::convert::f32_of(y),
                        crate::convert::f32_of_int(viewport.width()),
                        crate::convert::f32_of_int(viewport.height()),
                    );
                    shown.scene.pick(&shown.posed, ray)
                })
        };
        let node = hit.map(|hit| hit.node);
        if let Some(hit) = hit
            && let Item::Part(_) | Item::Hardware(_) = hit.item
        {
            self.select(node);
        } else {
            self.select(None);
        }
    }

    fn set_scene(self: &Rc<Self>, index: usize) {
        let mut state = self.state.borrow_mut();
        if state.filling_scenes {
            return;
        }
        state.problem = None;
        state.explode_target = None;
        let explode = state
            .shown
            .as_ref()
            .and_then(|shown| shown.scene.scenes.get(index))
            .map(|entry| if entry.explode { 1.0 } else { 0.0 });
        if let Some(shown) = state.shown.as_mut() {
            shown.scene_index = index;
        }
        if let Some(explode) = explode {
            state.explode = explode;
        }
        Self::repose(&mut state);
        drop(state);
        self.widgets.panel.show_problem(None, None);
        self.sync_explode_control();
        self.request_render();
    }

    /// Poses the design for what the window shows: a problem at its
    /// contact, or the current scene pulled apart by the explode amount.
    fn repose(state: &mut State) {
        let State {
            shown,
            problem,
            explode,
            ..
        } = state;
        let Some(shown) = shown.as_mut() else {
            return;
        };
        match problem {
            Some(problem) => {
                if let Some(posed) =
                    view::pose_at_contact(&shown.scene, &problem.contact, problem.amount)
                {
                    shown.posed = posed;
                }
            }
            None => {
                if let Some(entry) = shown.scene.scenes.get(shown.scene_index) {
                    shown.posed = shown.scene.pose_with(entry, &[], *explode);
                }
            }
        }
    }

    /// Shows the slider and toggle at the window's explode amount.
    fn sync_explode_control(&self) {
        let explode = {
            let mut state = self.state.borrow_mut();
            state.updating_controls = true;
            state.explode
        };
        let panel = &self.widgets.panel;
        panel.explode.set_value(f64::from(explode));
        panel.animate.set_active(explode >= 0.5);
        self.state.borrow_mut().updating_controls = false;
    }

    // ---- problems ----

    /// Poses the design where problem `index` happens.
    fn show_problem(self: &Rc<Self>, index: usize) {
        let mut state = self.state.borrow_mut();
        let Some(problem) = state.problems.get(index).cloned() else {
            return;
        };
        let (Some(contact), Some(shown)) = (problem.contact, state.shown.as_ref()) else {
            return;
        };
        let highlight = view::highlight(&shown.scene, &contact);
        let range = view::contact_motion_range(&shown.scene, &contact);
        let shown_amount = contact
            .motion
            .and_then(|motion| shown.scene.amount(motion.assembly, motion.shown));
        let turns = range.is_some_and(|r| r.turns);
        let slider = range.zip(shown_amount).map(|(range, amount)| {
            let (full, value) = if range.turns {
                (range.full.to_degrees(), amount.to_degrees())
            } else {
                (range.full, amount)
            };
            panel::MotionSlider {
                full: f64::from(full),
                value: f64::from(value),
                unit: if range.turns {
                    "degrees"
                } else {
                    "millimetres"
                },
            }
        });
        state.explode_target = None;
        state.problem = Some(ShownProblem {
            contact,
            highlight,
            amount: None,
            turns,
        });
        Self::repose(&mut state);
        // Frame the parts involved.
        let aspect = Self::aspect(&state);
        let framed = state.shown.as_ref().and_then(|shown| {
            let nodes = &state.problem.as_ref()?.highlight.nodes;
            let bounds = shown.scene.bounds_of(&shown.posed, nodes)?;
            Some(state.orbit?.framed(bounds, aspect))
        });
        if framed.is_some() {
            state.orbit = framed;
        }
        state.updating_controls = true;
        drop(state);
        self.widgets
            .panel
            .show_problem(Some(&problem.message), slider);
        self.state.borrow_mut().updating_controls = false;
        self.request_render();
    }

    /// The motion slider moved: degrees or millimetres.
    fn scrub(self: &Rc<Self>, value: f64) {
        let mut state = self.state.borrow_mut();
        if state.updating_controls {
            return;
        }
        let Some(problem) = state.problem.as_mut() else {
            return;
        };
        let value = narrow(value);
        problem.amount = Some(if problem.turns {
            value.to_radians()
        } else {
            value
        });
        Self::repose(&mut state);
        drop(state);
        self.request_render();
    }

    fn clear_problem(self: &Rc<Self>) {
        let mut state = self.state.borrow_mut();
        state.problem = None;
        Self::repose(&mut state);
        drop(state);
        self.widgets.panel.show_problem(None, None);
        self.request_render();
    }

    // ---- exploded view ----

    /// The explode slider moved.
    fn set_explode(self: &Rc<Self>, value: f64) {
        let mut state = self.state.borrow_mut();
        if state.updating_controls {
            return;
        }
        state.explode = narrow(value).clamp(0.0, 1.0);
        state.explode_target = None;
        state.problem = None;
        Self::repose(&mut state);
        drop(state);
        self.widgets.panel.show_problem(None, None);
        self.request_render();
    }

    /// "Pull apart" toggled: eases the explode amount to 1 or 0.
    fn animate_explode(self: &Rc<Self>, apart: bool) {
        {
            let mut state = self.state.borrow_mut();
            if state.updating_controls {
                return;
            }
            let already = state.explode_target.is_some();
            state.explode_target = Some(if apart { 1.0 } else { 0.0 });
            state.problem = None;
            if already {
                return;
            }
        }
        self.widgets.panel.show_problem(None, None);
        let window = self.clone();
        let last: std::cell::Cell<Option<i64>> = std::cell::Cell::new(None);
        self.widgets.viewport.add_tick_callback(move |_, clock| {
            let now = clock.frame_time();
            let seconds = last.get().map_or(0.0, |before| {
                // Microseconds between frames; a frame is never days.
                f64::from(i32::try_from(now - before).unwrap_or(0)) / 1e6
            });
            last.set(Some(now));
            let mut state = window.state.borrow_mut();
            let Some(target) = state.explode_target else {
                return glib::ControlFlow::Break;
            };
            let step = narrow(seconds * EXPLODE_PER_SECOND);
            let current = state.explode;
            let next = if target > current {
                (current + step).min(target)
            } else {
                (current - step).max(target)
            };
            state.explode = next;
            let done = (next - target).abs() < 1e-4;
            if done {
                state.explode_target = None;
            }
            Self::repose(&mut state);
            state.updating_controls = true;
            drop(state);
            window.widgets.panel.explode.set_value(f64::from(next));
            window.state.borrow_mut().updating_controls = false;
            window.request_render();
            if done {
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }

    fn update_orbit(self: &Rc<Self>, change: impl FnOnce(Orbit) -> Orbit) {
        let mut state = self.state.borrow_mut();
        state.orbit = state.orbit.map(change);
        drop(state);
        self.request_render();
    }

    // ---- drawing ----

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

    fn render_now(&self) {
        let mut state = self.state.borrow_mut();
        state.render_queued = false;
        let (width, height) = state.size;
        let State {
            shown,
            orbit,
            selection,
            renderer,
            problem,
            ..
        } = &mut *state;
        let (Some(shown), Some(orbit), Ok(renderer)) = (shown.as_ref(), *orbit, renderer) else {
            return;
        };
        // The shared wood is drawn only where it was measured; moved by the
        // slider, the parts stay tinted.
        let highlight = problem.as_ref().map(|problem| match problem.amount {
            None => problem.highlight.clone(),
            Some(_) => Highlight {
                nodes: problem.highlight.nodes.clone(),
                volumes: Vec::new(),
            },
        });
        let frame = Frame {
            scene: &shown.scene,
            posed: &shown.posed,
            orbit,
            selection: *selection,
            highlight: highlight.as_ref(),
            section: shown
                .scene
                .scenes
                .get(shown.scene_index)
                .and_then(|entry| entry.section),
            width,
            height,
        };
        match renderer.render(&frame) {
            Ok(image) => {
                let stride = usize::try_from(image.width).unwrap_or(0) * 4;
                let (Ok(width), Ok(height)) =
                    (i32::try_from(image.width), i32::try_from(image.height))
                else {
                    return;
                };
                let bytes = glib::Bytes::from_owned(image.rgba);
                let texture = gdk::MemoryTexture::new(
                    width,
                    height,
                    gdk::MemoryFormat::R8g8b8a8,
                    &bytes,
                    stride,
                );
                self.widgets.viewport.show_frame(texture.upcast());
                // Save the window once a frame of the view's real size is up.
                let settled = state.size.0 > 1 && state.size == self.widgets.viewport.device_size();
                drop(state);
                if settled {
                    self.capture_if_asked();
                }
            }
            Err(error) => {
                let message = format!("{error:#}");
                drop(state);
                self.show_status("Can't draw in 3D", &message);
            }
        }
    }

    // ---- events ----

    fn connect_events(self: &Rc<Self>) {
        let widgets = &self.widgets;
        let viewport = &widgets.viewport;

        let window = self.clone();
        viewport.connect_resized(move |width, height| {
            let changed = {
                let mut state = window.state.borrow_mut();
                let changed = state.size != (width, height);
                // The first camera was framed before the view had a size.
                if state.size.0 <= 1 && width > 1 {
                    state.size = (width, height);
                    state.orbit = Self::frame_all(&state);
                }
                state.size = (width, height);
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

        let keys = gtk::EventControllerKey::new();
        let window = self.clone();
        keys.connect_key_pressed(move |_, key, _, _| match key {
            gdk::Key::Escape => {
                window.select(None);
                glib::Propagation::Stop
            }
            gdk::Key::f | gdk::Key::F => {
                window.frame();
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        });
        widgets.window.add_controller(keys);

        let frame = gio::SimpleAction::new("frame", None);
        let window = self.clone();
        frame.connect_activate(move |_, _| window.frame());
        widgets.window.add_action(&frame);

        let window = self.clone();
        widgets.scenes.connect_selected_notify(move |dropdown| {
            if let Ok(index) = usize::try_from(dropdown.selected()) {
                window.set_scene(index);
            }
        });

        let window = Rc::downgrade(self);
        widgets.panel.connect_problem(move |index| {
            if let Some(window) = window.upgrade() {
                window.show_problem(index);
            }
        });
        let window = self.clone();
        widgets
            .panel
            .motion
            .connect_value_changed(move |scale| window.scrub(scale.value()));
        let window = self.clone();
        widgets
            .panel
            .close
            .connect_clicked(move |_| window.clear_problem());
        let window = self.clone();
        widgets
            .panel
            .explode
            .connect_value_changed(move |scale| window.set_explode(scale.value()));
        let window = self.clone();
        widgets
            .panel
            .animate
            .connect_toggled(move |button| window.animate_explode(button.is_active()));
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

    /// Watches the project folder and reloads after a `.stave` file
    /// changes.
    fn watch(self: &Rc<Self>) {
        let (sender, receiver) = async_channel::unbounded::<()>();
        let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            let Ok(event) = event else { return };
            let relevant = !matches!(event.kind, notify::EventKind::Access(_))
                && event.paths.iter().any(|path| is_source(path));
            if relevant {
                // The window may be gone; then nobody needs to know.
                let _ = sender.send_blocking(());
            }
        });
        let folder = self.state.borrow().folder.clone();
        let watcher = watcher.and_then(|mut watcher| {
            watcher.watch(&folder, notify::RecursiveMode::Recursive)?;
            Ok(watcher)
        });
        match watcher {
            Ok(watcher) => {
                self.watcher.replace(Some(watcher));
            }
            Err(error) => {
                eprintln!(
                    "warning: not watching '{}' for changes: {error}",
                    folder.display()
                );
                return;
            }
        }
        let window = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            while receiver.recv().await.is_ok() {
                glib::timeout_future(RELOAD_DELAY).await;
                while receiver.try_recv().is_ok() {}
                let Some(window) = window.upgrade() else {
                    break;
                };
                window.load().await;
            }
        });
    }
}

/// Draws the whole window into a PNG.
fn save_window(
    window: &adw::ApplicationWindow,
    paintable: &gtk::WidgetPaintable,
    path: &Path,
) -> Result<()> {
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(
        &snapshot,
        f64::from(window.width()),
        f64::from(window.height()),
    );
    let node = snapshot.to_node().context("the window drew nothing")?;
    // A software renderer of our own: it works on every display backend.
    let renderer = gtk::gsk::CairoRenderer::new();
    renderer
        .realize_for_display(&gtk::prelude::WidgetExt::display(window))
        .context("can't start a renderer for the window")?;
    let texture = renderer.render_texture(&node, None);
    renderer.unrealize();
    texture
        .save_to_png(path)
        .with_context(|| format!("can't write '{}'", path.display()))?;
    eprintln!("wrote {}", path.display());
    Ok(())
}

/// A `.stave` file outside hidden and build folders.
fn is_source(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension == "stave")
        && !path.components().any(|part| {
            let name = part.as_os_str().to_string_lossy();
            name == "target" || (name.starts_with('.') && name.len() > 1 && name != "..")
        })
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

fn build_widgets(application: &adw::Application, folder: &Path) -> Widgets {
    let folder_name = folder.file_name().map_or_else(
        || folder.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    let title = adw::WindowTitle::new(&folder_name, &folder.display().to_string());
    let scenes = gtk::DropDown::builder()
        .tooltip_text("Scene")
        .model(&gtk::StringList::new(&["As built"]))
        .build();
    // Icon names differ between icon themes: offer each a name it knows.
    let icon = |names: &[&str]| gtk::Image::from_gicon(&gio::ThemedIcon::from_names(names));
    let frame_button = gtk::Button::builder()
        .child(&icon(&["zoom-fit-best-symbolic", "zoom-fit-best"]))
        .tooltip_text("Frame everything (F)")
        .build();
    let sidebar_button = gtk::ToggleButton::builder()
        .child(&icon(&[
            "sidebar-show-right-symbolic",
            "sidebar-expand-right-symbolic",
            "view-right-pane-symbolic",
        ]))
        .tooltip_text("Side panel")
        .active(true)
        .build();
    let header = adw::HeaderBar::builder().title_widget(&title).build();
    header.pack_start(&scenes);
    header.pack_end(&sidebar_button);
    header.pack_end(&frame_button);

    let viewport = Viewport::default();
    let status = adw::StatusPage::builder()
        .icon_name("dialog-warning-symbolic")
        .title("Loading…")
        .build();
    let stack = gtk::Stack::new();
    stack.add_named(&viewport, Some("view"));
    stack.add_named(&status, Some("status"));
    stack.set_visible_child_name("status");
    let banner = adw::Banner::new("");
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&banner);
    content.append(&stack);

    let panel = Panel::new();
    let split = adw::OverlaySplitView::builder()
        .content(&content)
        .sidebar(&panel.root)
        .sidebar_position(gtk::PackType::End)
        .min_sidebar_width(300.0)
        .max_sidebar_width(380.0)
        .show_sidebar(true)
        .build();
    sidebar_button
        .bind_property("active", &split, "show-sidebar")
        .bidirectional()
        .build();
    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&split));
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&toasts));
    let window = adw::ApplicationWindow::builder()
        .application(application)
        .title(format!("{folder_name} — Stave"))
        .default_width(1400)
        .default_height(900)
        .content(&toolbar)
        .build();
    let breakpoint = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
        adw::BreakpointConditionLengthType::MaxWidth,
        600.0,
        adw::LengthUnit::Sp,
    ));
    breakpoint.add_setter(&split, "collapsed", Some(&true.to_value()));
    window.add_breakpoint(breakpoint);
    frame_button.set_action_name(Some("win.frame"));
    Widgets {
        window,
        title,
        scenes,
        viewport,
        stack,
        status,
        banner,
        toasts,
        split,
        panel,
    }
}

/// A control's `f64` value as the `f32` the scene works in.
fn narrow(value: f64) -> f32 {
    crate::convert::f32_of(value)
}
