//! The state of one open project in a viewer, without any UI: which view
//! is showing, what is selected, where back and forward go, the scene and
//! how far it is pulled apart, and how the background check is going.
//!
//! Views never change this state directly. They send a [`Command`] to
//! [`Session::apply`] and redraw what the returned [`Event`]s name. A
//! check is run by the viewer, off the main thread, when an event asks for
//! one ([`Event::CheckStarted`]); its [`Outcome`] goes back through
//! [`Session::finish_check`], which replaces the old result whole (a
//! stale run's outcome is dropped). Ids in the selection and the history
//! are carried over to the new result by name, so a selection survives
//! editing the file it is in.
//!
//! `docs/gui.md` describes how the window's views use this contract.

use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use stave_model::solved::{AssemblyId, HardwareId, JointId, PairMember, PartId, SolvedDesign};
use stave_scene::{Item, SceneEntry, SceneGraph};

use crate::check::{Checked, Outcome, Summary};
use crate::empty::Reason;
use crate::focus::Target;
use crate::view::Problem;

/// The most places the back and forward lists keep.
pub const HISTORY_LIMIT: usize = 100;

/// The window's three views.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum View {
    /// The parts tree, the 3D view and the inspector.
    #[default]
    Parts,
    /// The cut list.
    CutList,
    /// The project's files and the editor.
    Files,
}

impl View {
    /// Every view, in the switcher's order.
    pub const ALL: [View; 3] = [View::Parts, View::CutList, View::Files];

    /// Its name in the switcher: "Parts", "Cut list", "Files".
    pub fn title(self) -> &'static str {
        match self {
            View::Parts => "Parts",
            View::CutList => "Cut list",
            View::Files => "Files",
        }
    }

    /// A stable word for it (`parts`, `cut-list`, `files`), for actions and
    /// the command line.
    pub fn word(self) -> &'static str {
        match self {
            View::Parts => "parts",
            View::CutList => "cut-list",
            View::Files => "files",
        }
    }

    /// The view a word names.
    pub fn from_word(word: &str) -> Option<View> {
        View::ALL.into_iter().find(|view| view.word() == word)
    }
}

/// A line number in a file, from 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LineNumber(NonZeroU32);

impl LineNumber {
    /// The line numbered `number`; `None` for 0.
    pub fn new(number: u32) -> Option<LineNumber> {
        NonZeroU32::new(number).map(LineNumber)
    }

    /// The line number of a source map's 1-based `usize` line, saturating
    /// at `u32::MAX` (files are at most 4 GiB).
    pub fn from_usize(number: usize) -> Option<LineNumber> {
        LineNumber::new(u32::try_from(number).unwrap_or(u32::MAX))
    }

    /// The number.
    pub fn get(self) -> u32 {
        self.0.get()
    }
}

impl std::fmt::Display for LineNumber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A place in a source file: what a file:line link points at.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SourceLocation {
    /// The file's path as the source map names it: absolute for the
    /// project's own files, `std/...` for the standard library.
    pub path: String,
    /// The line.
    pub line: LineNumber,
}

impl SourceLocation {
    /// `doors.stave:9`: the path relative to `folder` when it is inside it.
    pub fn display(&self, folder: &Path) -> String {
        let path = Path::new(&self.path);
        let shown = path.strip_prefix(folder).unwrap_or(path);
        format!("{}:{}", shown.display(), self.line)
    }
}

/// One of the current result's problems, by its place in `stave check`
/// order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ProblemIndex(pub usize);

/// What is selected: one thing the views can all point at.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Selection {
    /// A part.
    Part(PartId),
    /// A piece of hardware.
    Hardware(HardwareId),
    /// One pair of a joint.
    Joint {
        /// The joint.
        joint: JointId,
        /// The pair's index in the joint.
        pair: usize,
    },
    /// The member a joint lays out on one pair (a dowel's pins).
    Member(PairMember),
    /// An assembly (a door, a drawer).
    Assembly(AssemblyId),
    /// A place in a file.
    Location(SourceLocation),
    /// A problem.
    Problem(ProblemIndex),
}

impl Selection {
    /// The selection a scene item is.
    pub fn of_item(item: Item) -> Selection {
        match item {
            Item::Part(id) => Selection::Part(id),
            Item::Hardware(id) => Selection::Hardware(id),
            Item::Member(member) => Selection::Member(member),
        }
    }

