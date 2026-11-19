//! The edit in progress (`docs/editing.md`, "The edit loop"): what the
//! user pointed at, the menu for it, the candidate chosen and filled in,
//! and its verdict, held between gestures. This joins the pieces the
//! editor is built from (signature matching in [`crate::candidates`], the
//! scratch check in [`crate::scratch`], the scene's picks) into one value
//! a window drives with commands, and that the command line drives with a
//! script.
//!
//! ```text
//! scene pick ──picked_from_*──▶ Picked ──Edit::start──▶ Edit { menu }
//!                                                        │ check_menu: each entry's applicability
//!   EditCommand::Choose / Bind / Set / Rename / ApplyFix │ each change re-runs the scratch check
//!                                                        ▼
//!                                              chosen candidate + Verdict
//!                                                        │ Edit::commit (only on Fits)
//!                                                        ▼
//!                                  text edits per file ──WriteEdits──▶ buffers or disk
//! ```
//!
//! - [`Edit::start`] prepares the project once ([`EditBase::shared`]) and
//!   lists the candidates for the selection. A project whose design does
//!   not solve can't be edited by pointing: the solver writes no design,
//!   so there is nothing to check a candidate against, and the start is
//!   refused with that said plainly ([`StartError::NotSolved`]).
//! - [`Edit::check_menu`] builds each entry in a scratch copy and records
//!   whether it applies; it can be stopped part way.
//! - [`Edit::apply`] takes the commands that change the candidate. Each
//!   change writes the candidate's patch again and re-runs the scratch
//!   check, so the verdict is always that of the candidate as it now is.
//!   A candidate with a place param still to point at or a setting still
//!   to choose is not tried: it *needs a choice*.
//! - [`Edit::commit`] writes the patch, only when it fits, as text edits
//!   per file through a [`WriteEdits`]: the open buffers in a window (so
//!   the editor and its undo see them), the disk on the command line. It
//!   gives the committed design back for the session to show.
//! - Cancelling is dropping the [`Edit`]: nothing was written, so nothing
//!   is left behind.
//!
//! The prepared base is never changed by trying: every candidate and
//! every change is tried from the design as it was when the edit began.

use crate::i18n::{fill_in, tr};
use std::cell::RefCell;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use stave_check::Cancelled;
use stave_lang::ast::{self, Expr, Ident, Name, Span};
use stave_lang::placement::Axis;
use stave_lang::units::Length;
use stave_lang::{Fix as LangFix, Parse, SourceMap, TextEdit, apply_patch, parse_patch_file};
use stave_model::ids::ParamId;
use stave_model::nonempty::NonEmpty;
use stave_model::trace::Amount;
use stave_modules::{FileSource, LoadError, MemoryFiles};

use crate::Opened;
use crate::candidates::applicability::{self, PATCH_NAME, still_to_choose};
use crate::candidates::{EditScope, Repeat, ScopeError, sort_menu};
use crate::edit::{
    Applicability, Blame, Candidate, Declaration, FacingPair, PartFace, Picked, PlaceSlot,
    SettingOption, SettingValue, Verdict,
};
use crate::grips::GripSelection;
use crate::hold::HoldRequest;
use crate::naming::RepeatCount;
use crate::naming::TakenNames;
use crate::option_fits::{FoundFit, OptionsJob, OptionsTicket};
use crate::scratch::{DiskWriter, EditBase, Tried, TryError};

// ---------------------------------------------------------------------------
// From the scene's picks to a typed selection
// ---------------------------------------------------------------------------

/// The selection a click on a part's face gives: its `Anchor` and its
/// `Part`.
pub fn picked_from_part_face(face: stave_scene::PartFace) -> Picked {
    Picked::Face(PartFace {
        part: face.part,
        face: face.face,
    })
}

/// The selection a click in a space gives: the region the scene picked
/// (the innermost one under the pointer, never one behind a part).
pub fn picked_from_region(hit: &stave_scene::RegionHit) -> Picked {
    Picked::Region(hit.region)
}

/// The selection what is under the pointer gives: a face of a part, or
/// the space of a region. `None` for a piece of hardware or a joint's
/// member, which are not places to put something.
pub fn picked_from_target(target: &stave_scene::Target) -> Option<Picked> {
    match target {
        stave_scene::Target::Node(hit) => hit.part_face().map(picked_from_part_face),
        stave_scene::Target::Region(hit) => Some(picked_from_region(hit)),
    }
}

/// The selection a click in the gap between two facing parts gives:
/// `Parts`, the two, the one on the low side (left, bottom, front) first.
/// `None` only if the scene paired a part with itself.
pub fn picked_from_facing(facing: &stave_scene::Facing) -> Option<Picked> {
    FacingPair::new(facing.first.part, facing.second.part)
        .ok()
        .map(Picked::Between)
}

// ---------------------------------------------------------------------------
// Commands and events
// ---------------------------------------------------------------------------

