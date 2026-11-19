//! Grips in the session (`docs/editing.md`, "Grips in the session"): the
//! grips of the selection listed, their stops found without holding the
//! window up, and a drag on one, tried at each value like any edit and
//! committed through the one writer.
//!
//! ```text
//! ListGrips(selection) ──grips()──▶ GripList { grips }        (no stops yet: fast)
//!                       StopsJob::run on another thread ──▶ FoundStops ──Session::grip_stops_found──▶ stops
//! BeginDrag(i) ──▶ a Change holding a GripDrag
//! DragTo(value) ──▶ movable: `set path = value` (snapped)
//!                   follows, solved params: the solver's Drag (solved once, exact arithmetic after)
//!                   follows, one plain param: single-param inversion
//!                   follows, several: the list to pick from (PickParam)
//!                   follows, none: what stops it
//!               ──▶ scratch check ──▶ verdict and ghost
//! Add ──▶ the one writer      Cancel ──▶ nothing written
//! ```
//!
//! The grips are listed from one prepared base, which the drag shares, so
//! the ids a grip carries are those the drag reads. A commit makes the
//! list stale, so the session drops it.

use crate::i18n::{fill_in, tr};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use stave_lang::ast::{Name, Span};
use stave_lang::units::Length;
use stave_model::ids::ParamId;
use stave_model::nonempty::NonEmpty;
use stave_model::trace::{Amount, Quantity};
use stave_modules::FileSource;

use crate::Opened;
use crate::candidates::EditScope;
use crate::edit::Grip;
use crate::editing::{Progress, StartError};
use crate::grips::{
    GripAction, GripMove, GripOptions, GripSelection, GripSize, MoveError, SelectionGrip,
    grips as list_grips, move_grip, move_grip_by,
};
use crate::scratch::EditBase;
use crate::solve_edit::{Blocker, Drag, DragStart, DragStep, ParamChange, changes_patch};
use crate::stops::{
    Bounds, Side, StopsError, StopsRequest, amount_text, find_stops, param_at_path,
};

/// How far either way from its value a length with no `within` is
/// searched for stops, and how far a grip on it ranges before a search.
pub const GRIP_REACH: Length = Length::from_mm(1000);

/// One listing of grips, numbered so a stale search's answer can be told
/// apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GripsTicket(pub(crate) u64);

/// The stops of one grip, as far as they are known.
#[derive(Debug, Clone, PartialEq)]
pub enum GripStops {
    /// A movable param whose stops haven't been searched yet: its range
    /// is its `within`, or [`GRIP_REACH`] either way.
    NotSearched,
    /// A grip with no stops to search: a follows grip, or the boundary of
    /// a split (its range is the two pieces it shares).
    NotSearchable,
    /// Searched: the last good value each way and what stops it. The
    /// grip's range is now between the two last good values.
    Found {
        /// Going down.
        below: Box<Side>,
        /// Going up.
        above: Box<Side>,
    },
    /// The search failed, in words; the range is as before.
    Failed(String),
}

/// One grip of the listed selection.
#[derive(Debug, Clone, PartialEq)]
pub struct ListedGrip {
    /// The grip: its size, label, axis, what it moves or follows, and what
    /// the scene's `grip_place` needs ([`SelectionGrip::place`]).
    pub grip: SelectionGrip,
    /// Its stops.
    pub stops: GripStops,
}

/// The grips of a selection, listed for the window to draw.
pub struct GripList {
    ticket: GripsTicket,
    selection: GripSelection,
    folder: PathBuf,
    files: Arc<dyn FileSource + Send + Sync>,
    base: Rc<EditBase<'static>>,
    grips: Vec<ListedGrip>,
}

impl std::fmt::Debug for GripList {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GripList")
            .field("ticket", &self.ticket)
            .field("selection", &self.selection)
            .field("grips", &self.grips)
            .finish_non_exhaustive()
    }
}