    /// The scene item it is, when it is a part, a piece of hardware or a
    /// joint's member.
    pub fn item(&self) -> Option<Item> {
        match self {
            Selection::Part(id) => Some(Item::Part(*id)),
            Selection::Hardware(id) => Some(Item::Hardware(*id)),
            Selection::Member(member) => Some(Item::Member(*member)),
            Selection::Joint { .. }
            | Selection::Assembly(_)
            | Selection::Location(_)
            | Selection::Problem(_) => None,
        }
    }

    /// The selection a focus target is about.
    pub fn of_target(target: Target) -> Selection {
        match target {
            Target::Joint { joint, pair } => Selection::Joint { joint, pair },
            Target::Hardware(id) => Selection::Hardware(id),
        }
    }
}

/// Where the window is: one entry of the back and forward history.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Place {
    /// The view showing.
    pub view: View,
    /// What is selected.
    pub selection: Option<Selection>,
    /// Focus mode on a joint or a piece of hardware, in the Parts view.
    pub focus: Option<Target>,
}

/// What the Parts view's left sidebar shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum Sidebar {
    /// The parts tree.
    #[default]
    Parts,
    /// The problems list (opened by the status pill).
    Problems,
}

/// How far the parts are pulled apart: 0 (as built) to 1 (fully apart).
/// A display amount for the renderer, not a measurement.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default)]
pub struct Explode(f32);

/// An explode amount outside 0 to 1.
#[derive(Debug, Clone, Copy, PartialEq, thiserror::Error)]
#[error("the amount must be between 0 and 1, not {0}")]
pub struct ExplodeOutOfRange(pub f32);

impl Explode {
    /// As built.
    pub const TOGETHER: Explode = Explode(0.0);
    /// Fully apart.
    pub const APART: Explode = Explode(1.0);

    /// An amount from 0 to 1.
    pub fn new(amount: f32) -> Result<Explode, ExplodeOutOfRange> {
        if (0.0..=1.0).contains(&amount) {
            Ok(Explode(amount))
        } else {
            Err(ExplodeOutOfRange(amount))
        }
    }

    /// An amount held to 0 to 1 (a slider's value); not-a-number is 0.
    pub fn clamped(amount: f32) -> Explode {
        if amount.is_nan() {
            Explode::TOGETHER
        } else {
            Explode(amount.clamp(0.0, 1.0))
        }
    }

    /// The amount.
    pub fn get(self) -> f32 {
        self.0
    }

    /// A scene's own amount: apart for `explode: true`.
    pub fn of_scene(scene: &SceneEntry) -> Explode {
        if scene.explode {
            Explode::APART
        } else {
            Explode::TOGETHER
        }
    }
}

/// What the last finished check found, as the status shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The pipeline ran.
    Checked(Summary),
    /// The folder couldn't be read.
    CantOpen(String),
}

/// How the background check is going.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum CheckStatus {
    /// Nothing has been checked yet.
    #[default]
    Idle,
    /// A check is running; the last result stays on show meanwhile.
    Checking {
        /// What the last finished check found, if any finished.
        last: Option<Verdict>,
    },
    /// The last check finished.
    Done(Verdict),
}

impl CheckStatus {
    /// What the last finished check found, whether or not one is running.
    pub fn verdict(&self) -> Option<&Verdict> {
        match self {
            CheckStatus::Idle => None,
            CheckStatus::Checking { last } => last.as_ref(),
            CheckStatus::Done(verdict) => Some(verdict),
        }
    }

    /// `true` while a check runs.
    pub fn is_checking(&self) -> bool {
        matches!(self, CheckStatus::Checking { .. })
    }

    /// The title bar's status pill.
    pub fn pill(&self) -> Pill {
        let activity = if self.is_checking() {
            Activity::Checking
        } else {
            Activity::Idle
        };
        let (tone, label) = match self.verdict() {
            None if self.is_checking() => (Tone::Unknown, "Checking…".to_owned()),
            None => (Tone::Unknown, "Not checked yet".to_owned()),
            Some(Verdict::CantOpen(_)) => (Tone::Errors, "Can't open".to_owned()),
            Some(Verdict::Checked(summary)) => {
                let tone = if summary.errors > 0 {
                    Tone::Errors
                } else if summary.warnings > 0 {
                    Tone::Warnings
                } else {
                    Tone::Clean
                };
                let label = match summary.problems() {
                    0 => "No problems".to_owned(),
                    1 => "1 problem".to_owned(),
                    count => format!("{count} problems"),
                };
                (tone, label)
            }
        };
        Pill {
            tone,
            label,
            activity,
        }
    }