/// What the window (or a script) asks of the edit in progress.
#[derive(Debug, Clone, PartialEq)]
pub enum EditCommand {
    /// Start an edit from a selection: list its menu. An edit already in
    /// progress is dropped first.
    Start(Picked),
    /// Work out whether each menu entry applies (builds each in a scratch
    /// copy), to the end.
    CheckMenu,
    /// Choose the menu entry at this index of [`Edit::menu`].
    Choose(usize),
    /// Bind the next place param still to point at with a further pick.
    Bind(Picked),
    /// Set a setting param to a value typed in the card.
    Set {
        /// The setting.
        name: Ident,
        /// Its value.
        value: Expr,
    },
    /// Put the setting a refusal blames back to the value it had before
    /// the change that made the candidate stop fitting ([`Chosen::blame`]).
    PutBack,
    /// Put a setting back to its param's own default, so nothing is
    /// written for it (the short list's "default" entry).
    UseDefault(Ident),
    /// Name the new item, in place of the generated name.
    Rename(Ident),
    /// Apply one of the refusal's fixes: the fix at `fix` of the problem at
    /// `problem`, both counted from 0 in the order the card shows them.
    ApplyFix {
        /// The problem.
        problem: usize,
        /// The fix.
        fix: usize,
    },
    /// Make the chosen candidate an array of pieces filling its region,
    /// split along an axis (`Some`), or a single one again (`None`). The
    /// array is named by the naming rules (`drawer[bottom, top]` filling
    /// `drawer_bay[bottom, top]`).
    Repeat(Option<Repeat>),
    /// Start a change: repeat an assembly of the project that fills a
    /// region `count` times, or change the count of one already repeated
    /// (`axis` is needed for a first repeat and ignored after).
    RepeatItem {
        /// The assembly.
        item: Ident,
        /// How many pieces.
        count: RepeatCount,
        /// The axis to split its region along.
        axis: Option<Axis>,
    },
    /// Start a change: move the boundary between pieces `lower` and
    /// `lower + 1` (from 0, in label order) of a split region, so the
    /// lower piece is `boundary` long along the split; their sum is kept.
    MoveShares {
        /// The region array: `drawer_bay`.
        region: Ident,
        /// The first of the two pieces.
        lower: usize,
        /// The new size of that piece.
        boundary: Length,
    },
    /// Start a change: add the mirror image of an assembly (renaming it
    /// with its side). [`crate::session::Session::mirror_offer`] says
    /// whether it has one, without trying it.
    Mirror(Ident),
    /// Start a change: make an assembly that fills a whole space a
    /// mirrored pair, splitting the space in two along `axis`
    /// ([`crate::repeat::mirror_split_assembly`]): "repeat across as a
    /// mirrored pair".
    MirrorSplit {
        /// The assembly.
        item: Ident,
        /// The axis the space is split along.
        axis: Axis,
    },
    /// Start a change: set a param of an assembly of the project, as a
    /// grip does when it is let go (`set apron_front.setback = 8mm`).
    SetParam {
        /// The assembly.
        item: Ident,
        /// Its component's param.
        param: Ident,
        /// The value.
        value: Expr,
    },
    /// Start a change: hold a measured distance as a `require`.
    Hold(HoldRequest),
    /// Start a change: remove an item. While something depends on it the
    /// change needs a choice, naming what does; then
    /// [`EditCommand::IncludeDependants`] removes those too.
    Remove(Ident),
    /// Remove the dependants too (a removal waiting on them).
    IncludeDependants,
    /// Start a change: rename an item of the project everywhere it is
    /// used.
    RenameItem {
        /// The item.
        item: Ident,
        /// Its new name.
        to: Ident,
    },
    /// Start a change: repeat a part spaced evenly along a span, from one
    /// anchor to another, or change how many there are
    /// ([`crate::repeat::repeat_part`]).
    RepeatPart {
        /// The part: `slat`.
        part: Ident,
        /// How many it becomes.
        count: RepeatCount,
        /// The axis they are spaced along (it names the labels of two or
        /// three).
        axis: Axis,
        /// The anchor the span starts at, as written: `apron_left.right`.
        from: String,
        /// The anchor it ends at: `apron_right.left`.
        to: String,
    },
    /// List the grips of a selection: an assembly, the envelope, or the
    /// boundaries of a split ([`crate::session::Session::grips`]). Their
    /// stops are not searched yet: run
    /// [`crate::session::Session::grip_stops_job`] off the main thread.
    ListGrips(GripSelection),
    /// Start a drag on the grip at this index of the listed grips (a
    /// change; what was in progress is dropped, the grips stay listed).
    BeginDrag(usize),
    /// Move the grip being dragged to a value: its patch is written again
    /// and tried, as an edit in progress is. `Add` ends the drag by
    /// writing it, `Cancel` by dropping it.
    DragTo(Amount),
    /// Pick which plain param a follows grip changes, from the drag's
    /// choices ([`crate::gripping::GripDetail::choices`]).
    PickParam(ParamId),
    /// Type a value for the grip at this index: a drag begun and moved
    /// there in one, ready for `Add`.
    TypeGripValue {
        /// The grip's index in the listed grips.
        grip: usize,
        /// The value.
        value: Amount,
    },
    /// Start a change: make a param of the project a `solved param` the
    /// solver may change within `low..high`.
    MakeSolved {
        /// The param.
        param: Ident,
        /// The lowest value it may take.
        low: Expr,
        /// The highest.
        high: Expr,
    },
    /// Start a change: make a solved param of the project plain again,
    /// keeping its range.
    MakePlain(Ident),
    /// Write the candidate, or the change, into the text (only when it
    /// fits).
    Add,
    /// Drop the edit or the change, writing nothing.
    Cancel,
}

/// What changed in the edit in progress, for the card and the ghost to
/// redraw.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EditEvent {
    /// An edit began: [`Edit::menu`] lists what can go where the user
    /// pointed.
    Started,
    /// Menu entries' applicability changed, or their order.
    MenuChanged,
    /// A candidate was chosen.
    Chosen,
    /// A place param was bound by pointing.
    PlaceBound,
    /// A setting's value changed.
    SettingChanged,
    /// Options of the chosen candidate's settings were found to fit or
    /// not ([`crate::edit::SettingOption::fit`]), or their order changed.
    OptionsChanged,
    /// The new item's name changed.
    Renamed,
    /// A fix was added to the candidate.
    FixApplied,
    /// The chosen candidate's repeat changed.
    RepeatChanged,
    /// A change of the design began ([`crate::session::Session::change`]):
    /// it has been tried, and [`EditEvent::VerdictChanged`] follows.
    ChangeStarted,
    /// A removal now takes the items that depend on it too.
    DependantsIncluded,
    /// The chosen candidate's [`Progress`] changed: its verdict, or what is
    /// still to choose.
    VerdictChanged,
    /// [`crate::session::Session::grips`] changed: listed, or dropped
    /// (after a commit, the design they were listed from is gone).
    GripsChanged,
    /// The stops of the grip at this index were found.
    GripStopsFound(usize),
    /// The grip being dragged moved: the change's detail
    /// ([`crate::change::ChangeDetail::Grip`]) says to what, and what
    /// changes.
    DragMoved,
    /// A param was picked for a follows grip.
    ParamPicked,
    /// The candidate was written into the text; the edit is over and the
    /// committed design is the result.
    Committed,
    /// The edit was dropped; nothing was written.
    Cancelled,
    /// A command could not be done; nothing changed.
    Rejected(Rejection),
}

/// Why a command was not done. Each says it in words with
/// [`Rejection::text`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Rejection {
    /// There is no edit in progress.
    NoEdit,
    /// The command needs a chosen candidate, and none is chosen.
    NothingChosen,
    /// No menu entry has this index.
    NoSuchEntry(usize),
    /// The pick fills none of the place params still to point at.
    PickFillsNothing,
    /// The candidate has no setting by this name.
    NoSuchSetting(String),
    /// The name is already used in the project, is a word of the language,
    /// or is not a name.
    NameUnavailable(String),
    /// The verdict has no such fix.
    NoSuchFix,
    /// [`EditCommand::PutBack`] with no refusal that blames a setting.
    NothingToPutBack,
    /// [`EditCommand::UseDefault`] on a setting whose param has no default.
    NoDefault(String),
    /// Add was asked for a candidate that doesn't fit.
    DoesNotFit,
    /// The edit couldn't start (see [`StartError`]), in words.
    CantStart(String),
    /// The text couldn't be written, in words.
    CantWrite(String),
    /// [`EditCommand::IncludeDependants`] without a removal waiting on
    /// its dependants.
    NothingToInclude,
    /// A grip command with no grips listed.
    NoGrips,
    /// No listed grip has this index.
    NoSuchGrip(usize),
    /// A drag command with no drag in progress.
    NotDragging,
    /// The param picked is not one the grip could change.
    NotAChoice,
}