impl GripList {
    /// Lists the grips of `selection` in the project in `folder`, without
    /// searching their stops.
    pub(crate) fn list(
        folder: &Path,
        files: Arc<dyn FileSource + Send + Sync>,
        selection: GripSelection,
        ticket: GripsTicket,
    ) -> Result<GripList, String> {
        let base = EditBase::shared(folder, files.clone()).map_err(|e| e.to_string())?;
        if base.opened().design.is_none() {
            return Err(StartError::NotSolved.to_string());
        }
        let never = AtomicBool::new(false);
        let options = GripOptions {
            search_stops: false,
            reach: GRIP_REACH,
            cancel: &never,
        };
        let grips = list_grips(&base, selection, options)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|grip| {
                let stops = if search_path(&grip).is_some() {
                    GripStops::NotSearched
                } else {
                    GripStops::NotSearchable
                };
                ListedGrip { grip, stops }
            })
            .collect();
        Ok(GripList {
            ticket,
            selection,
            folder: folder.to_path_buf(),
            files,
            base: Rc::new(base),
            grips,
        })
    }

    /// This listing's ticket: a [`FoundStops`] for another is dropped.
    pub fn ticket(&self) -> GripsTicket {
        self.ticket
    }

    /// What the grips are of.
    pub fn selection(&self) -> GripSelection {
        self.selection
    }

    /// The grips, in [`crate::grips::grips`]' order; commands name a grip
    /// by its index here.
    pub fn grips(&self) -> &[ListedGrip] {
        &self.grips
    }

    /// The design the grips were listed from.
    pub fn base(&self) -> &Opened {
        self.base.opened()
    }

    /// The contract's [`Grip`] for the grip at `index`, when it can say
    /// it ([`SelectionGrip::contract`]).
    pub fn contract(&self, index: usize) -> Option<Grip> {
        let model = self.base.opened().model.as_ref()?;
        self.grips.get(index)?.grip.contract(model)
    }

    /// The search for the stops of every grip not searched yet, to run off
    /// the main thread ([`StopsJob::run`]) and hand back grip by grip.
    pub fn stops_job(&self) -> StopsJob {
        let searches = self
            .grips
            .iter()
            .enumerate()
            .filter(|(_, listed)| listed.stops == GripStops::NotSearched)
            .filter_map(|(index, listed)| {
                let (path, limit) = search_path(&listed.grip)?;
                Some(StopsSearch { index, path, limit })
            })
            .collect();
        StopsJob {
            folder: self.folder.clone(),
            files: self.files.clone(),
            ticket: self.ticket,
            searches,
        }
    }

    /// Takes a search's answer; `false` when it is for another listing or
    /// a grip it doesn't have.
    pub(crate) fn found(&mut self, found: FoundStops) -> bool {
        if found.ticket != self.ticket {
            return false;
        }
        let Some(listed) = self.grips.get_mut(found.index) else {
            return false;
        };
        match found.result {
            Ok((below, above)) => {
                if let GripAction::Moves(movable) = &mut listed.grip.action {
                    movable.low = below.last_good;
                    movable.high = above.last_good;
                    movable.stops = Some((below.clone(), above.clone()));
                }
                listed.stops = GripStops::Found {
                    below: Box::new(below),
                    above: Box::new(above),
                };
            }
            Err(message) => listed.stops = GripStops::Failed(message),
        }
        true
    }

    /// The shared base, for a drag.
    pub(crate) fn shared_base(&self) -> Rc<EditBase<'static>> {
        self.base.clone()
    }
}

/// The `set` path and search limit of a movable param grip.
fn search_path(grip: &SelectionGrip) -> Option<(String, Bounds)> {
    let (
        GripSize::Param {
            path: Some(path), ..
        },
        GripAction::Moves(movable),
    ) = (&grip.size, &grip.action)
    else {
        return None;
    };
    Some((
        path.clone(),
        Bounds {
            low: movable.low,
            high: movable.high,
        },
    ))
}

/// One grip's search.
#[derive(Debug, Clone, PartialEq)]
pub struct StopsSearch {
    /// The grip's index in its listing.
    pub index: usize,
    /// The param's `set` path.
    pub path: String,
    /// How far to look when the param has no `within`.
    pub limit: Bounds,
}

/// The search for grips' stops, apart from the session so it can run on
/// another thread: it prepares the project again itself.
#[derive(Clone)]
pub struct StopsJob {
    folder: PathBuf,
    files: Arc<dyn FileSource + Send + Sync>,
    ticket: GripsTicket,
    searches: Vec<StopsSearch>,
}