    /// The status bar's line: "Checked 10 files · 0 errors · 0 warnings ·
    /// 26 ms", or "Checking again… last result: 1 error".
    pub fn line(&self) -> String {
        match self {
            CheckStatus::Idle => "Not checked yet".to_owned(),
            CheckStatus::Checking { last: None } => "Checking the project…".to_owned(),
            CheckStatus::Checking { last: Some(last) } => {
                let result = match last {
                    Verdict::Checked(summary) => {
                        let mut counts = Vec::new();
                        if summary.errors > 0 || summary.warnings == 0 {
                            counts.push(plural(summary.errors, "error", "errors"));
                        }
                        if summary.warnings > 0 {
                            counts.push(plural(summary.warnings, "warning", "warnings"));
                        }
                        counts.join(", ")
                    }
                    Verdict::CantOpen(_) => "the folder couldn't be opened".to_owned(),
                };
                format!("Checking again… last result: {result}")
            }
            CheckStatus::Done(Verdict::Checked(summary)) => format!(
                "Checked {} · {} · {} · {}",
                plural(summary.files, "file", "files"),
                plural(summary.errors, "error", "errors"),
                plural(summary.warnings, "warning", "warnings"),
                duration_text(summary.elapsed)
            ),
            CheckStatus::Done(Verdict::CantOpen(message)) => format!("Can't open: {message}"),
        }
    }
}

/// `1 file`, `10 files`.
fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// `26 ms`, `1.4 s`.
fn duration_text(elapsed: Duration) -> String {
    let millis = elapsed.as_millis();
    if millis < 1000 {
        format!("{millis} ms")
    } else {
        let tenths = millis / 100;
        format!("{}.{} s", tenths / 10, tenths % 10)
    }
}

/// The colour of the status pill.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tone {
    /// Not checked yet: neutral.
    Unknown,
    /// No problems: green.
    Clean,
    /// Warnings only: amber.
    Warnings,
    /// At least one error: red.
    Errors,
}

/// Whether a check is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Activity {
    /// Nothing running.
    Idle,
    /// A check is running: the pill shows a spinner in place of its icon.
    Checking,
}

/// The title bar's status pill: its colour, its words and whether it
/// spins. While a check runs it keeps its last colour and words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pill {
    /// Its colour.
    pub tone: Tone,
    /// "No problems", "3 problems".
    pub label: String,
    /// Whether a check is running.
    pub activity: Activity,
}

/// The build button's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Build {
    /// Ready.
    #[default]
    Idle,
    /// Building: checking first (the button is disabled and spins).
    Building,
}

/// A short message for the viewer to show as a toast.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Notice {
    /// The build plan was asked for, and the PDF doesn't exist yet.
    BuildPlanNotBuilt,
    /// The build plan was asked for, and the design has errors.
    BuildPlanHasErrors,
}

impl Notice {
    /// Its words.
    pub fn text(self) -> &'static str {
        match self {
            Notice::BuildPlanNotBuilt => "The build plan (PDF) isn't built yet",
            Notice::BuildPlanHasErrors => {
                "Fix the problems first: the build plan needs a design without errors"
            }
        }
    }
}

/// One check run, numbered so a stale run's outcome can be told apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CheckTicket(u64);

/// What a view asks the session to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Switch to a view.
    ShowView(View),
    /// Select something, or nothing.
    Select(Option<Selection>),
    /// Focus on a joint or a piece of hardware (the Parts view).
    Focus(Target),
    /// Leave focus mode, keeping the selection.
    LeaveFocus,
    /// Go back to the previous place (Alt+←).
    Back,
    /// Go forward again (Alt+→).
    Forward,
    /// Show a place in a file: switches to the Files view with it
    /// selected (a "Defined in the text" link).
    GoToSource(SourceLocation),
    /// The editor's cursor is on this item (or on nothing the views can
    /// show): select it where the window is, without a history entry, so
    /// the 3D view follows the text as the cursor moves.
    Link(Option<Selection>),
    /// Show the scene with this name (its own explode amount with it).
    ShowScene(String),
    /// Pull the parts apart this far.
    SetExplode(Explode),
    /// Show the parts tree or the problems list in the sidebar.
    ShowSidebar(Sidebar),
    /// Check the project again now.
    Check,
    /// Build the plan (Ctrl+B): checks first.
    Build,
}