impl Rejection {
    /// The words for it.
    pub fn text(&self) -> String {
        match self {
            Rejection::NoEdit => tr("nothing is being edited"),
            Rejection::NothingChosen => tr("choose something from the menu first"),
            Rejection::NoSuchEntry(index) => fill_in(
                &tr("the menu has no entry {number}"),
                &[("number", &(index + 1))],
            ),
            Rejection::PickFillsNothing => tr("that isn't what the card is asking you to point at"),
            Rejection::NoSuchSetting(name) => fill_in(
                &tr("there is no setting called '{name}'"),
                &[("name", name)],
            ),
            Rejection::NameUnavailable(name) => fill_in(
                &tr(
                    "'{name}' can't be used: it is already used in this project, is a word of the language, or is not a name",
                ),
                &[("name", name)],
            ),
            Rejection::NoSuchFix => tr("there is no such fix"),
            Rejection::NothingToPutBack => {
                tr("nothing to put back: no setting change made it stop fitting")
            }
            Rejection::NoDefault(name) => fill_in(
                &tr("'{name}' has no default: choose a value"),
                &[("name", name)],
            ),
            Rejection::DoesNotFit => tr("the edit doesn't fit, so nothing was written"),
            Rejection::CantStart(message) | Rejection::CantWrite(message) => message.clone(),
            Rejection::NothingToInclude => tr("there is no removal waiting on what depends on it"),
            Rejection::NoGrips => tr("list the grips of something first"),
            Rejection::NoSuchGrip(index) => fill_in(
                &tr("there is no grip {number}"),
                &[("number", &(index + 1))],
            ),
            Rejection::NotDragging => tr("no grip is being dragged"),
            Rejection::NotAChoice => tr("that is not one of the sizes this grip could change"),
        }
    }
}

// ---------------------------------------------------------------------------
// Starting
// ---------------------------------------------------------------------------