impl std::fmt::Debug for StopsJob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StopsJob")
            .field("folder", &self.folder)
            .field("ticket", &self.ticket)
            .field("searches", &self.searches)
            .finish_non_exhaustive()
    }
}

/// One grip's stops, found by a [`StopsJob`], for
/// [`crate::session::Session::grip_stops_found`].
#[derive(Debug, Clone, PartialEq)]
pub struct FoundStops {
    /// The listing it was searched for.
    pub ticket: GripsTicket,
    /// The grip's index in it.
    pub index: usize,
    /// The last good value each way with what stops it, or why the search
    /// failed.
    pub result: Result<(Side, Side), String>,
}

impl StopsJob {
    /// The searches it will run, in grip order.
    pub fn searches(&self) -> &[StopsSearch] {
        &self.searches
    }

    /// The same job for the grip at `index` only.
    pub fn only(mut self, index: usize) -> StopsJob {
        self.searches.retain(|search| search.index == index);
        self
    }

    /// Runs each search in turn, handing each grip's answer to `found` as
    /// soon as it is known. Setting `cancel` stops it between checks; the
    /// grips reached keep their answer.
    ///
    /// # Errors
    ///
    /// The project can't be prepared, or the search was cancelled, in
    /// words.
    pub fn run(
        &self,
        cancel: &AtomicBool,
        found: &mut dyn FnMut(FoundStops),
    ) -> Result<(), String> {
        if self.searches.is_empty() {
            return Ok(());
        }
        let base = EditBase::shared(&self.folder, self.files.clone()).map_err(|e| e.to_string())?;
        for search in &self.searches {
            let request = StopsRequest {
                path: &search.path,
                limit: search.limit,
            };
            let result = match find_stops(&base, request, cancel) {
                Ok(stops) => Ok((stops.below, stops.above)),
                Err(StopsError::Cancelled) => return Err(StopsError::Cancelled.to_string()),
                Err(error) => Err(error.to_string()),
            };
            found(FoundStops {
                ticket: self.ticket,
                index: search.index,
                result,
            });
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// A drag
// ---------------------------------------------------------------------------

/// What a drag found at its last value, for the card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GripDetail {
    /// The grip's index in the listing.
    pub index: usize,
    /// The grip's label: `divider.height`.
    pub label: String,
    /// Where it was last moved to, as asked (before snapping).
    pub target: Option<Amount>,
    /// The params the drag changes and to what, in declaration order:
    /// the value written for a movable grip, the inverted param for a
    /// follows grip, every solved param the solver moves. Empty before
    /// the first move, for a boundary (its shares are in the patch) and
    /// while a choice is waited for.
    pub changes: Vec<ParamChange>,
    /// When several plain params could give the size: each, with the
    /// change it would make or why it can't. Pick one with
    /// `EditCommand::PickParam`.
    pub choices: Vec<GripChoice>,
    /// The param picked from [`GripDetail::choices`].
    pub picked: Option<ParamId>,
    /// When nothing the size follows can change: what stops it, in words.
    pub stopped: Option<String>,
    /// When the solver can't reach the value: what blocks it (the
    /// requires by name, the `within`s), smallest set first.
    pub blockers: Vec<Blocker>,
    /// `true` when the size depends on solved params and the solver
    /// chooses what changes.
    pub solved: bool,
    /// The size the written values give, when snapping to the resolution
    /// made it miss the target.
    pub reached: Option<Amount>,
}

/// One plain param a follows grip could change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GripChoice {
    /// The param.
    pub param: ParamId,
    /// Its name as the grip lists it: `overall_width`, `case.overhang`.
    pub name: String,
    /// The change it would make, or why it can't give the size.
    pub change: Result<ParamChange, String>,
}

/// Whether the solver handles the drag.
#[derive(Debug)]
enum Solver {
    NotTried,
    Fallback,
    Ready(Box<Drag>),
}

/// A drag on one grip in progress, held by its change.
#[derive(Debug)]
pub(crate) struct GripDrag {
    grip: SelectionGrip,
    quantity: Option<Quantity>,
    solver: Solver,
    detail: GripDetail,
}

/// What a move gives: a patch to try, or where the drag stands without
/// one.
pub(crate) enum DragOutcome {
    /// Try this patch.
    Patch {
        /// What it does: "Set divider.height to 400mm".
        summary: String,
        /// The patch file.
        text: String,
    },
    /// Nothing to try: a choice to make, or why not.
    Stands(Progress),
}

impl GripDrag {
    /// A drag on `grip`, at `index` of its listing.
    pub(crate) fn new(index: usize, grip: SelectionGrip) -> GripDrag {
        let quantity = match &grip.size {
            GripSize::Param { param, .. } => Some(Quantity::Param(*param)),
            GripSize::PartSize { part, axis } => Some(Quantity::PartSize {
                part: *part,
                axis: *axis,
            }),
            GripSize::RegionSize { region, axis } => Some(Quantity::RegionSize {
                region: *region,
                axis: *axis,
            }),
            GripSize::Boundary { .. } => None,
        };
        let detail = GripDetail {
            index,
            label: grip.label.clone(),
            target: None,
            changes: Vec::new(),
            choices: Vec::new(),
            picked: None,
            stopped: None,
            blockers: Vec::new(),
            solved: false,
            reached: None,
        };
        GripDrag {
            grip,
            quantity,
            solver: Solver::NotTried,
            detail,
        }
    }