/// What changed, for the views to redraw.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Event {
    /// [`Session::view`] changed.
    ViewChanged,
    /// [`Session::selection`] changed.
    SelectionChanged,
    /// [`Session::focus`] changed.
    FocusChanged,
    /// [`Session::can_go_back`] or [`Session::can_go_forward`] may have
    /// changed.
    HistoryChanged,
    /// [`Session::scene`] changed.
    SceneChanged,
    /// [`Session::explode`] changed.
    ExplodeChanged,
    /// [`Session::sidebar`] changed.
    SidebarChanged,
    /// [`Session::check_status`] changed.
    CheckStatusChanged,
    /// [`Session::build`] changed.
    BuildChanged,
    /// A new result replaced the old one: the design, the problems, the
    /// scene graph.
    ResultChanged,
    /// Run a check now, off the main thread, and hand its outcome to
    /// [`Session::finish_check`] with this ticket.
    CheckStarted(CheckTicket),
    /// Show this message.
    Notice(Notice),
}

/// Whether another check was asked for while one runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Queued {
    Nothing,
    Again,
}

/// One open project.
#[derive(Debug)]
pub struct Session {
    folder: PathBuf,
    place: Place,
    back: Vec<Place>,
    forward: Vec<Place>,
    scene: Option<String>,
    explode: Explode,
    sidebar: Sidebar,
    status: CheckStatus,
    build: Build,
    next_ticket: u64,
    running: Option<CheckTicket>,
    queued: Queued,
    /// The last check's result.
    latest: Option<Rc<Checked>>,
    /// The last result that solved: what the 3D view shows.
    shown: Option<Rc<Checked>>,
}

impl Session {
    /// A session for a project folder, before its first check.
    pub fn new(folder: PathBuf) -> Session {
        Session {
            folder,
            place: Place::default(),
            back: Vec::new(),
            forward: Vec::new(),
            scene: None,
            explode: Explode::TOGETHER,
            sidebar: Sidebar::Parts,
            status: CheckStatus::Idle,
            build: Build::Idle,
            next_ticket: 0,
            running: None,
            queued: Queued::Nothing,
            latest: None,
            shown: None,
        }
    }

    // ---- reading ----

    /// The project folder.
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// Where the window is.
    pub fn place(&self) -> &Place {
        &self.place
    }

    /// The view showing.
    pub fn view(&self) -> View {
        self.place.view
    }

    /// What is selected.
    pub fn selection(&self) -> Option<&Selection> {
        self.place.selection.as_ref()
    }

    /// Focus mode's target, when on.
    pub fn focus(&self) -> Option<Target> {
        self.place.focus
    }

    /// Whether Back has somewhere to go.
    pub fn can_go_back(&self) -> bool {
        !self.back.is_empty()
    }

    /// Whether Forward has somewhere to go (only after going back).
    pub fn can_go_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    /// The name of the scene asked for (`None`: the first, `rest`).
    pub fn scene(&self) -> Option<&str> {
        self.scene.as_deref()
    }

    /// The index of the scene showing in the shown result's scene graph.
    pub fn scene_index(&self) -> usize {
        let Some(graph) = self.shown.as_ref().and_then(|c| c.scene.as_ref()) else {
            return 0;
        };
        self.scene
            .as_deref()
            .and_then(|name| graph.scenes.iter().position(|s| s.name == name))
            .unwrap_or(0)
    }

    /// The scene showing, from the shown result's scene graph.
    pub fn scene_entry(&self) -> Option<&SceneEntry> {
        self.shown
            .as_ref()
            .and_then(|c| c.scene.as_ref())
            .and_then(|graph| graph.scenes.get(self.scene_index()))
    }

    /// How far the parts are pulled apart.
    pub fn explode(&self) -> Explode {
        self.explode
    }

    /// What the sidebar shows.
    pub fn sidebar(&self) -> Sidebar {
        self.sidebar
    }

    /// How the background check is going.
    pub fn check_status(&self) -> &CheckStatus {
        &self.status
    }

    /// The build button's state.
    pub fn build(&self) -> Build {
        self.build
    }