/// Why an edit can't start.
#[derive(Debug, thiserror::Error)]
pub enum StartError {
    /// The project folder can't be loaded.
    #[error(transparent)]
    Load(#[from] LoadError),
    /// Names and types didn't resolve far enough to have a model.
    #[error(transparent)]
    Scope(#[from] ScopeError),
    /// The design doesn't solve, so there is nothing to check an edit
    /// against.
    #[error("{}", self.text())]
    NotSolved,
}

impl StartError {
    /// The error in the window's words (translated).
    fn text(&self) -> String {
        match self {
            StartError::Load(error) => error.to_string(),
            StartError::Scope(error) => error.to_string(),
            StartError::NotSolved => tr(
                "the design can't be edited by pointing until it places: fix the errors `stave check` shows first",
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// The edit in progress
// ---------------------------------------------------------------------------

/// Where the chosen candidate stands.
#[derive(Debug, Clone, PartialEq)]
pub enum Progress {
    /// Something is still to choose before it can be tried: these place
    /// params (to point at) and settings (to choose in the card), place
    /// params first.
    NeedsChoice(NonEmpty<Name>),
    /// Its patch can't be written (the reason in words).
    Unwritable(String),
    /// It was tried: *fits* or *refused*.
    Tried(Verdict),
}

/// The candidate chosen from the menu, as filled in so far.
#[derive(Debug)]
pub struct Chosen {
    /// The candidate, with what has been pointed at and chosen.
    candidate: Candidate,
    /// The name typed for the new item, if any.
    name: Option<Ident>,
    /// The fixes added, in order.
    fixes: Vec<LangFix>,
    /// The repeat asked for, if any.
    repeat: Option<Repeat>,
    /// The patch as last written: the candidate's, then each fix.
    patch: Option<String>,
    /// The name the new item takes in that patch.
    item_name: Option<Ident>,
    /// Where it stands.
    progress: Progress,
    /// The last try, kept for the ghost and the commit.
    tried: Option<Tried>,
    /// The candidate as it stands now, for its options' checks.
    ticket: OptionsTicket,
    /// The setting change that made it stop fitting, while it is refused
    /// because of it.
    blame: Option<Blame>,
}

impl Chosen {
    /// The candidate, as filled in so far.
    pub fn candidate(&self) -> &Candidate {
        &self.candidate
    }

    /// Where it stands.
    pub fn progress(&self) -> &Progress {
        &self.progress
    }

    /// The patch as last written (the candidate's, then each fix), when
    /// it could be written.
    pub fn patch(&self) -> Option<&str> {
        self.patch.as_deref()
    }

    /// The name the new item takes: typed, or generated.
    pub fn item_name(&self) -> Option<&Ident> {
        self.item_name.as_ref()
    }

    /// The fixes added so far.
    pub fn fixes(&self) -> &[LangFix] {
        &self.fixes
    }

    /// The repeat asked for: how many pieces, split along which axis.
    pub fn repeat(&self) -> Option<Repeat> {
        self.repeat
    }

    /// The scratch copy as last checked, for the ghost; `None` when the
    /// candidate wasn't tried or its patch couldn't be applied.
    pub fn ghost(&self) -> Option<&Opened> {
        self.tried.as_ref().and_then(Tried::opened)
    }

    /// The last try, with its sources for rendering its diagnostics.
    pub fn tried(&self) -> Option<&Tried> {
        self.tried.as_ref()
    }

    /// The change to one setting that turned the candidate from fitting
    /// (or still needing that choice) into refused: which setting, from
    /// what, to what. `None` when it fits, when it was refused before
    /// the change, and after any other change but a rename, a fix or a
    /// repeat that leaves it refused.
    pub fn blame(&self) -> Option<&Blame> {
        self.blame.as_ref()
    }

    /// The candidate as it stands now: a [`FoundFit`] for another ticket
    /// is dropped.
    pub fn options_ticket(&self) -> OptionsTicket {
        self.ticket
    }
}

/// One edit in progress: the base it is tried on, the selection, its
/// menu and the candidate chosen.
pub struct Edit {
    folder: PathBuf,
    files: Arc<dyn FileSource + Send + Sync>,
    base: EditBase<'static>,
    picked: Picked,
    menu: Vec<Candidate>,
    chosen: Option<Chosen>,
}

impl fmt::Debug for Edit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Edit")
            .field("folder", &self.folder)
            .field("picked", &self.picked)
            .field("menu", &self.menu.len())
            .field("chosen", &self.chosen)
            .finish_non_exhaustive()
    }
}

/// What a commit wrote and the design it made.
#[derive(Debug)]
pub struct Commit {
    /// The text edits written, per file, in path order.
    pub files: Vec<FileEdit>,
    /// The committed design, as checked.
    pub opened: Opened,
}

/// The text edits that turn one file's old text into its new text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEdit {
    /// The file.
    pub path: PathBuf,
    /// `true` when the patch creates it.
    pub created: bool,
    /// The text the edits were computed from (empty for a new file).
    pub before: String,
    /// The edits, last in the file first ([`stave_lang::apply_text_edits`]).
    pub edits: Vec<TextEdit>,
    /// The new text.
    pub after: String,
}

/// Why a commit wrote nothing, or not everything.
#[derive(Debug, thiserror::Error)]
pub enum CommitError {
    /// No candidate is chosen.
    #[error("{}", self.text())]
    NothingChosen,
    /// The candidate doesn't fit.
    #[error("{}", self.text())]
    DoesNotFit,
    /// A file couldn't be written (the files before it were).
    #[error("{}", self.text())]
    Write {
        /// The file.
        path: PathBuf,
        /// What went wrong.
        source: io::Error,
    },
}

impl CommitError {
    /// The error in the window's words (translated).
    fn text(&self) -> String {
        match self {
            CommitError::NothingChosen => tr("choose something from the menu first"),
            CommitError::DoesNotFit => tr("the edit doesn't fit, so nothing was written"),
            CommitError::Write { path, source } => fill_in(
                &tr("can't write '{path}': {source}"),
                &[("path", &path.display()), ("source", source)],
            ),
        }
    }
}

impl Edit {
    /// Starts an edit of the project in `folder`, read through `files`,
    /// from a selection: prepares the project and lists the menu. Every
    /// entry is [`Applicability::NotChecked`] until [`Edit::check_menu`].
    ///
    /// # Errors
    ///
    /// The folder can't be loaded, its names and types don't resolve, or
    /// its design doesn't solve ([`StartError`]).
    pub fn start(
        folder: &Path,
        files: Arc<dyn FileSource + Send + Sync>,
        picked: Picked,
    ) -> Result<Edit, StartError> {
        let base = EditBase::shared(folder, files.clone())?;
        if base.opened().design.is_none() {
            return Err(StartError::NotSolved);
        }
        let menu =
            EditScope::new(base.opened())?.described_candidates(&base.opened().sources, &picked);
        Ok(Edit {
            folder: folder.to_path_buf(),
            files,
            base,
            picked,
            menu,
            chosen: None,
        })
    }

    /// The project folder.
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// The selection the edit began from.
    pub fn picked(&self) -> &Picked {
        &self.picked
    }

    /// The menu, in menu order.
    pub fn menu(&self) -> &[Candidate] {
        &self.menu
    }

    /// Where the refused entries of the menu begin, for the window to
    /// fold them: `menu()[refused_from()..]` are greyed, the entries before
    /// apply, need a choice or are not checked yet
    /// ([`crate::candidates::refused_from`]).
    pub fn refused_from(&self) -> usize {
        crate::candidates::refused_from(&self.menu)
    }

    /// The chosen candidate, once one is chosen.
    pub fn chosen(&self) -> Option<&Chosen> {
        self.chosen.as_ref()
    }

    /// The design the edit is tried on, as it was when the edit began.
    pub fn base(&self) -> &Opened {
        self.base.opened()
    }

    /// The editor's view of the base design.
    ///
    /// # Errors
    ///
    /// Never for an edit that started; [`ScopeError`] is kept for the type.
    pub fn scope(&self) -> Result<EditScope<'_>, ScopeError> {
        EditScope::new(self.base.opened())
    }

    /// Works out whether each menu entry not yet checked applies, building
    /// each in a scratch copy of the base (never changing the base). Once
    /// every entry is checked the menu is sorted again, greyed entries
    /// after the rest in each group. Returns `true` when an entry changed.
    ///
    /// # Errors
    ///
    /// [`Cancelled`] when `cancel` is set: the entries reached keep their
    /// answer, the rest stay not checked, and the order is unchanged.
    pub fn check_menu(&mut self, cancel: &AtomicBool) -> Result<bool, Cancelled> {
        let before: Vec<Applicability> = self
            .menu
            .iter()
            .map(|candidate| candidate.applicability.clone())
            .collect();
        let scope = EditScope::new(self.base.opened()).map_err(|_| Cancelled)?;
        let result = applicability::check_menu(&self.base, &scope, &mut self.menu, cancel);
        drop(scope);
        let changed = self
            .menu
            .iter()
            .zip(&before)
            .any(|(candidate, before)| candidate.applicability != *before);
        result?;
        sort_menu(&mut self.menu);
        Ok(changed)
    }

    /// Does a command that changes the candidate ([`EditCommand::Choose`],
    /// [`EditCommand::Bind`], [`EditCommand::Set`],
    /// [`EditCommand::Rename`], [`EditCommand::ApplyFix`],
    /// [`EditCommand::CheckMenu`]) and says what changed. Starting, adding
    /// and cancelling end or replace the edit, so they belong to its owner
    /// ([`crate::session::Session`], or [`Edit::start`] and
    /// [`Edit::commit`]); here they are rejected.
    pub fn apply(&mut self, command: EditCommand) -> Vec<EditEvent> {
        let mut events = Vec::new();
        match command {
            EditCommand::CheckMenu => {
                let never = AtomicBool::new(false);
                if let Ok(true) = self.check_menu(&never) {
                    events.push(EditEvent::MenuChanged);
                }
            }
            EditCommand::Choose(index) => match self.menu.get(index) {
                Some(candidate) => {
                    self.chosen = Some(Chosen {
                        candidate: candidate.clone(),
                        name: None,
                        fixes: Vec::new(),
                        repeat: None,
                        patch: None,
                        item_name: None,
                        progress: Progress::Unwritable(String::new()),
                        tried: None,
                        ticket: OptionsTicket::next(),
                        blame: None,
                    });
                    events.push(EditEvent::Chosen);
                    self.retry(&mut events);
                }
                None => events.push(EditEvent::Rejected(Rejection::NoSuchEntry(index))),
            },
            EditCommand::Bind(picked) => {
                let Some(chosen) = self.chosen.as_mut() else {
                    events.push(EditEvent::Rejected(Rejection::NothingChosen));
                    return events;
                };
                if bind_further(&mut chosen.candidate.place, &picked) {
                    events.push(EditEvent::PlaceBound);
                    self.retry(&mut events);
                } else {
                    events.push(EditEvent::Rejected(Rejection::PickFillsNothing));
                }
            }
            EditCommand::Set { name, value } => {
                let Some(chosen) = self.chosen.as_mut() else {
                    events.push(EditEvent::Rejected(Rejection::NothingChosen));
                    return events;
                };
                let setting = chosen
                    .candidate
                    .settings
                    .iter_mut()
                    .find(|setting| setting.name.as_str() == name.as_str());
                match setting {
                    Some(setting) => {
                        let from = std::mem::replace(
                            &mut setting.value,
                            SettingValue::Chosen(value.clone()),
                        );
                        let change = SettingChange {
                            setting: setting.name.clone(),
                            from,
                            from_text: setting.text.clone(),
                            to_text: crate::candidates::print::expr_text(&value),
                            options: setting.options.clone(),
                        };
                        events.push(EditEvent::SettingChanged);
                        self.retry_after_set(change, &mut events);
                    }
                    None => events.push(EditEvent::Rejected(Rejection::NoSuchSetting(
                        name.as_str().to_owned(),
                    ))),
                }
            }
            EditCommand::UseDefault(name) => {
                let Some(chosen) = self.chosen.as_mut() else {
                    events.push(EditEvent::Rejected(Rejection::NothingChosen));
                    return events;
                };
                let Some(setting) = chosen
                    .candidate
                    .settings
                    .iter_mut()
                    .find(|setting| setting.name.as_str() == name.as_str())
                else {
                    events.push(EditEvent::Rejected(Rejection::NoSuchSetting(
                        name.as_str().to_owned(),
                    )));
                    return events;
                };
                let Some(default) = setting.default_text.clone() else {
                    events.push(EditEvent::Rejected(Rejection::NoDefault(
                        name.as_str().to_owned(),
                    )));
                    return events;
                };
                if setting.value == SettingValue::Default {
                    return events;
                }
                let from = std::mem::replace(&mut setting.value, SettingValue::Default);
                let change = SettingChange {
                    setting: setting.name.clone(),
                    from,
                    from_text: setting.text.clone(),
                    to_text: default,
                    options: setting.options.clone(),
                };
                events.push(EditEvent::SettingChanged);
                self.retry_after_set(change, &mut events);
            }
            EditCommand::PutBack => {
                let Some(chosen) = self.chosen.as_mut() else {
                    events.push(EditEvent::Rejected(Rejection::NothingChosen));
                    return events;
                };
                let Some(blame) = chosen.blame.take() else {
                    events.push(EditEvent::Rejected(Rejection::NothingToPutBack));
                    return events;
                };
                if let Some(setting) = chosen
                    .candidate
                    .settings
                    .iter_mut()
                    .find(|setting| setting.name.as_str() == blame.setting.as_str())
                {
                    setting.value = blame.from;
                }
                events.push(EditEvent::SettingChanged);
                self.retry(&mut events);
            }
            EditCommand::Rename(name) => {
                let Some(chosen) = self.chosen.as_mut() else {
                    events.push(EditEvent::Rejected(Rejection::NothingChosen));
                    return events;
                };
                let taken = self
                    .base
                    .opened()
                    .model
                    .as_ref()
                    .map(TakenNames::of_project)
                    .unwrap_or_default();
                if chosen.item_name.as_ref() == Some(&name) {
                    return events;
                }
                if taken.is_unavailable(name.as_str()) {
                    events.push(EditEvent::Rejected(Rejection::NameUnavailable(
                        name.as_str().to_owned(),
                    )));
                    return events;
                }
                if let Some(old) = &chosen.item_name {
                    chosen.fixes = chosen
                        .fixes
                        .iter()
                        .map(|fix| renamed_fix(fix, old.as_str(), name.as_str()))
                        .collect();
                }
                chosen.name = Some(name);
                events.push(EditEvent::Renamed);
                self.retry(&mut events);
            }
            EditCommand::ApplyFix { problem, fix } => {
                let Some(chosen) = self.chosen.as_mut() else {
                    events.push(EditEvent::Rejected(Rejection::NothingChosen));
                    return events;
                };
                let found = match &chosen.progress {
                    Progress::Tried(Verdict::Refused(refusal)) => refusal
                        .problems()
                        .get(problem)
                        .and_then(|problem| problem.diagnostic().fixes.get(fix))
                        .cloned(),
                    Progress::Tried(Verdict::Fits(_) | Verdict::Checking)
                    | Progress::NeedsChoice(_)
                    | Progress::Unwritable(_) => None,
                };
                match found {
                    Some(found) => {
                        chosen.fixes.push(found);
                        events.push(EditEvent::FixApplied);
                        self.retry(&mut events);
                    }
                    None => events.push(EditEvent::Rejected(Rejection::NoSuchFix)),
                }
            }
            EditCommand::Repeat(repeat) => {
                let Some(chosen) = self.chosen.as_mut() else {
                    events.push(EditEvent::Rejected(Rejection::NothingChosen));
                    return events;
                };
                if chosen.repeat != repeat {
                    chosen.repeat = repeat;
                    events.push(EditEvent::RepeatChanged);
                    self.retry(&mut events);
                }
            }
            EditCommand::Start(_)
            | EditCommand::Add
            | EditCommand::Cancel
            | EditCommand::RepeatItem { .. }
            | EditCommand::MoveShares { .. }
            | EditCommand::Mirror(_)
            | EditCommand::MirrorSplit { .. }
            | EditCommand::SetParam { .. }
            | EditCommand::Hold(_)
            | EditCommand::Remove(_)
            | EditCommand::IncludeDependants
            | EditCommand::RenameItem { .. }
            | EditCommand::RepeatPart { .. }
            | EditCommand::ListGrips(_)
            | EditCommand::BeginDrag(_)
            | EditCommand::DragTo(_)
            | EditCommand::PickParam(_)
            | EditCommand::TypeGripValue { .. }
            | EditCommand::MakeSolved { .. }
            | EditCommand::MakePlain(_) => {
                events.push(EditEvent::Rejected(Rejection::NoEdit));
            }
        }
        events
    }

    /// Writes the chosen candidate into the text through `to`, when it
    /// fits, and gives back what was written and the committed design. On
    /// an error nothing about the edit changes, so it can be tried again.
    ///
    /// # Errors
    ///
    /// No candidate chosen, a candidate that doesn't fit, or a file that
    /// can't be written ([`CommitError`]).
    pub fn commit(&mut self, to: &mut dyn WriteEdits) -> Result<Commit, CommitError> {
        let chosen = self.chosen.as_mut().ok_or(CommitError::NothingChosen)?;
        let fits = matches!(chosen.progress, Progress::Tried(Verdict::Fits(_)));
        let (Some(tried), Some(patch), true) = (&chosen.tried, &chosen.patch, fits) else {
            return Err(CommitError::DoesNotFit);
        };
        if !tried.fits() {
            return Err(CommitError::DoesNotFit);
        }
        let parse = SourceMap::new()
            .add(PATCH_NAME, patch.as_str())
            .map(|file| parse_patch_file(file, patch))
            .ok();
        let files = file_edits(self.base.opened(), tried, parse.as_ref());
        write_all(&files, to)?;
        let opened = chosen
            .tried
            .take()
            .and_then(Tried::into_opened)
            .ok_or(CommitError::DoesNotFit)?;
        self.chosen = None;
        Ok(Commit { files, opened })
    }

    /// [`Edit::retry`] after a setting was set: the options of that
    /// setting keep the answers they had (they were tried with the other
    /// settings, which did not change), and the blame is worked out
    /// again.
    fn retry_after_set(&mut self, change: SettingChange, events: &mut Vec<EditEvent>) {
        let was_refused = self
            .chosen
            .as_ref()
            .is_some_and(|chosen| is_refused(&chosen.progress));
        let blamed_before = self
            .chosen
            .as_ref()
            .and_then(|chosen| chosen.blame.clone())
            .filter(|blame| blame.setting.as_str() == change.setting.as_str());
        self.retry_keeping(Some((&change.setting, &change.options)), events);
        let Some(chosen) = self.chosen.as_mut() else {
            return;
        };
        chosen.blame = if !is_refused(&chosen.progress) {
            None
        } else if let Some(blame) = blamed_before {
            // The same setting changed again while it is to blame: it still
            // stopped fitting when that setting left its first value.
            Some(Blame {
                to_text: change.to_text,
                ..blame
            })
        } else if was_refused {
            None
        } else {
            Some(Blame {
                setting: change.setting,
                from: change.from,
                from_text: change.from_text,
                to_text: change.to_text,
            })
        };
    }

    /// Writes the chosen candidate's patch again and re-runs the scratch
    /// check, recording where it stands.
    fn retry(&mut self, events: &mut Vec<EditEvent>) {
        self.retry_keeping(None, events);
    }

    /// [`Edit::retry`], keeping the options' answers of the setting named
    /// in `kept` (the setting's options as they were before).
    fn retry_keeping(
        &mut self,
        kept: Option<(&Ident, &[SettingOption])>,
        events: &mut Vec<EditEvent>,
    ) {
        let Some(chosen) = self.chosen.as_mut() else {
            return;
        };
        chosen.ticket = OptionsTicket::next();
        let before = chosen.progress.clone();
        chosen.tried = None;
        if let Ok(scope) = EditScope::new(self.base.opened()) {
            scope.describe(&self.base.opened().sources, &mut chosen.candidate);
        }
        let (progress, patch, item_name, tried) = attempt(&self.base, chosen);
        chosen.progress = progress;
        chosen.patch = patch;
        chosen.item_name = item_name;
        chosen.tried = tried;
        crate::candidates::describe::fill_default_values(
            &mut chosen.candidate,
            chosen.item_name.as_ref(),
            chosen.tried.as_ref().and_then(Tried::opened),
        );
        crate::option_fits::prepare(&mut chosen.candidate, &chosen.progress, kept);
        if !is_refused(&chosen.progress) {
            chosen.blame = None;
        }
        if chosen.progress != before {
            events.push(EditEvent::VerdictChanged);
        }
    }

    /// The checks of the chosen candidate's options not checked yet, to
    /// run off the main thread ([`OptionsJob::run`]); hand each answer to
    /// [`Edit::option_found`]. `None` with nothing chosen.
    pub fn options_job(&self) -> Option<OptionsJob> {
        let chosen = self.chosen.as_ref()?;
        Some(OptionsJob::new(
            &self.folder,
            self.files.clone(),
            chosen.ticket,
            &chosen.candidate,
            chosen.repeat,
            chosen.name.as_ref(),
            &chosen.fixes,
        ))
    }

    /// Takes one option's fit from an [`OptionsJob`]; `false` when it is
    /// for the candidate as it was (another ticket), for an option it
    /// doesn't have, or changes nothing.
    pub fn option_found(&mut self, found: FoundFit) -> bool {
        match self.chosen.as_mut() {
            Some(chosen) if chosen.ticket == found.ticket => {
                crate::option_fits::record(&mut chosen.candidate, found)
            }
            Some(_) | None => false,
        }
    }
}

/// A setting just set, as it was before.
struct SettingChange {
    setting: Ident,
    from: SettingValue,
    from_text: Option<String>,
    to_text: String,
    options: Vec<SettingOption>,
}

/// `true` when the candidate was tried and refused.
fn is_refused(progress: &Progress) -> bool {
    matches!(progress, Progress::Tried(Verdict::Refused(_)))
}

/// Writes the patch for `chosen` and tries it on `base`: where it stands,
/// the patch text, the new item's name and the try.
fn attempt(base: &EditBase<'_>, chosen: &Chosen) -> Attempt {
    let never = AtomicBool::new(false);
    let attempted = try_candidate(
        base,
        &chosen.candidate,
        chosen.repeat,
        chosen.name.as_ref(),
        &chosen.fixes,
        &never,
    );
    match attempted {
        Ok(attempted) => attempted,
        // Never set, so never cancelled.
        Err(Cancelled) => (
            Progress::Unwritable(tr("the check was stopped")),
            None,
            None,
            None,
        ),
    }
}

/// Where a try leaves a candidate: its progress, the patch text, the new
/// item's name and the try itself.
pub(crate) type Attempt = (Progress, Option<String>, Option<Ident>, Option<Tried>);

/// Writes the patch for a candidate (with its repeat, the name typed for
/// it and the fixes added) and tries it on `base`: where it stands, the
/// patch text, the new item's name and the try. Used for the chosen
/// candidate and for each option of its settings
/// ([`crate::option_fits::OptionsJob`]).
///
/// # Errors
///
/// [`Cancelled`] when `cancel` is set before the check finishes.
pub(crate) fn try_candidate(
    base: &EditBase<'_>,
    candidate: &Candidate,
    repeat: Option<Repeat>,
    name: Option<&Ident>,
    fixes: &[LangFix],
    cancel: &AtomicBool,
) -> Result<Attempt, Cancelled> {
    if let Some(names) = still_to_choose(candidate) {
        return Ok((Progress::NeedsChoice(names), None, None, None));
    }
    let scope = match EditScope::new(base.opened()) {
        Ok(scope) => scope,
        Err(error) => return Ok((Progress::Unwritable(error.to_string()), None, None, None)),
    };
    let written = match scope.write_patch_named(candidate, repeat, name) {
        Ok(written) => written,
        Err(error) => return Ok((Progress::Unwritable(error.to_string()), None, None, None)),
    };
    let mut text = written.text.clone();
    for fix in fixes {
        text.push('\n');
        text.push_str(&fix.to_string());
        text.push('\n');
    }
    let parse = match SourceMap::new().add(PATCH_NAME, text.as_str()) {
        Ok(file) => parse_patch_file(file, &text),
        Err(error) => {
            return Ok((
                Progress::Unwritable(error.to_string()),
                Some(text),
                Some(written.name),
                None,
            ));
        }
    };
    match base.try_patch(PATCH_NAME, &parse, cancel) {
        Ok(tried) => Ok((
            Progress::Tried(tried.verdict().clone()),
            Some(text),
            Some(written.name),
            Some(tried),
        )),
        Err(TryError::Load(error)) => Ok((
            Progress::Unwritable(error.to_string()),
            Some(text),
            Some(written.name),
            None,
        )),
        Err(TryError::Cancelled(cancelled)) => Err(cancelled),
    }
}

/// Binds a further pick to the first place param still to point at that
/// it can fill, in declaration order (the order the card asks for them):
/// a region to a `Region` param; a face to an `Anchor` param, or its part
/// to a `Part` param, whichever comes first, else its part to a `Parts`
/// param; two facing parts or a group to a `Parts` param, else one to
/// each `Part` param in order. `false`, with nothing changed, when it
/// fills none.
pub fn bind_further(place: &mut [crate::edit::PlaceParam], picked: &Picked) -> bool {
    let first = |place: &mut [crate::edit::PlaceParam],
                 fill: &mut dyn FnMut(&mut PlaceSlot) -> bool| {
        place.iter_mut().any(|param| fill(&mut param.slot))
    };
    match picked {
        Picked::Region(region) => first(place, &mut |slot| match slot {
            PlaceSlot::Region(slot @ None) => {
                *slot = Some(*region);
                true
            }
            PlaceSlot::Region(_)
            | PlaceSlot::Anchor(_)
            | PlaceSlot::Part(_)
            | PlaceSlot::Parts(_) => false,
        }),
        Picked::Face(face) => {
            first(place, &mut |slot| match slot {
                PlaceSlot::Anchor(slot @ None) => {
                    *slot = Some(*face);
                    true
                }
                PlaceSlot::Part(slot @ None) => {
                    *slot = Some(face.part);
                    true
                }
                PlaceSlot::Region(_)
                | PlaceSlot::Anchor(_)
                | PlaceSlot::Part(_)
                | PlaceSlot::Parts(_) => false,
            }) || first(place, &mut |slot| match slot {
                PlaceSlot::Parts(slot @ None) => {
                    *slot = Some(NonEmpty::one(face.part));
                    true
                }
                PlaceSlot::Region(_)
                | PlaceSlot::Anchor(_)
                | PlaceSlot::Part(_)
                | PlaceSlot::Parts(_) => false,
            })
        }
        Picked::Between(_) | Picked::Group(_) => {
            let parts: Vec<_> = match picked {
                Picked::Between(pair) => vec![pair.first(), pair.second()],
                Picked::Group(parts) => parts.iter().copied().collect(),
                Picked::Region(_) | Picked::Face(_) => Vec::new(),
            };
            let Some(all) = NonEmpty::new(parts.clone()) else {
                return false;
            };
            if first(place, &mut |slot| match slot {
                PlaceSlot::Parts(slot @ None) => {
                    *slot = Some(all.clone());
                    true
                }
                PlaceSlot::Region(_)
                | PlaceSlot::Anchor(_)
                | PlaceSlot::Part(_)
                | PlaceSlot::Parts(_) => false,
            }) {
                return true;
            }
            let open: Vec<usize> = place
                .iter()
                .enumerate()
                .filter(|(_, param)| matches!(param.slot, PlaceSlot::Part(None)))
                .map(|(index, _)| index)
                .collect();
            if open.len() < parts.len() {
                return false;
            }
            for (index, part) in open.into_iter().zip(parts) {
                if let Some(param) = place.get_mut(index) {
                    param.slot = PlaceSlot::Part(Some(part));
                }
            }
            true
        }
    }
}

/// A fix with every use of the new item's old name, as a name of its own
/// (not after a `.`), changed to its new name.
fn renamed_fix(fix: &LangFix, old: &str, new: &str) -> LangFix {
    let operations = rename_word(fix.operations(), old, new);
    LangFix::new(fix.label(), &operations).unwrap_or_else(|_| fix.clone())
}

/// `text` with each whole word `old` that doesn't follow a `.` replaced by
/// `new`.
fn rename_word(text: &str, old: &str, new: &str) -> String {
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    let mut previous: Option<char> = None;
    while !rest.is_empty() {
        let starts_word = previous.is_none_or(|c| !is_word(c) && c != '.');
        if starts_word
            && let Some(after) = rest.strip_prefix(old)
            && after.chars().next().is_none_or(|c| !is_word(c))
        {
            out.push_str(new);
            previous = old.chars().last();
            rest = after;
            continue;
        }
        let mut chars = rest.chars();
        if let Some(c) = chars.next() {
            out.push(c);
            previous = Some(c);
        }
        rest = chars.as_str();
    }
    out
}

/// Writes a try that fits, tried from `base` with `patch`, through `to`:
/// the one writer of the editor, which the command line's `--patch
/// --commit` uses too. Every file is checked before any is written, so a
/// file changed since `base` was read stops the commit with nothing
/// written. Gives back what was written.
///
/// # Errors
///
/// [`CommitError::DoesNotFit`] when the try doesn't fit (nothing is
/// written); [`CommitError::Write`] when a file changed since it was read
/// or can't be written.
pub fn commit_tried(
    base: &Opened,
    tried: &Tried,
    patch: &Parse<ast::PatchFile>,
    to: &mut dyn WriteEdits,
) -> Result<Vec<FileEdit>, CommitError> {
    if !tried.fits() {
        return Err(CommitError::DoesNotFit);
    }
    let files = file_edits(base, tried, Some(patch));
    write_all(&files, to)?;
    Ok(files)
}

/// Writes files' edits through `to`, checking every file before writing
/// any: a file whose text is no longer what the edits were computed from
/// stops the whole write with nothing written.
///
/// # Errors
///
/// [`CommitError::Write`] for the first file that changed since it was
/// read, or that can't be written (files before it were written).
pub fn write_all(files: &[FileEdit], to: &mut dyn WriteEdits) -> Result<(), CommitError> {
    for file in files {
        to.can_write(file).map_err(|source| CommitError::Write {
            path: file.path.clone(),
            source,
        })?;
    }
    for file in files {
        to.write_edits(file).map_err(|source| CommitError::Write {
            path: file.path.clone(),
            source,
        })?;
    }
    Ok(())
}

/// The text edits a fitting try makes, per file, in path order: computed
/// against the base's own files, so they apply to the text the edit began
/// from.
fn file_edits(
    base: &Opened,
    tried: &Tried,
    patch: Option<&Parse<ast::PatchFile>>,
) -> Vec<FileEdit> {
    let patched = EditScope::new(base)
        .ok()
        .zip(patch)
        .and_then(|(scope, patch)| apply_patch(&scope.patch_target(), patch).ok());
    let mut out: Vec<FileEdit> = tried
        .changed()
        .iter()
        .map(|changed| {
            let name = changed
                .path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned());
            let found = patched.as_ref().and_then(|patched| {
                patched
                    .iter()
                    .find(|file| Some(&file.name) == name.as_ref() && file.text == changed.text)
            });
            let before = if changed.created {
                String::new()
            } else {
                base.sources
                    .files()
                    .find(|(_, source)| Path::new(source.path()) == changed.path)
                    .map(|(_, source)| source.text().to_owned())
                    .unwrap_or_default()
            };
            let edits = match found {
                Some(file) => file.edits.clone(),
                None => vec![TextEdit {
                    range: 0..before.len(),
                    text: changed.text.clone(),
                }],
            };
            FileEdit {
                path: changed.path.clone(),
                created: changed.created,
                before,
                edits,
                after: changed.text.clone(),
            }
        })
        .collect();
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

/// Where a commit's text edits go.
pub trait WriteEdits {
    /// Makes one file's edits: applies them to the file's text, or writes
    /// its new text.
    ///
    /// # Errors
    ///
    /// The file can't be written, or its text is no longer the text the
    /// edits were computed from.
    fn write_edits(&mut self, file: &FileEdit) -> io::Result<()>;

    /// Checks, writing nothing, that [`WriteEdits::write_edits`] would
    /// find the text the edits were computed from: a commit checks every
    /// file before it writes any.
    ///
    /// # Errors
    ///
    /// The file's text is no longer the text the edits were computed
    /// from, or it can't be read.
    fn can_write(&mut self, file: &FileEdit) -> io::Result<()> {
        let _ = file;
        Ok(())
    }
}

/// The file changed since the edit began.
fn changed_since(file: &FileEdit) -> io::Error {
    io::Error::other(fill_in(
        &tr("'{file}' changed since the edit began; start the edit again"),
        &[("file", &file.path.display())],
    ))
}

/// Checks that the file on disk still holds the text the edits were
/// computed from, or, for a file the patch creates, that it still
/// doesn't exist.
fn disk_unchanged(file: &FileEdit) -> io::Result<()> {
    if file.created {
        return match std::fs::symlink_metadata(&file.path) {
            Ok(_) => Err(changed_since(file)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        };
    }
    if std::fs::read_to_string(&file.path)? == file.before {
        Ok(())
    } else {
        Err(changed_since(file))
    }
}

/// Files on disk: a file is written only while it still holds the text
/// the edits were computed from, so a change saved since the edit began
/// is never overwritten.
impl WriteEdits for DiskWriter {
    fn write_edits(&mut self, file: &FileEdit) -> io::Result<()> {
        disk_unchanged(file)?;
        if file.created {
            // Refuse, rather than replace, a file that appeared meanwhile.
            use std::io::Write as _;
            let mut out = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&file.path)
                .map_err(|error| {
                    if error.kind() == io::ErrorKind::AlreadyExists {
                        changed_since(file)
                    } else {
                        error
                    }
                })?;
            return out.write_all(file.after.as_bytes());
        }
        std::fs::write(&file.path, &file.after)
    }

    fn can_write(&mut self, file: &FileEdit) -> io::Result<()> {
        disk_unchanged(file)
    }
}

impl WriteEdits for MemoryFiles {
    fn write_edits(&mut self, file: &FileEdit) -> io::Result<()> {
        self.insert(&file.path, file.after.as_str());
        Ok(())
    }
}

/// The open buffers of an editor: an edit goes into the buffer, opened if
/// it isn't, as edits to its text, so the editor shows it unsaved and its
/// undo can take it back. A file the patch creates is written to disk
/// first (a buffer needs a file to open), then opened.
impl WriteEdits for stave_ide::Buffers {
    fn write_edits(&mut self, file: &FileEdit) -> io::Result<()> {
        let document = stave_ide::Document::File(file.path.clone());
        if file.created && !self.is_open(&document) {
            std::fs::write(&file.path, &file.after)?;
            self.open(&document).map_err(io::Error::other)?;
            return Ok(());
        }
        let current = self.open(&document).map_err(io::Error::other)?.to_owned();
        if current != file.before {
            return Err(changed_since(file));
        }
        let text = stave_lang::apply_text_edits(&current, &file.edits)
            .ok_or_else(|| io::Error::other("the edits don't fit the file's text"))?;
        self.edit(&document, &text).map_err(io::Error::other)
    }

    fn can_write(&mut self, file: &FileEdit) -> io::Result<()> {
        let document = stave_ide::Document::File(file.path.clone());
        match self.text(&document) {
            Some(text) if text != file.before => Err(changed_since(file)),
            Some(_) => Ok(()),
            None => disk_unchanged(file),
        }
    }
}

/// A writer shared with a window that keeps it too (its buffers).
impl<T: WriteEdits> WriteEdits for Rc<RefCell<T>> {
    fn write_edits(&mut self, file: &FileEdit) -> io::Result<()> {
        self.borrow_mut().write_edits(file)
    }

    fn can_write(&mut self, file: &FileEdit) -> io::Result<()> {
        self.borrow_mut().can_write(file)
    }
}

// ---------------------------------------------------------------------------
// Reading values typed in a card or a script
// ---------------------------------------------------------------------------

/// Why a typed value isn't one.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("'{0}' is not a value Stave can read")]
pub struct NotAValue(pub String);

/// Reads a value as the card or a script types it (`400mm`, `oak_19`,
/// `cabinet_depth - 20mm`), as an expression of the grammar.
///
/// # Errors
///
/// [`NotAValue`] when the text is not exactly one expression.
pub fn parse_value(text: &str) -> Result<Expr, NotAValue> {
    let not = || NotAValue(text.to_owned());
    if text.contains(['\n', '\r', '{', '}']) || text.trim().is_empty() {
        return Err(not());
    }
    let file_text = format!("stave 1\n\npatch \"Value\" {{ set value.value = {text} }}\n");
    let file = SourceMap::new()
        .add("value.stave", file_text.as_str())
        .map_err(|_| not())?;
    let parse = parse_patch_file(file, &file_text);
    if parse.has_errors() {
        return Err(not());
    }
    let ops = parse
        .ast
        .as_ref()
        .and_then(|file| file.patches.first())
        .map(|patch| patch.ops.as_slice())
        .unwrap_or_default();
    match ops {
        [only] => match &only.kind {
            ast::PatchOpKind::Set { value, .. } => Ok(value.clone()),
            ast::PatchOpKind::Add { .. }
            | ast::PatchOpKind::AddImport { .. }
            | ast::PatchOpKind::Replace { .. }
            | ast::PatchOpKind::Unset { .. }
            | ast::PatchOpKind::Remove { .. }
            | ast::PatchOpKind::RemoveImport { .. }
            | ast::PatchOpKind::Rename { .. } => Err(not()),
        },
        _ => Err(not()),
    }
}

/// A length as typed (`400mm`, `3/4in`), exactly.
///
/// # Errors
///
/// [`NotAValue`] when the text is not one length literal.
pub fn parse_length(text: &str) -> Result<Length, NotAValue> {
    let not = || NotAValue(text.to_owned());
    let expr = parse_value(text)?;
    let ast::ExprKind::Quantity(quantity) = expr.kind else {
        return Err(not());
    };
    stave_lang::units::quantity_to_length(quantity)
        .ok()
        .and_then(|exact| exact.to_length())
        .ok_or_else(not)
}

/// A name as typed (`hat_shelf`).
///
/// # Errors
///
/// [`NotAValue`] when it is not a name of the grammar.
pub fn parse_name(text: &str) -> Result<Ident, NotAValue> {
    Ident::new(text.trim(), Span::default()).map_err(|_| NotAValue(text.to_owned()))
}

/// The declaration's label with its group, as the command line names a
/// menu entry: `Shelf`, or `casework.Shelf` when the label alone is
/// ambiguous.
pub fn entry_matches(candidate: &Candidate, wanted: &str) -> bool {
    let label = candidate.entry.label.as_str();
    if wanted == label {
        return true;
    }
    let group = match &candidate.entry.group {
        crate::edit::MenuGroup::Project => return false,
        crate::edit::MenuGroup::Module(name) => name.as_str(),
        crate::edit::MenuGroup::Standard(name) => name.as_str(),
    };
    wanted
        .strip_prefix(group)
        .and_then(|rest| rest.strip_prefix('.'))
        == Some(label)
}

/// `true` when the candidate instantiates a component (writes an
/// assembly), `false` for a kind.
pub fn is_component(candidate: &Candidate) -> bool {
    match candidate.declaration {
        Declaration::Component(_) => true,
        Declaration::Kind(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_face_fills_the_first_place_the_card_asks_for() {
        use crate::edit::{PartFace, PlaceParam};
        use stave_lang::placement::Face;
        use stave_model::ids::PartId;
        // An apron's places after a click between two legs: under (the
        // top, a Part), then outside and start (faces of a leg).
        let param = |name: &str, slot: PlaceSlot| PlaceParam {
            name: Name::new(name, Span::default()).unwrap(),
            slot,
            prompt: String::new(),
            text: None,
        };
        let mut place = vec![
            param("under", PlaceSlot::Part(None)),
            param("outside", PlaceSlot::Anchor(None)),
            param("start", PlaceSlot::Anchor(None)),
        ];
        let top = PartFace {
            part: PartId::from_usize(0),
            face: Face::Top,
        };
        let leg = PartFace {
            part: PartId::from_usize(1),
            face: Face::Front,
        };
        assert!(bind_further(&mut place, &Picked::Face(top)));
        assert!(bind_further(&mut place, &Picked::Face(leg)));
        let slots: Vec<&PlaceSlot> = place.iter().map(|param| &param.slot).collect();
        assert_eq!(
            slots,
            [
                &PlaceSlot::Part(Some(top.part)),
                &PlaceSlot::Anchor(Some(leg)),
                &PlaceSlot::Anchor(None),
            ]
        );
    }

    #[test]
    fn a_rename_changes_whole_words_only() {
        assert_eq!(
            rename_word("set shelf_2.headroom = 100mm", "shelf_2", "hat"),
            "set hat.headroom = 100mm"
        );
        assert_eq!(
            rename_word("set shelf_22.x = a.shelf_2", "shelf_2", "hat"),
            "set shelf_22.x = a.shelf_2"
        );
    }

    #[test]
    fn values_read_as_expressions() {
        assert!(parse_value("400mm").is_ok());
        assert!(parse_value("cabinet_depth - 20mm").is_ok());
        assert!(parse_value("oak_19").is_ok());
        assert!(parse_value("").is_err());
        assert!(parse_value("400mm }").is_err());
        assert_eq!(parse_length("400mm"), Ok(Length::from_mm(400)));
        assert!(parse_length("oak_19").is_err());
    }
}