    /// What the drag found at its last value.
    pub(crate) fn detail(&self) -> &GripDetail {
        &self.detail
    }

    /// The value it was last moved to.
    pub(crate) fn target(&self) -> Option<Amount> {
        self.detail.target
    }

    /// The patch for moving the grip to `target`, tried from `base`.
    pub(crate) fn move_to(&mut self, base: &EditBase<'_>, target: Amount) -> DragOutcome {
        let detail = &mut self.detail;
        detail.target = Some(target);
        detail.changes.clear();
        detail.choices.clear();
        detail.stopped = None;
        detail.blockers.clear();
        detail.reached = None;
        let value = match &self.grip.action {
            GripAction::Moves(movable) => movable.value,
            GripAction::Follows(follows) => follows.value,
        };
        if !target.same_dimension(value) {
            return stands(MoveError::Mismatch.to_string());
        }
        // A size worked out from solved params: the solver chooses.
        if let (GripAction::Follows(_), Some(quantity), Solver::NotTried) =
            (&self.grip.action, self.quantity, &self.solver)
        {
            self.solver = match Drag::start(base, quantity, target) {
                Ok(DragStart::Ready(drag)) => Solver::Ready(Box::new(drag)),
                Ok(DragStart::Fallback(_)) | Err(_) => Solver::Fallback,
            };
        }
        if let Solver::Ready(drag) = &mut self.solver {
            self.detail.solved = true;
            return solved_move(drag, &mut self.detail, target);
        }
        let opened = base.opened();
        let (Ok(scope), Some(design)) = (EditScope::new(opened), opened.design.as_ref()) else {
            return stands(StartError::NotSolved.to_string());
        };
        let model = scope.model();
        let change = |path: &str, before: Amount, after: Amount| {
            param_at_path(model, path).map(|param| ParamChange {
                param,
                path: path.to_owned(),
                from: before,
                to: after,
            })
        };
        match move_grip(&scope, design, &self.grip, target) {
            Ok(GripMove::Set(patch)) => {
                self.detail
                    .changes
                    .extend(change(&patch.path, patch.before, patch.after));
                DragOutcome::Patch {
                    summary: patch.patch.summary.clone(),
                    text: patch.patch.text.clone(),
                }
            }
            Ok(GripMove::Choose(choices)) => {
                self.detail.choices = choices
                    .iter()
                    .filter_map(|choice| {
                        let stave_model::trace::Source::Param(param) = choice.followed.source
                        else {
                            return None;
                        };
                        Some(GripChoice {
                            param,
                            name: choice.followed.name.clone(),
                            change: match &choice.result {
                                Ok(patch) => change(&patch.path, patch.before, patch.after)
                                    .ok_or_else(|| MoveError::NotFollowed.to_string()),
                                Err(error) => Err(error.to_string()),
                            },
                        })
                    })
                    .collect();
                match self.detail.picked {
                    Some(param) => match move_grip_by(model, &self.grip, param, target) {
                        Ok(patch) => {
                            self.detail.changes.extend(change(
                                &patch.path,
                                patch.before,
                                patch.after,
                            ));
                            DragOutcome::Patch {
                                summary: patch.patch.summary.clone(),
                                text: patch.patch.text.clone(),
                            }
                        }
                        Err(error) => stands(error.to_string()),
                    },
                    None => {
                        let names: Vec<Name> = self
                            .detail
                            .choices
                            .iter()
                            .filter_map(|choice| choice_word(&choice.name))
                            .collect();
                        match NonEmpty::new(names) {
                            Some(names) => DragOutcome::Stands(Progress::NeedsChoice(names)),
                            None => stands(tr("pick which size changes")),
                        }
                    }
                }
            }
            Err(error @ MoveError::Stopped(_)) => {
                let words = error.to_string();
                self.detail.stopped = Some(words.clone());
                stands(words)
            }
            Err(error) => stands(error.to_string()),
        }
    }