    /// The window's title: the project's or library's own declared title
    /// (`project "Kitchen wall cabinet"`, `library "Japanese joinery
    /// samples"`), falling back to the folder's name only when none is
    /// declared or nothing has been read yet.
    pub fn title(&self) -> String {
        self.latest
            .iter()
            .chain(self.shown.iter())
            .find_map(|checked| checked.declared_title())
            .map_or_else(
                || {
                    self.folder.file_name().map_or_else(
                        || self.folder.display().to_string(),
                        |name| name.to_string_lossy().into_owned(),
                    )
                },
                str::to_owned,
            )
    }

    /// Whether there is anything to build a plan for: `Ok` for a project
    /// whose latest check has parts and no errors, otherwise why not
    /// ([`Reason::nothing_to_build`] says it in words). The build button
    /// is enabled only when this is `Ok`, and [`Command::Build`] does
    /// nothing otherwise.
    pub fn buildable(&self) -> Result<(), Reason> {
        if let Some(Verdict::CantOpen(_)) = self.status.verdict() {
            return Err(Reason::CantOpen);
        }
        let Some(latest) = &self.latest else {
            return Err(Reason::Checking);
        };
        if latest.summary.errors > 0 {
            return Err(Reason::Errors);
        }
        match latest.design() {
            // No errors and nothing solved: a library, which places nothing.
            None => Err(Reason::Library),
            Some(design) if design.parts.is_empty() && design.hardware.is_empty() => {
                Err(Reason::NoParts)
            }
            Some(_) => Ok(()),
        }
    }

