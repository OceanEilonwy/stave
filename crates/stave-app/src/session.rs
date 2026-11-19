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
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use stave_model::solved::{AssemblyId, HardwareId, JointId, PairMember, PartId, SolvedDesign};
use stave_modules::{DiskFiles, FileSource};
use stave_scene::{Item, SceneEntry, SceneGraph};

use crate::change::{Change, ChangeError, ChangeRequest, MirrorOffer};
use crate::check::{Checked, Outcome, Summary};
use crate::editing::{Commit, Edit, EditCommand, EditEvent, FileEdit, Rejection, WriteEdits};
use crate::empty::Reason;
use crate::focus::Target;
use crate::gripping::{FoundStops, GripList, GripsTicket, StopsJob};
use crate::scratch::DiskWriter;
use crate::view::Problem;

/// The most places the back and forward lists keep.
pub const HISTORY_LIMIT: usize = 100;

/// The window's two views.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum View {
    /// The design: the 3D view filling the window, the model tree over it,
    /// the selection's card and the editor that slides in from the right.
    #[default]
    Design,
    /// The cut list.
    CutList,
}

impl View {
    /// Every view, in the switcher's order.
    pub const ALL: [View; 2] = [View::Design, View::CutList];

    /// Its name in the switcher: "Design", "Cut list".
    pub fn title(self) -> &'static str {
        match self {
            View::Design => "Design",
            View::CutList => "Cut list",
        }
    }

    /// A stable word for it (`design`, `cut-list`), for actions and the
    /// command line.
    pub fn word(self) -> &'static str {
        match self {
            View::Design => "design",
            View::CutList => "cut-list",
        }
    }

    /// The view a word names. The words of the older views, `parts` and
    /// `files`, both name the design (the text now opens beside it).
    pub fn from_word(word: &str) -> Option<View> {
        match word {
            "parts" | "files" => Some(View::Design),
            _ => View::ALL.into_iter().find(|view| view.word() == word),
        }
    }
}

/// Whether the editor is open beside the 3D view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum Editor {
    /// Closed: only the rail of open files shows at the right edge.
    #[default]
    Closed,
    /// Open: the text slides in from the right.
    Open,
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

    /// The place a `FILE[:LINE]` written on the command line names, the
    /// inverse of [`SourceLocation::display`]: a path relative to `folder`
    /// (or `std/...` for the standard library) with an optional line, the
    /// first line when none is given or it is not a number. `None` for
    /// line 0.
    pub fn parse(folder: &Path, text: &str) -> Option<SourceLocation> {
        let (path, line) = match text.rsplit_once(':') {
            Some((path, line)) => match line.parse::<u32>() {
                Ok(line) => (path, line),
                Err(_) => (text, 1),
            },
            None => (text, 1),
        };
        let path = if path.starts_with("std/") {
            path.to_owned()
        } else {
            folder.join(path).display().to_string()
        };
        Some(SourceLocation {
            path,
            line: LineNumber::new(line)?,
        })
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
    /// Focus mode on a joint or a piece of hardware, in the design.
    pub focus: Option<Target>,
    /// Whether the editor is open.
    pub editor: Editor,
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
                // Errors and warnings counted apart, so the words say
                // which there are without the pill's colour.
                let label = match (summary.errors, summary.warnings) {
                    (0, 0) => "No problems".to_owned(),
                    (errors, 0) => plural(errors, "error", "errors"),
                    (0, warnings) => plural(warnings, "warning", "warnings"),
                    (errors, warnings) => format!(
                        "{}, {}",
                        plural(errors, "error", "errors"),
                        plural(warnings, "warning", "warnings")
                    ),
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
    /// "No problems", "1 error", "2 warnings", "1 error, 2 warnings".
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
/// Numbers come from one counter for the whole process, never per
/// session: a window that opens another folder makes a new session while
/// the old one's check may still be running, and a fresh count would give
/// the new check the old one's number, so the old outcome would be taken
/// for the new folder's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CheckTicket(u64);

impl CheckTicket {
    /// A ticket no other check in this process has had.
    fn next() -> CheckTicket {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        CheckTicket(NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
    }
}

/// What a view asks the session to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Switch to a view.
    ShowView(View),
    /// Select something, or nothing.
    Select(Option<Selection>),
    /// Focus on a joint or a piece of hardware (the design).
    Focus(Target),
    /// Leave focus mode, keeping the selection.
    LeaveFocus,
    /// Go back to the previous place (Alt+←).
    Back,
    /// Go forward again (Alt+→).
    Forward,
    /// Show a place in a file: opens the editor with it selected (a
    /// "Defined in the text" link).
    GoToSource(SourceLocation),
    /// Open or close the editor, without a history entry.
    ShowEditor(Editor),
    /// The editor's cursor is on this item (or on nothing the views can
    /// show): select it where the window is, without a history entry, so
    /// the 3D view follows the text as the cursor moves.
    Link(Option<Selection>),
    /// Show the scene with this name (its own explode amount with it).
    ShowScene(String),
    /// Pull the parts apart this far.
    SetExplode(Explode),
    /// Select the next problem, from the first again after the last
    /// (F8).
    NextProblem,
    /// Select the previous problem, from the last again before the first
    /// (Shift+F8).
    PreviousProblem,
    /// Check the project again now.
    Check,
    /// Build the plan (Ctrl+B): checks first.
    Build,
    /// Drive the edit in progress (`docs/editing.md`): start one from a
    /// selection, choose and fill in a candidate, add it or cancel it.
    Edit(EditCommand),
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
    /// [`Session::editor`] changed.
    EditorChanged,
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
    /// [`Session::edit`] changed in this way.
    Edit(EditEvent),
}

/// Which way [`Command::NextProblem`] and [`Command::PreviousProblem`]
/// step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Next,
    Previous,
}

/// Whether another check was asked for while one runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Queued {
    Nothing,
    Again,
}