    /// Picks the plain param a follows grip changes, from its choices.
    /// `false` when it is not one of them.
    pub(crate) fn pick(&mut self, param: ParamId) -> bool {
        if !self
            .detail
            .choices
            .iter()
            .any(|choice| choice.param == param)
        {
            return false;
        }
        self.detail.picked = Some(param);
        true
    }
}

/// The word a choice is asked for by: the param's own name, the last
/// part of `drawer[top].top_gap`.
fn choice_word(name: &str) -> Option<Name> {
    let tail = name.rsplit('.').next().unwrap_or(name);
    let word = tail.split('[').next().unwrap_or(tail);
    Name::new(word, Span::default()).ok()
}

fn stands(reason: String) -> DragOutcome {
    DragOutcome::Stands(Progress::Unwritable(reason))
}

/// A move worked out by the solver's drag.
fn solved_move(drag: &mut Drag, detail: &mut GripDetail, target: Amount) -> DragOutcome {
    match drag.values_at(target) {
        Ok(DragStep::Values(values)) => {
            let (changes, reached) = match drag.changes_at(&values) {
                Ok(found) => found,
                Err(error) => return stands(error.to_string()),
            };
            if reached != target {
                detail.reached = Some(reached);
            }
            if changes.is_empty() {
                return stands(tr("the size is already that"));
            }
            let patch = match changes_patch(&changes) {
                Ok(patch) => patch,
                Err(error) => return stands(error.to_string()),
            };
            detail.changes = changes;
            DragOutcome::Patch {
                summary: patch.summary,
                text: patch.text,
            }
        }
        Ok(DragStep::Unreachable(blockers)) => {
            let words = blocked_text(target, &blockers);
            detail.blockers = blockers;
            stands(words)
        }
        Ok(DragStep::TimedOut) => stands(tr("the solver ran out of time")),
        Err(error) => stands(error.to_string()),
    }
}

/// Why the solver can't reach `target`, naming each require that blocks
/// it: "can't reach 1500mm: the rule 'fits_the_room' stops it (the cabinet
/// must fit the alcove)".
pub fn blocked_text(target: Amount, blockers: &[Blocker]) -> String {
    let value = amount_text(target).unwrap_or_else(|| tr("that value"));
    let mut reasons = Vec::new();
    for blocker in blockers {
        match blocker {
            Blocker::Require { name, message } => {
                reasons.push(fill_in(
                    &tr("the rule '{name}' stops it ({message})"),
                    &[("name", name), ("message", message)],
                ));
            }
            Blocker::Within { path, .. } => {
                reasons.push(fill_in(
                    &tr("'{path}' would leave its range"),
                    &[("path", path)],
                ));
            }
            Blocker::Size => {}
        }
    }
    if reasons.is_empty() {
        fill_in(&tr("can't reach {value}"), &[("value", &value)])
    } else {
        fill_in(
            &tr("can't reach {value}: {reasons}"),
            &[("value", &value), ("reasons", &reasons.join("; "))],
        )
    }
}