    /// The build button's tooltip: its name and shortcut, what it is
    /// doing, or why there is nothing to build.
    pub fn build_tooltip(&self) -> &'static str {
        match (self.build, self.buildable()) {
            (Build::Building, _) => "Building the plan…",
            (Build::Idle, Ok(())) => "Build plan (Ctrl+B)",
            (Build::Idle, Err(reason)) => reason.nothing_to_build(),
        }
    }

    /// The last check's result: its problems are the current problems.
    pub fn latest(&self) -> Option<&Rc<Checked>> {
        self.latest.as_ref()
    }

    /// The last result that solved: the design the 3D view, the tree and
    /// the inspector show. It can be older than [`Session::latest`] when
    /// the latest has errors that stop the design from solving.
    pub fn shown(&self) -> Option<&Rc<Checked>> {
        self.shown.as_ref()
    }

    /// The shown design and its scene graph.
    pub fn drawable(&self) -> Option<(&SolvedDesign, &SceneGraph)> {
        self.shown.as_ref().and_then(|c| c.drawable())
    }

    /// `true` when the latest check didn't solve and an older design is on
    /// show.
    pub fn showing_older(&self) -> bool {
        match (&self.latest, &self.shown) {
            (Some(latest), Some(shown)) => !Rc::ptr_eq(latest, shown),
            (None, Some(_)) => true,
            (_, None) => false,
        }
    }

    /// The current problems, in `stave check` order.
    pub fn problems(&self) -> &[Problem] {
        self.latest.as_ref().map_or(&[], |c| c.problems.as_slice())
    }

    /// The selected problem, when a problem is selected.
    pub fn selected_problem(&self) -> Option<&Problem> {
        match self.selection()? {
            Selection::Problem(ProblemIndex(index)) => self.problems().get(*index),
            _ => None,
        }
    }

    /// How each node of the shown design is drawn outside focus mode: when
    /// a joint's member is selected, or a selected problem is about one,
    /// the parts it sits in are see-through so it shows
    /// ([`crate::view::see_into`]). `None` draws every node as usual.
    pub fn looks(&self) -> Option<Vec<stave_scene::Look>> {
        let (design, graph) = self.drawable()?;
        let items: Vec<Item> = match self.selected_problem() {
            Some(problem) => problem
                .contact
                .iter()
                .flat_map(|contact| contact.items.iter().copied())
                .map(crate::view::contact_item)
                .collect(),
            None => self
                .selection()
                .and_then(Selection::item)
                .into_iter()
                .collect(),
        };
        crate::view::see_into(design, graph, &items)
    }

    // ---- changing ----

    /// Does what a view asks and says what changed.
    pub fn apply(&mut self, command: Command) -> Vec<Event> {
        let mut events = Vec::new();
        match command {
            Command::ShowView(view) => {
                if view != self.place.view {
                    let place = Place {
                        view,
                        ..self.place.clone()
                    };
                    self.go(place, &mut events);
                }
            }
            Command::Select(selection) => {
                if selection != self.place.selection || self.place.focus.is_some() {
                    let place = Place {
                        view: self.place.view,
                        selection,
                        focus: None,
                    };
                    self.go(place, &mut events);
                }
            }
            Command::Focus(target) => {
                if self.place.focus != Some(target) || self.place.view != View::Parts {
                    let place = Place {
                        view: View::Parts,
                        selection: Some(Selection::of_target(target)),
                        focus: Some(target),
                    };
                    self.go(place, &mut events);
                }
            }
            Command::LeaveFocus => {
                if self.place.focus.is_some() {
                    let place = Place {
                        focus: None,
                        ..self.place.clone()
                    };
                    self.go(place, &mut events);
                }
            }
            Command::Back => {
                if let Some(place) = self.back.pop() {
                    let current = std::mem::replace(&mut self.place, place);
                    push_limited(&mut self.forward, current);
                    self.changed_place_events(&mut events);
                }
            }
            Command::Forward => {
                if let Some(place) = self.forward.pop() {
                    let current = std::mem::replace(&mut self.place, place);
                    push_limited(&mut self.back, current);
                    self.changed_place_events(&mut events);
                }
            }
            Command::GoToSource(location) => {
                let place = Place {
                    view: View::Files,
                    selection: Some(Selection::Location(location)),
                    focus: None,
                };
                if place != self.place {
                    self.go(place, &mut events);
                }
            }
            Command::Link(selection) => {
                if selection != self.place.selection {
                    let before = self.place.clone();
                    self.place.selection = selection;
                    self.place.focus = None;
                    if before.selection != self.place.selection {
                        events.push(Event::SelectionChanged);
                    }
                    if before.focus.is_some() {
                        events.push(Event::FocusChanged);
                    }
                }
            }
            Command::ShowScene(name) => {
                let entry = self
                    .shown
                    .as_ref()
                    .and_then(|c| c.scene.as_ref())
                    .and_then(|graph| graph.scenes.iter().find(|s| s.name == name));
                if let Some(entry) = entry {
                    let explode = Explode::of_scene(entry);
                    if self.scene.as_deref() != Some(name.as_str()) {
                        self.scene = Some(name);
                        events.push(Event::SceneChanged);
                    }
                    if explode != self.explode {
                        self.explode = explode;
                        events.push(Event::ExplodeChanged);
                    }
                }
            }
            Command::SetExplode(explode) => {
                if explode != self.explode {
                    self.explode = explode;
                    events.push(Event::ExplodeChanged);
                }
            }
            Command::ShowSidebar(sidebar) => {
                if sidebar != self.sidebar {
                    self.sidebar = sidebar;
                    events.push(Event::SidebarChanged);
                }
            }
            Command::Check => self.request_check(&mut events),
            Command::Build => {
                if self.build == Build::Idle && self.buildable().is_ok() {
                    self.build = Build::Building;
                    events.push(Event::BuildChanged);
                    self.request_check(&mut events);
                }
            }
        }
        finish(events)
    }

    /// Hands back a check's outcome. A stale run's outcome (an older ticket
    /// than the one running) is dropped. The new result replaces the old
    /// one whole; the selection, focus and history are carried over by
    /// name, and anything that no longer exists is let go.
    pub fn finish_check(&mut self, ticket: CheckTicket, outcome: Outcome) -> Vec<Event> {
        let mut events = Vec::new();
        if self.running != Some(ticket) {
            return events;
        }
        self.running = None;
        let verdict = match outcome {
            Outcome::Checked(checked) => {
                let verdict = Verdict::Checked(checked.summary);
                let checked: Rc<Checked> = Rc::from(checked);
                self.adopt(checked, &mut events);
                verdict
            }
            Outcome::CantOpen { message, .. } => Verdict::CantOpen(message),
        };
        self.status = CheckStatus::Done(verdict);
        events.push(Event::CheckStatusChanged);
        if self.queued == Queued::Again {
            self.queued = Queued::Nothing;
            self.request_check(&mut events);
        } else if self.build == Build::Building {
            self.build = Build::Idle;
            events.push(Event::BuildChanged);
            let errors = match self.status.verdict() {
                Some(Verdict::Checked(summary)) => summary.errors > 0,
                Some(Verdict::CantOpen(_)) | None => true,
            };
            events.push(Event::Notice(if errors {
                Notice::BuildPlanHasErrors
            } else {
                Notice::BuildPlanNotBuilt
            }));
        }
        finish(events)
    }

    /// Starts a check, or asks for another once the running one is done.
    fn request_check(&mut self, events: &mut Vec<Event>) {
        if self.running.is_some() {
            self.queued = Queued::Again;
            return;
        }
        let ticket = CheckTicket(self.next_ticket);
        self.next_ticket = self.next_ticket.saturating_add(1);
        self.running = Some(ticket);
        let last = self.status.verdict().cloned();
        self.status = CheckStatus::Checking { last };
        events.push(Event::CheckStatusChanged);
        events.push(Event::CheckStarted(ticket));
    }

    /// Takes a new result: the latest always, the shown design when it
    /// solved, carrying ids over by name.
    fn adopt(&mut self, checked: Rc<Checked>, events: &mut Vec<Event>) {
        let old_shown = self.shown.clone();
        let old_problems = self.latest.clone();
        let solved = checked.drawable().is_some();
        self.latest = Some(checked.clone());
        if solved {
            self.shown = Some(checked.clone());
        }
        let new_shown = self.shown.clone();
        let carry = Carry {
            old: old_shown.as_deref().and_then(Checked::design),
            new: new_shown.as_deref().and_then(Checked::design),
            old_problems: old_problems
                .as_deref()
                .map_or(&[], |c| c.problems.as_slice()),
            new_problems: &checked.problems,
        };
        let before = self.place.clone();
        self.place = carry.place(&self.place);
        for place in self.back.iter_mut().chain(self.forward.iter_mut()) {
            *place = carry.place(place);
        }
        if self.place.selection != before.selection {
            events.push(Event::SelectionChanged);
        }
        if self.place.focus != before.focus {
            events.push(Event::FocusChanged);
        }
        // A scene that is gone falls back to the first.
        let scene_exists = self.scene.as_deref().is_none_or(|name| {
            self.shown
                .as_ref()
                .and_then(|c| c.scene.as_ref())
                .is_some_and(|graph| graph.scenes.iter().any(|s| s.name == name))
        });
        if !scene_exists {
            self.scene = None;
            events.push(Event::SceneChanged);
        }
        // The first design shows its scene's own explode amount.
        if old_shown.is_none()
            && let Some(entry) = self.scene_entry()
        {
            let explode = Explode::of_scene(entry);
            if explode != self.explode {
                self.explode = explode;
                events.push(Event::ExplodeChanged);
            }
        }
        events.push(Event::HistoryChanged);
        events.push(Event::ResultChanged);
    }

    /// Moves to a new place, remembering the current one for Back.
    fn go(&mut self, place: Place, events: &mut Vec<Event>) {
        let current = std::mem::replace(&mut self.place, place);
        let changed_from = current.clone();
        push_limited(&mut self.back, current);
        self.forward.clear();
        self.place_events(&changed_from, events);
    }

    fn changed_place_events(&self, events: &mut Vec<Event>) {
        // Back and Forward may change anything; say so.
        events.extend([
            Event::ViewChanged,
            Event::SelectionChanged,
            Event::FocusChanged,
            Event::HistoryChanged,
        ]);
    }

    fn place_events(&self, before: &Place, events: &mut Vec<Event>) {
        if before.view != self.place.view {
            events.push(Event::ViewChanged);
        }
        if before.selection != self.place.selection {
            events.push(Event::SelectionChanged);
        }
        if before.focus != self.place.focus {
            events.push(Event::FocusChanged);
        }
        events.push(Event::HistoryChanged);
    }
}