/// Where the edit in progress reads the project and writes its commits.
struct EditPorts {
    /// What an edit is started from: the disk, or the editor's buffers.
    files: Arc<dyn FileSource + Send + Sync>,
    /// Where Add writes.
    writer: Box<dyn WriteEdits>,
}

impl std::fmt::Debug for EditPorts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EditPorts").finish_non_exhaustive()
    }
}

impl Default for EditPorts {
    fn default() -> Self {
        EditPorts {
            files: Arc::new(DiskFiles),
            writer: Box::new(DiskWriter),
        }
    }
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
    status: CheckStatus,
    build: Build,
    running: Option<CheckTicket>,
    queued: Queued,
    /// The last check's result.
    latest: Option<Rc<Checked>>,
    /// The last result that solved: what the 3D view shows.
    shown: Option<Rc<Checked>>,
    /// The edit in progress.
    edit: Option<Edit>,
    /// The change of the design in progress (never at the same time as
    /// an edit).
    change: Option<Change>,
    /// What the last commit wrote.
    last_commit: Vec<FileEdit>,
    /// The grips listed for the selection, if any.
    grips: Option<GripList>,
    /// The ticket of the next listing of grips.
    next_grips: u64,
    /// Where it reads and writes.
    ports: EditPorts,
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
            status: CheckStatus::Idle,
            build: Build::Idle,
            running: None,
            queued: Queued::Nothing,
            latest: None,
            shown: None,
            edit: None,
            change: None,
            last_commit: Vec::new(),
            grips: None,
            next_grips: 0,
            ports: EditPorts::default(),
        }
    }

    /// Where an edit started from now on reads the project: the disk by
    /// default; a window passes its buffers' overlay
    /// ([`stave_ide::Buffers::overlay`]) so unsaved text is what is edited.
    pub fn set_edit_files(&mut self, files: Arc<dyn FileSource + Send + Sync>) {
        self.ports.files = files;
    }

    /// Where Add writes: the disk by default; a window passes its open
    /// buffers, so the editor and its undo see the change.
    pub fn set_edit_writer(&mut self, writer: Box<dyn WriteEdits>) {
        self.ports.writer = writer;
    }

    /// The edit in progress, if any.
    pub fn edit(&self) -> Option<&Edit> {
        self.edit.as_ref()
    }

    /// The change of the design in progress (a repeat, a shares grip, a
    /// mirror, a held rule, a removal, a rename), if any.
    pub fn change(&self) -> Option<&Change> {
        self.change.as_ref()
    }

    /// The files the last commit (an edit's or a change's Add) wrote, in
    /// path order; empty before the first.
    pub fn last_commit(&self) -> &[FileEdit] {
        &self.last_commit
    }

    /// What adding the mirror image of the assembly `item` would do, when
    /// it has one: the offer the card makes, read from the project as the
    /// edit files give it. `None` when it has none or the project doesn't
    /// place.
    pub fn mirror_offer(&self, item: &str) -> Option<MirrorOffer> {
        let opened = crate::open_with(&self.folder, self.ports.files.as_ref()).ok()?;
        crate::change::mirror_offer(&self.folder, self.ports.files.as_ref(), &opened, item)
    }

    /// What the card of the item of the project by this name offers: how
    /// many pieces it is, whether the repeat controls apply, and what "And
    /// the other side" does ([`crate::item_offer::item_offer`]), read from
    /// the project as the edit files give it. `None` when it has no design.
    pub fn item_offer(&self, item: &str) -> Option<crate::item_offer::ItemOffer> {
        let opened = crate::open_with(&self.folder, self.ports.files.as_ref()).ok()?;
        crate::item_offer::item_offer(&self.folder, self.ports.files.as_ref(), &opened, item)
    }

    /// The grips listed by the last [`EditCommand::ListGrips`], with their
    /// stops as far as found; `None` before a listing and after a commit.
    pub fn grips(&self) -> Option<&GripList> {
        self.grips.as_ref()
    }

    /// The search for the stops of the listed grips not searched yet, to
    /// run off the main thread ([`StopsJob::run`]); hand each answer to
    /// [`Session::grip_stops_found`]. Set its cancel flag when the
    /// selection changes. `None` with no grips listed.
    pub fn grip_stops_job(&self) -> Option<StopsJob> {
        self.grips.as_ref().map(GripList::stops_job)
    }

    /// Takes one grip's stops from a [`StopsJob`]: the grip's range
    /// becomes the two last good values. An answer for an older listing is
    /// dropped.
    pub fn grip_stops_found(&mut self, found: FoundStops) -> Vec<Event> {
        let index = found.index;
        if self.grips.as_mut().is_some_and(|grips| grips.found(found)) {
            vec![Event::Edit(EditEvent::GripStopsFound(index))]
        } else {
            Vec::new()
        }
    }

    /// Searches the stops of the listed grips here and now (of the grip at
    /// `only`, or of every one not searched yet), stopping when `cancel`
    /// is set: what a script does, where nothing else waits. A window runs
    /// [`Session::grip_stops_job`] off the main thread instead.
    pub fn find_grip_stops(&mut self, only: Option<usize>, cancel: &AtomicBool) -> Vec<Event> {
        let Some(job) = self.grip_stops_job() else {
            return vec![Event::Edit(EditEvent::Rejected(Rejection::NoGrips))];
        };
        let job = match only {
            Some(index) => job.only(index),
            None => job,
        };
        let mut found = Vec::new();
        let result = job.run(cancel, &mut |stops| found.push(stops));
        let mut events = Vec::new();
        for stops in found {
            events.extend(self.grip_stops_found(stops));
        }
        if let Err(message) = result {
            events.push(Event::Edit(EditEvent::Rejected(Rejection::CantStart(
                message,
            ))));
        }
        finish(events)
    }

    /// The distance between two things as the design on disk (or in the
    /// editor's buffers) has it now: the value Hold offers before the user
    /// types another ([`crate::hold::measured_distance`]).
    ///
    /// # Errors
    ///
    /// The project doesn't place, or the two can't be measured, in words.
    pub fn measure(
        &self,
        first: &crate::hold::Measured,
        second: &crate::hold::Measured,
    ) -> Result<stave_lang::units::Length, String> {
        let opened =
            crate::open_with(&self.folder, self.ports.files.as_ref()).map_err(|e| e.to_string())?;
        let design = opened
            .design
            .as_ref()
            .ok_or_else(|| crate::editing::StartError::NotSolved.to_string())?;
        crate::hold::measured_distance(design, first, second).map_err(|e| e.to_string())
    }

    /// The checks of the options of the chosen candidate's settings not
    /// checked yet, to run off the main thread
    /// ([`crate::option_fits::OptionsJob::run`]); hand each answer to
    /// [`Session::option_fit_found`]. Set its cancel flag when the
    /// candidate changes (any [`EditEvent::VerdictChanged`],
    /// [`EditEvent::SettingChanged`] and the like): its answers would be
    /// dropped. `None` with no candidate chosen.
    pub fn options_job(&self) -> Option<crate::option_fits::OptionsJob> {
        self.edit.as_ref().and_then(Edit::options_job)
    }

    /// Takes one option's fit from an options job; says
    /// [`EditEvent::OptionsChanged`] when it changed the card. An answer
    /// for the candidate as it was before a change is dropped.
    pub fn option_fit_found(&mut self, found: crate::option_fits::FoundFit) -> Vec<Event> {
        if self
            .edit
            .as_mut()
            .is_some_and(|edit| edit.option_found(found))
        {
            vec![Event::Edit(EditEvent::OptionsChanged)]
        } else {
            Vec::new()
        }
    }

    /// Checks the options of the chosen candidate's settings here and now
    /// (of the setting named `only`, or of every one), stopping when
    /// `cancel` is set: what a script does, where nothing else waits. A
    /// window runs [`Session::options_job`] off the main thread instead.
    pub fn check_setting_options(&mut self, only: Option<&str>, cancel: &AtomicBool) -> Vec<Event> {
        let Some(job) = self.options_job() else {
            return vec![Event::Edit(EditEvent::Rejected(Rejection::NothingChosen))];
        };
        let job = match only {
            Some(setting) => job.only(setting),
            None => job,
        };
        let mut found = Vec::new();
        let result = job.run(cancel, &mut |fit| found.push(fit));
        let mut events = Vec::new();
        for fit in found {
            events.extend(self.option_fit_found(fit));
        }
        if let Err(message) = result {
            events.push(Event::Edit(EditEvent::Rejected(Rejection::CantStart(
                message,
            ))));
        }
        finish(events)
    }

    /// Works out whether each entry of the edit's menu applies, as
    /// [`EditCommand::CheckMenu`] does, but stops when `cancel` is set
    /// (the entries reached keep their answer).
    pub fn check_edit_menu(&mut self, cancel: &AtomicBool) -> Vec<Event> {
        let mut events = Vec::new();
        if let Some(edit) = self.edit.as_mut()
            && let Ok(true) = edit.check_menu(cancel)
        {
            events.push(Event::Edit(EditEvent::MenuChanged));
        }
        finish(events)
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

    /// Whether the editor is open.
    pub fn editor(&self) -> Editor {
        self.place.editor
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
                        editor: self.place.editor,
                    };
                    self.go(place, &mut events);
                }
            }
            Command::Focus(target) => {
                if self.place.focus != Some(target) || self.place.view != View::Design {
                    let place = Place {
                        view: View::Design,
                        selection: Some(Selection::of_target(target)),
                        focus: Some(target),
                        editor: self.place.editor,
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
                    view: View::Design,
                    selection: Some(Selection::Location(location)),
                    focus: None,
                    editor: Editor::Open,
                };
                if place != self.place {
                    self.go(place, &mut events);
                }
            }
            Command::ShowEditor(editor) => {
                if editor != self.place.editor {
                    self.place.editor = editor;
                    events.push(Event::EditorChanged);
                }
            }
            Command::Link(selection) => {
                if selection != self.place.selection {
                    let had_focus = self.place.focus.is_some();
                    self.place.selection = selection;
                    self.place.focus = None;
                    events.push(Event::SelectionChanged);
                    if had_focus {
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
            Command::NextProblem => self.step_problem(Step::Next, &mut events),
            Command::PreviousProblem => self.step_problem(Step::Previous, &mut events),
            Command::Check => self.request_check(&mut events),
            Command::Build => {
                if self.build == Build::Idle && self.buildable().is_ok() {
                    self.build = Build::Building;
                    events.push(Event::BuildChanged);
                    self.request_check(&mut events);
                }
            }
            Command::Edit(command) => self.edit_command(command, &mut events),
        }
        finish(events)
    }

    /// Does an edit command: starting, adding and cancelling here, and the
    /// changes of the design; the rest in the edit itself.
    fn edit_command(&mut self, command: EditCommand, events: &mut Vec<Event>) {
        match command {
            EditCommand::Start(picked) => {
                self.drop_edit(events);
                match Edit::start(&self.folder, self.ports.files.clone(), picked) {
                    Ok(edit) => {
                        self.edit = Some(edit);
                        events.push(Event::Edit(EditEvent::Started));
                    }
                    Err(error) => events.push(Event::Edit(EditEvent::Rejected(
                        Rejection::CantStart(error.to_string()),
                    ))),
                }
            }
            EditCommand::Cancel => {
                if !self.drop_edit(events) {
                    events.push(Event::Edit(EditEvent::Rejected(Rejection::NoEdit)));
                }
            }
            EditCommand::Add => {
                let started = std::time::Instant::now();
                let result = if let Some(change) = self.change.as_mut() {
                    change.commit(self.ports.writer.as_mut())
                } else if let Some(edit) = self.edit.as_mut() {
                    edit.commit(self.ports.writer.as_mut())
                } else {
                    events.push(Event::Edit(EditEvent::Rejected(Rejection::NoEdit)));
                    return;
                };
                match result {
                    Ok(commit) => self.committed(commit, started, events),
                    Err(crate::editing::CommitError::NothingChosen) => {
                        events.push(Event::Edit(EditEvent::Rejected(Rejection::NothingChosen)));
                    }
                    Err(crate::editing::CommitError::DoesNotFit) => {
                        events.push(Event::Edit(EditEvent::Rejected(Rejection::DoesNotFit)));
                    }
                    Err(error @ crate::editing::CommitError::Write { .. }) => events.push(
                        Event::Edit(EditEvent::Rejected(Rejection::CantWrite(error.to_string()))),
                    ),
                }
            }
            EditCommand::RepeatItem { item, count, axis } => self.start_change(
                ChangeRequest::Repeat {
                    item,
                    count,
                    axis,
                    shares: None,
                },
                events,
            ),
            EditCommand::MoveShares {
                region,
                lower,
                boundary,
            } => self.start_change(
                ChangeRequest::MoveShares {
                    region,
                    lower,
                    boundary,
                },
                events,
            ),
            EditCommand::Mirror(item) => self.start_change(ChangeRequest::Mirror { item }, events),
            EditCommand::MirrorSplit { item, axis } => {
                self.start_change(ChangeRequest::MirrorSplit { item, axis }, events);
            }
            EditCommand::SetParam { item, param, value } => self.start_change(
                ChangeRequest::SetParam {
                    item,
                    param,
                    value: crate::candidates::print::expr_text(&value),
                },
                events,
            ),
            EditCommand::Hold(request) => self.start_change(ChangeRequest::Hold(request), events),
            EditCommand::RepeatPart {
                part,
                count,
                axis,
                from,
                to,
            } => self.start_change(
                ChangeRequest::RepeatPart {
                    part,
                    count,
                    axis,
                    from,
                    to,
                },
                events,
            ),
            EditCommand::MakeSolved { param, low, high } => self.start_change(
                ChangeRequest::ParamKind {
                    param,
                    kind: crate::param_kind::NewParamKind::Solved {
                        low: crate::candidates::print::expr_text(&low),
                        high: crate::candidates::print::expr_text(&high),
                    },
                },
                events,
            ),
            EditCommand::MakePlain(param) => self.start_change(
                ChangeRequest::ParamKind {
                    param,
                    kind: crate::param_kind::NewParamKind::Plain,
                },
                events,
            ),
            EditCommand::ListGrips(selection) => {
                let ticket = GripsTicket(self.next_grips);
                self.next_grips = self.next_grips.saturating_add(1);
                match GripList::list(&self.folder, self.ports.files.clone(), selection, ticket) {
                    Ok(grips) => {
                        self.grips = Some(grips);
                        events.push(Event::Edit(EditEvent::GripsChanged));
                    }
                    Err(message) => events.push(Event::Edit(EditEvent::Rejected(
                        Rejection::CantStart(message),
                    ))),
                }
            }
            EditCommand::BeginDrag(index) => {
                self.begin_drag(index, events);
            }
            EditCommand::DragTo(target) => match self.change.as_mut() {
                Some(change) if change.is_drag() => {
                    events.extend(change.grip_move(target).into_iter().map(Event::Edit));
                }
                Some(_) | None => {
                    events.push(Event::Edit(EditEvent::Rejected(Rejection::NotDragging)));
                }
            },
            EditCommand::PickParam(param) => match self.change.as_mut() {
                Some(change) if change.is_drag() => {
                    events.extend(change.grip_pick(param).into_iter().map(Event::Edit));
                }
                Some(_) | None => {
                    events.push(Event::Edit(EditEvent::Rejected(Rejection::NotDragging)));
                }
            },
            EditCommand::TypeGripValue { grip, value } => {
                if self.begin_drag(grip, events)
                    && let Some(change) = self.change.as_mut()
                {
                    events.extend(change.grip_move(value).into_iter().map(Event::Edit));
                }
            }
            EditCommand::Remove(item) => self.start_change(ChangeRequest::Remove { item }, events),
            EditCommand::RenameItem { item, to } => {
                self.start_change(ChangeRequest::Rename { item, to }, events);
            }
            EditCommand::IncludeDependants => match self.change.as_mut() {
                Some(change) => {
                    events.extend(change.include_dependants().into_iter().map(Event::Edit));
                }
                None => events.push(Event::Edit(EditEvent::Rejected(
                    Rejection::NothingToInclude,
                ))),
            },
            EditCommand::ApplyFix { problem, fix } if self.change.is_some() => {
                if let Some(change) = self.change.as_mut() {
                    events.extend(change.apply_fix(problem, fix).into_iter().map(Event::Edit));
                }
            }
            command @ (EditCommand::CheckMenu
            | EditCommand::Choose(_)
            | EditCommand::Bind(_)
            | EditCommand::Set { .. }
            | EditCommand::Rename(_)
            | EditCommand::PutBack
            | EditCommand::UseDefault(_)
            | EditCommand::Repeat(_)
            | EditCommand::ApplyFix { .. }) => match self.edit.as_mut() {
                Some(edit) => events.extend(edit.apply(command).into_iter().map(Event::Edit)),
                None => events.push(Event::Edit(EditEvent::Rejected(Rejection::NoEdit))),
            },
        }
    }

    /// Starts a drag on the listed grip at `index`, dropping what was in
    /// progress; `false`, with a rejection, when there is no such grip.
    fn begin_drag(&mut self, index: usize, events: &mut Vec<Event>) -> bool {
        let Some(grips) = self.grips.as_ref() else {
            events.push(Event::Edit(EditEvent::Rejected(Rejection::NoGrips)));
            return false;
        };
        let Some(listed) = grips.grips().get(index) else {
            events.push(Event::Edit(EditEvent::Rejected(Rejection::NoSuchGrip(
                index,
            ))));
            return false;
        };
        let change = Change::for_grip(
            &self.folder,
            self.ports.files.clone(),
            grips.shared_base(),
            index,
            listed.grip.clone(),
        );
        self.drop_edit(events);
        self.change = Some(change);
        events.push(Event::Edit(EditEvent::ChangeStarted));
        true
    }

    /// Drops the edit or change in progress, saying so; `false` when there
    /// was none.
    fn drop_edit(&mut self, events: &mut Vec<Event>) -> bool {
        let dropped = self.edit.take().is_some() | self.change.take().is_some();
        if dropped {
            events.push(Event::Edit(EditEvent::Cancelled));
        }
        dropped
    }

    /// Starts a change of the design, dropping what was in progress.
    fn start_change(&mut self, request: ChangeRequest, events: &mut Vec<Event>) {
        self.drop_edit(events);
        match Change::start(&self.folder, self.ports.files.clone(), request) {
            Ok(change) => {
                self.change = Some(change);
                events.push(Event::Edit(EditEvent::ChangeStarted));
                events.push(Event::Edit(EditEvent::VerdictChanged));
            }
            Err(error @ (ChangeError::Start(_) | ChangeError::Unwritable(_))) => events.push(
                Event::Edit(EditEvent::Rejected(Rejection::CantStart(error.to_string()))),
            ),
        }
    }

    /// Shows the design a commit made, as after a check.
    fn committed(&mut self, commit: Commit, started: std::time::Instant, events: &mut Vec<Event>) {
        self.edit = None;
        self.change = None;
        // The grips were listed from the design as it was.
        if self.grips.take().is_some() {
            events.push(Event::Edit(EditEvent::GripsChanged));
        }
        self.last_commit = commit.files;
        events.push(Event::Edit(EditEvent::Committed));
        let checked = crate::check::checked(&self.folder, commit.opened, started);
        let verdict = Verdict::Checked(checked.summary);
        self.adopt(Rc::new(checked), events);
        if self.running.is_none() {
            self.status = CheckStatus::Done(verdict);
            events.push(Event::CheckStatusChanged);
        }
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
        let cant_open_first = matches!(verdict, Verdict::CantOpen(_)) && self.latest().is_none();
        self.status = CheckStatus::Done(verdict);
        events.push(Event::CheckStatusChanged);
        // With no design in yet, what the view shows changes from
        // "Checking…" to why the folder can't be opened (an empty folder
        // included): said as a changed result so the window redraws it.
        if cant_open_first {
            events.push(Event::ResultChanged);
        }
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

    /// Selects the problem after (or before) the selected one, wrapping
    /// round; from anything else, the first (or the last). Leaves focus
    /// and shows the design.
    fn step_problem(&mut self, step: Step, events: &mut Vec<Event>) {
        let count = self.problems().len();
        let Some(last) = count.checked_sub(1) else {
            return;
        };
        let current = match self.place.selection {
            Some(Selection::Problem(ProblemIndex(index))) if index < count => Some(index),
            _ => None,
        };
        let index = match (step, current) {
            (Step::Next, None) => 0,
            (Step::Previous, None) => last,
            (Step::Next, Some(index)) if index >= last => 0,
            (Step::Next, Some(index)) => index.saturating_add(1),
            (Step::Previous, Some(0)) => last,
            (Step::Previous, Some(index)) => index.saturating_sub(1),
        };
        let place = Place {
            view: View::Design,
            selection: Some(Selection::Problem(ProblemIndex(index))),
            focus: None,
            editor: self.place.editor,
        };
        if place != self.place {
            self.go(place, events);
        }
    }

    /// Starts a check, or asks for another once the running one is done.
    fn request_check(&mut self, events: &mut Vec<Event>) {
        if self.running.is_some() {
            self.queued = Queued::Again;
            return;
        }
        let ticket = CheckTicket::next();
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
        let place = carry.place(&self.place);
        if place.selection != self.place.selection {
            events.push(Event::SelectionChanged);
        }
        if place.focus != self.place.focus {
            events.push(Event::FocusChanged);
        }
        self.place = place;
        for place in self.back.iter_mut().chain(self.forward.iter_mut()) {
            *place = carry.place(place);
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
        self.place_events(&current, events);
        push_limited(&mut self.back, current);
        self.forward.clear();
    }

    fn changed_place_events(&self, events: &mut Vec<Event>) {
        // Back and Forward may change anything; say so.
        events.extend([
            Event::ViewChanged,
            Event::SelectionChanged,
            Event::FocusChanged,
            Event::EditorChanged,
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
        if before.editor != self.place.editor {
            events.push(Event::EditorChanged);
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
            editor: place.editor,
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
            fn key(p: &Problem) -> (&str, &str, Option<&str>) {
                (&p.code, &p.message, p.location.as_deref())
            }
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
    fn the_pill_counts_errors_and_warnings_apart() {
        let words = |errors, warnings| {
            let pill = CheckStatus::Done(Verdict::Checked(Summary {
                errors,
                warnings,
                files: 1,
                elapsed: Duration::ZERO,
            }))
            .pill();
            (pill.tone, pill.label)
        };
        assert_eq!(words(0, 0), (Tone::Clean, "No problems".to_owned()));
        assert_eq!(words(1, 0), (Tone::Errors, "1 error".to_owned()));
        assert_eq!(words(0, 2), (Tone::Warnings, "2 warnings".to_owned()));
        assert_eq!(
            words(3, 1),
            (Tone::Errors, "3 errors, 1 warning".to_owned())
        );
    }

    #[test]
    fn a_file_and_line_from_the_command_line_names_a_place() {
        let folder = Path::new("/work/bench");
        let place = |text| SourceLocation::parse(folder, text);
        let line = |number| LineNumber::new(number).expect("not 0");
        assert_eq!(
            place("doors.stave:9"),
            Some(SourceLocation {
                path: "/work/bench/doors.stave".to_owned(),
                line: line(9),
            })
        );
        assert_eq!(
            place("std/hinges.stave"),
            Some(SourceLocation {
                path: "std/hinges.stave".to_owned(),
                line: line(1),
            })
        );
        assert_eq!(place("doors.stave:top").map(|p| p.line), Some(line(1)));
        assert_eq!(place("doors.stave:0"), None);
    }

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
        assert_eq!(View::from_word("parts"), Some(View::Design));
        assert_eq!(View::from_word("files"), Some(View::Design));
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