/// Keeps at most [`HISTORY_LIMIT`] places, dropping the oldest.
fn push_limited(list: &mut Vec<Place>, place: Place) {
    if list.last() == Some(&place) {
        return;
    }
    list.push(place);
    if list.len() > HISTORY_LIMIT {
        list.remove(0);
    }
}

/// Sorted and without repeats, so views redraw once per thing.
fn finish(mut events: Vec<Event>) -> Vec<Event> {
    events.sort();
    events.dedup();
    events
}

/// Carries ids from one result to the next by name.
struct Carry<'a> {
    old: Option<&'a SolvedDesign>,
    new: Option<&'a SolvedDesign>,
    old_problems: &'a [Problem],
    new_problems: &'a [Problem],
}

impl Carry<'_> {
    fn place(&self, place: &Place) -> Place {
        let selection = place.selection.as_ref().and_then(|s| self.selection(s));
        let focus = place.focus.and_then(|target| self.target(target));
        // Focus without its selection makes no sense.
        let focus = focus.filter(|target| selection == Some(Selection::of_target(*target)));
        Place {
            view: place.view,
            selection,
            focus,
        }
    }

    fn same_design(&self) -> bool {
        match (self.old, self.new) {
            (Some(old), Some(new)) => std::ptr::eq(old, new),
            _ => false,
        }
    }

    fn selection(&self, selection: &Selection) -> Option<Selection> {
        if let Selection::Problem(ProblemIndex(index)) = selection {
            let old = self.old_problems.get(*index)?;
            let key = |p: &Problem| (p.code.clone(), p.message.clone(), p.location.clone());
            let wanted = key(old);
            return self
                .new_problems
                .iter()
                .position(|p| key(p) == wanted)
                .map(|index| Selection::Problem(ProblemIndex(index)));
        }
        if let Selection::Location(location) = selection {
            return Some(Selection::Location(location.clone()));
        }
        if self.same_design() {
            return Some(selection.clone());
        }
        let (old, new) = (self.old?, self.new?);
        match selection {
            Selection::Part(id) => {
                let name = &old.part(*id)?.name;
                new.parts
                    .iter()
                    .find(|p| &p.name == name)
                    .map(|p| Selection::Part(p.id))
            }
            Selection::Hardware(id) => {
                let name = &old.hardware.get(id.index())?.name;
                new.hardware
                    .iter()
                    .find(|h| &h.name == name)
                    .map(|h| Selection::Hardware(h.id))
            }
            Selection::Assembly(id) => {
                let name = &old.assemblies.get(id.index())?.name;
                new.assemblies
                    .iter()
                    .find(|a| &a.name == name)
                    .map(|a| Selection::Assembly(a.id))
            }
            Selection::Joint { joint, pair } => {
                let name = &old.joints.get(joint.index())?.name;
                new.joints
                    .iter()
                    .find(|j| &j.name == name && *pair < j.pairs.len())
                    .map(|j| Selection::Joint {
                        joint: j.id,
                        pair: *pair,
                    })
            }
            Selection::Member(member) => {
                let joint = &old.joints.get(member.joint.index())?.name;
                let body = &old.pair_member(*member)?.name;
                new.pair_members()
                    .find(|(found, found_body)| {
                        found.pair == member.pair
                            && &found_body.name == body
                            && new
                                .joints
                                .get(found.joint.index())
                                .is_some_and(|j| &j.name == joint)
                    })
                    .map(|(found, _)| Selection::Member(found))
            }
            Selection::Location(_) | Selection::Problem(_) => None,
        }
    }

    fn target(&self, target: Target) -> Option<Target> {
        match self.selection(&Selection::of_target(target))? {
            Selection::Joint { joint, pair } => Some(Target::Joint { joint, pair }),
            Selection::Hardware(id) => Some(Target::Hardware(id)),
            Selection::Part(_)
            | Selection::Member(_)
            | Selection::Assembly(_)
            | Selection::Location(_)
            | Selection::Problem(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explode_amounts_stay_between_0_and_1() {
        assert!(Explode::new(1.5).is_err());
        assert!(Explode::new(-0.1).is_err());
        assert_eq!(Explode::new(0.5).map(Explode::get), Ok(0.5));
        assert_eq!(Explode::clamped(3.0), Explode::APART);
        assert_eq!(Explode::clamped(f32::NAN), Explode::TOGETHER);
    }

    #[test]
    fn durations_read_plainly() {
        assert_eq!(duration_text(Duration::from_millis(26)), "26 ms");
        assert_eq!(duration_text(Duration::from_millis(1440)), "1.4 s");
    }

    #[test]
    fn views_have_words() {
        for view in View::ALL {
            assert_eq!(View::from_word(view.word()), Some(view));
        }
        assert_eq!(View::from_word("cut-list"), Some(View::CutList));
    }

    #[test]
    fn locations_show_relative_to_the_project() {
        let location = SourceLocation {
            path: "/p/cabinet/doors.stave".to_owned(),
            line: LineNumber::new(9).unwrap(),
        };
        assert_eq!(location.display(Path::new("/p/cabinet")), "doors.stave:9");
        assert_eq!(LineNumber::new(0), None);
    }
}
