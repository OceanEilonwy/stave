//! Grips for a selection, and what moving one writes (`docs/editing.md`,
//! "Grips"; grammar, "Smart edits", rules 1 and 2).
//!
//! ```text
//! selection ──grips()──▶ [SelectionGrip]  Moves: a plain number, its range and stops
//!                                          Follows: what an expression follows, and how
//! SelectionGrip + target ──move_grip()──▶ GripMove: one patch, or the plain params to pick from
//! ```
//!
//! - [`grips`] lists the grips of a selected assembly, of the envelope, or
//!   of the boundaries of a split (a repeat). A param written as a plain
//!   number is a *movable* grip, with its range (its `within`, or the
//!   stops [`crate::stops::find_stops`] finds) and its stops with their
//!   reasons. A size written as an expression (a param's default, a
//!   part's length or width) is a *follows* grip that names what it
//!   follows, from the dependency trace ([`stave_solve::trace`]), each
//!   with its kind: plain, solved, locked, or a stock's or a kind's size,
//!   which stop the chain. A length grip also carries what the scene's
//!   [`stave_scene::SceneGraph::grip_place`] needs and can't know itself:
//!   every length param of the assembly with its value, and the two faces
//!   the length is measured between, read from the placement term the
//!   param is the gap or inset of.
//! - [`move_grip`] gives the patch for a drag: `set <path> = <value>` for a
//!   movable grip, snapped to the project's `resolution:` or to the
//!   `within` list; for a follows grip, the single-param inversion
//!   ([`stave_model::trace::invert`]) on its one plain param, with the
//!   value before and after; the plain params to pick from when there are
//!   several ([`move_grip_by`] then makes the patch for the one picked); and
//!   a refusal naming what stops it when there are none. A solved param is
//!   treated as a plain one: solving for several at once is a later stage.
//!
//! The contract's [`Grip`] can't say everything a grip here carries (a
//! part's size has no [`GripTarget`]; a component param's path is not an
//! [`Ident`]; a movable grip's stops have no reasons), so these are richer
//! types, and [`SelectionGrip::contract`] gives the contract value where
//! one can be made.

use std::sync::atomic::AtomicBool;

use stave_lang::ast::{Ident, Span};
use stave_lang::placement::{Axis, Face, PartAxis};
use stave_lang::units::{Angle, Length, Rational};
use stave_model::Model;
use stave_model::defs::{Member, PartKind, PlaceTermKind, RExpr, RExprKind, StockField};
use stave_model::ids::{AssemblyId, ParamId, PartId, RegionId};
use stave_model::instance::ItemSlot;
use stave_model::nonempty::NonEmpty;
use stave_model::solved::SolvedDesign;
use stave_model::trace::{
    Amount, Dependence, Form, Freedom, InvertError, ParamDomain, Quantity, Source, Trace,
};
use stave_scene::{FaceOf, GripFace};
use stave_schema::functions::Builtin;

use crate::candidates::EditScope;
use crate::edit::{Follows, Grip, GripRange, GripTarget, Handle, Movable, Stops};
use crate::repeat::{GeneratedPatch, PatchBuildError, RepeatError, SharesMove, move_shares};
use crate::scratch::EditBase;
use crate::stops::{
    Bounds, SearchSteps, Side, StopsError, StopsRequest, amount_text, find_stops, path_of_param,
    snap, step_for, written_number,
};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// What the grips are asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GripSelection {
    /// A selected assembly: its params and its parts' sizes.
    Assembly(AssemblyId),
    /// The envelope: the three sizes of the project's root box.
    Envelope,
    /// One piece of a split: the boundaries between its neighbours.
    Split(RegionId),
}

/// How much work [`grips`] does.
#[derive(Debug, Clone, Copy)]
pub struct GripOptions<'a> {
    /// Search each movable grip's stops ([`find_stops`]): tens of checks
    /// per grip. Without it a grip's range is its `within`, or the reach.
    pub search_stops: bool,
    /// How far either way from its value a length with no `within` may be
    /// searched (or ranges, without a search). Other values with no
    /// `within` reach a hundred steps.
    pub reach: Length,
    /// Stops a search between checks.
    pub cancel: &'a AtomicBool,
}

/// A grip on a selection.
#[derive(Debug, Clone, PartialEq)]
pub struct SelectionGrip {
    /// What size it is.
    pub size: GripSize,
    /// The size's name, as the card shows it: `divider.height`,
    /// `drawer[bottom].sub_front.length`, `drawer_bay[bottom|top]`.
    pub label: String,
    /// The world axis it drags along, when it has one.
    pub axis: Option<Axis>,
    /// A number to move, or what the size follows.
    pub action: GripAction,
    /// What the scene needs to place the handle.
    pub place: Option<PlaceInput>,
}

/// What size a grip is on.
#[derive(Debug, Clone, PartialEq)]
pub enum GripSize {
    /// A param's value. `path` is what a `set` writes (`divider.height`),
    /// `None` for a param no patch can address (one deeper inside).
    Param {
        /// The param (for an array of assemblies, the first element's).
        param: ParamId,
        /// Its `set` path.
        path: Option<String>,
    },
    /// A part's drawn length or width.
    PartSize {
        /// The part.
        part: PartId,
        /// Which size.
        axis: PartAxis,
    },
    /// A size of a region (the envelope's, when it is not one param).
    RegionSize {
        /// The region.
        region: RegionId,
        /// Which size.
        axis: Axis,
    },
    /// The boundary between two neighbouring pieces of a split.
    Boundary {
        /// The region array the split makes: `drawer_bay`.
        region: String,
        /// The index of the first of the two pieces, in label order.
        lower: usize,
        /// The piece below (left of, in front of) the boundary.
        low: RegionId,
        /// The piece above it.
        high: RegionId,
    },
}

/// What a grip does.
#[derive(Debug, Clone, PartialEq)]
pub enum GripAction {
    /// The size is a plain number: the grip moves it.
    Moves(Box<MovableGrip>),
    /// The size is an expression: the grip shows what it follows.
    Follows(Box<FollowsGrip>),
}

/// A plain number a grip moves.
#[derive(Debug, Clone, PartialEq)]
pub struct MovableGrip {
    /// The value now. For a boundary, the size of the low piece along the
    /// split.
    pub value: Amount,
    /// The lowest value it may take: the `within`'s low end, or the last
    /// good value of the stops below, or the reach.
    pub low: Amount,
    /// The highest.
    pub high: Amount,
    /// Where it snaps.
    pub steps: SearchSteps,
    /// The param's kind: plain or solved (a locked param has no grip).
    pub freedom: Freedom,
    /// The stops going down and up, with what stops each, when searched.
    pub stops: Option<(Side, Side)>,
}

/// What a size written as an expression follows.
#[derive(Debug, Clone, PartialEq)]
pub struct FollowsGrip {
    /// The value now.
    pub value: Amount,
    /// The trace it was read from, for the inversion.
    pub trace: Trace,
    /// The values it follows, in the trace's order.
    pub follows: Vec<FollowedValue>,
}

/// One value a size follows.
#[derive(Debug, Clone, PartialEq)]
pub struct FollowedValue {
    /// The value in the trace.
    pub source: Source,
    /// Its name: `overall_width`, `drawer[top].top_gap`, `maple_12.thickness`,
    /// `side_mount_slide.clearance`.
    pub name: String,
    /// The `set` path of a param a patch can write.
    pub path: Option<String>,
    /// Its kind.
    pub kind: FollowKind,
    /// How much the size changes for each unit it changes (raw units, see
    /// [`Amount::raw`]), when the size is linear in it; `None` when it is
    /// not, and it is not offered (rule 2).
    pub coefficient: Option<Rational>,
    /// Its value now.
    pub value: Amount,
}

/// The kind of a value a size follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FollowKind {
    /// A plain `param`: offered.
    Plain,
    /// A `solved param`: offered, as a plain one until z3 is used.
    Solved,
    /// A `locked param`: stops the chain.
    Locked,
    /// A stock's field: stops the chain.
    StockSize,
    /// A kind's own size: stops the chain.
    KindSize,
}

impl FollowKind {
    fn of(freedom: Freedom) -> Self {
        match freedom {
            Freedom::Plain => FollowKind::Plain,
            Freedom::Solved => FollowKind::Solved,
            Freedom::Locked => FollowKind::Locked,
            Freedom::StockField => FollowKind::StockSize,
            Freedom::KindSize => FollowKind::KindSize,
        }
    }

    /// `true` for a value a drag may change.
    pub fn is_candidate(self) -> bool {
        matches!(self, FollowKind::Plain | FollowKind::Solved)
    }

    /// A short phrase for the card: "locked", "a stock size".
    pub fn words(self) -> &'static str {
        match self {
            FollowKind::Plain => "plain",
            FollowKind::Solved => "solved",
            FollowKind::Locked => "locked",
            FollowKind::StockSize => "a size of the stock",
            FollowKind::KindSize => "a size of the hardware or joint",
        }
    }
}

/// What [`stave_scene::SceneGraph::grip_place`] needs for a grip, owned
/// (its [`stave_scene::GripOf`] borrows).
#[derive(Debug, Clone, PartialEq)]
pub enum PlaceInput {
    /// A length param of an assembly.
    Length {
        /// The assembly.
        assembly: AssemblyId,
        /// The world axis it runs along.
        axis: Axis,
        /// The param's name in the assembly: `height`.
        param: String,
        /// Every length param of the assembly with its value: the
        /// scene's `lengths`.
        lengths: Vec<(String, Length)>,
        /// The faces it is measured between, from the placement term it is
        /// the gap or inset of: the face it is measured from (its owner
        /// found as the region or part outside the assembly with that face
        /// in that plane, the region first) and the part's face that moves.
        /// `None` when the param is not one term's gap or inset.
        between: Option<(GripFace, GripFace)>,
    },
    /// A size of the envelope.
    Envelope {
        /// The box.
        region: RegionId,
        /// The face that moves: right, top or back.
        face: Face,
    },
    /// A boundary of a split.
    Share {
        /// The low neighbour.
        low: RegionId,
        /// The high neighbour.
        high: RegionId,
        /// The split's axis.
        axis: Axis,
    },
}

/// Why grips can't be listed.
#[derive(Debug, thiserror::Error)]
pub enum GripsError {
    /// The base design does not solve.
    #[error("the design doesn't place yet, so it has no grips; fix its errors first")]
    NoDesign,
    /// The selection names nothing in the design.
    #[error("the selection is not in this design")]
    NotInDesign,
    /// The project has no envelope box.
    #[error("this project has no envelope: its root is a part, not a box")]
    NoEnvelope,
    /// The region is not a piece of a split.
    #[error("this space is not one piece of a split, so it has no boundaries to move")]
    NotASplit,
    /// A stops search failed.
    #[error(transparent)]
    Stops(#[from] StopsError),
}

// ---------------------------------------------------------------------------
// Listing
// ---------------------------------------------------------------------------

/// The grips of a selection, in a fixed order: for an assembly its params
/// in declaration order, then its parts' lengths and widths; for the
/// envelope across, up, deep; for a split each boundary in order.
///
/// A `locked param` has no grip. A param that is not a quantity (a
/// region, a stock) has none. A follows grip may follow nothing that can
/// change (an expression of numbers only); its list is then empty.
///
/// # Errors
///
/// See [`GripsError`]; with `search_stops`, a cancelled search.
pub fn grips(
    base: &EditBase<'_>,
    selection: GripSelection,
    options: GripOptions<'_>,
) -> Result<Vec<SelectionGrip>, GripsError> {
    let opened = base.opened();
    let (Some(model), Some(design)) = (opened.model.as_ref(), opened.design.as_ref()) else {
        return Err(GripsError::NoDesign);
    };
    let lister = Lister {
        base,
        model,
        design,
        options,
    };
    match selection {
        GripSelection::Assembly(assembly) => lister.assembly(assembly),
        GripSelection::Envelope => lister.envelope(),
        GripSelection::Split(region) => lister.split(region),
    }
}

struct Lister<'a, 'b, 'f> {
    base: &'b EditBase<'f>,
    model: &'a Model,
    design: &'a SolvedDesign,
    options: GripOptions<'a>,
}

impl Lister<'_, '_, '_> {
    fn assembly(&self, assembly: AssemblyId) -> Result<Vec<SelectionGrip>, GripsError> {
        let model = self.model;
        let inst = model
            .assemblies
            .get(assembly.index())
            .ok_or(GripsError::NotInDesign)?;
        let solved = self
            .design
            .assemblies
            .iter()
            .find(|a| a.id == assembly)
            .ok_or(GripsError::NotInDesign)?;
        let params: Vec<ParamId> = model
            .instance(inst.inner)
            .map(|instance| {
                instance
                    .items
                    .values()
                    .filter_map(|slot| match slot {
                        ItemSlot::Param(param) => Some(*param),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        let parts: Vec<PartId> = solved.parts.clone();
        // Everything is traced in one solve.
        let mut quantities: Vec<Quantity> = params.iter().map(|p| Quantity::Param(*p)).collect();
        for part in &parts {
            for axis in [Axis::Across, Axis::Up, Axis::Deep] {
                quantities.push(Quantity::PlaceOffset { part: *part, axis });
            }
            for axis in [PartAxis::Length, PartAxis::Width] {
                quantities.push(Quantity::PartSize { part: *part, axis });
            }
        }
        let traces: Vec<Option<Trace>> = stave_solve::trace::trace_all(model, &quantities)
            .into_iter()
            .map(Result::ok)
            .collect();
        let traced = |quantity: Quantity| {
            quantities
                .iter()
                .position(|q| *q == quantity)
                .and_then(|index| traces.get(index))
                .and_then(Option::as_ref)
        };
        let lengths: Vec<(String, Length)> = params
            .iter()
            .filter_map(|param| {
                let trace = traced(Quantity::Param(*param))?;
                let Amount::Length(length) = trace.value else {
                    return None;
                };
                Some((param_name(model, *param)?, length.to_length()?))
            })
            .collect();
        let mut out = Vec::new();
        for param in &params {
            let Some(trace) = traced(Quantity::Param(*param)) else {
                continue;
            };
            let path = path_of_param(model, *param);
            let label = path
                .clone()
                .unwrap_or_else(|| full_name(model, *param).unwrap_or_default());
            let measure = self.measure(assembly, *param, &parts, &traced);
            let axis = measure.as_ref().map(|m| m.0);
            let size = GripSize::Param {
                param: *param,
                path: path.clone(),
            };
            if written_number(model, *param).is_some() {
                let own = Source::Param(crate::stops::original(model, *param));
                let Form::Linear(linear) = &trace.form else {
                    continue;
                };
                let Some(term) = linear.term(&own) else {
                    continue;
                };
                if term.dependence.freedom == Freedom::Locked {
                    continue;
                }
                let Some(path) = path else {
                    continue;
                };
                let movable = self.movable(&path, &term.dependence)?;
                let place = match (&trace.value, measure) {
                    (Amount::Length(_), Some((axis, between))) => Some(PlaceInput::Length {
                        assembly,
                        axis,
                        param: param_name(model, *param).unwrap_or_default(),
                        lengths: lengths.clone(),
                        between,
                    }),
                    _ => None,
                };
                out.push(SelectionGrip {
                    size,
                    label,
                    axis,
                    action: GripAction::Moves(Box::new(movable)),
                    place,
                });
            } else {
                out.push(SelectionGrip {
                    size,
                    label,
                    axis,
                    action: GripAction::Follows(Box::new(follows(model, trace))),
                    place: None,
                });
            }
        }
        for part in &parts {
            let Some(solved_part) = self.design.part(*part) else {
                continue;
            };
            for axis in [PartAxis::Length, PartAxis::Width] {
                let Some(trace) = traced(Quantity::PartSize { part: *part, axis }) else {
                    continue;
                };
                out.push(SelectionGrip {
                    size: GripSize::PartSize { part: *part, axis },
                    label: format!("{}.{}", solved_part.name, axis.word()),
                    axis: Some(solved_part.pose.orientation.direction(axis).axis()),
                    action: GripAction::Follows(Box::new(follows(model, trace))),
                    place: None,
                });
            }
        }
        Ok(out)
    }

    /// The world axis a param of the assembly runs along, and the faces it
    /// is measured between when it is one placement term's gap or inset;
    /// else the axis of a part size that follows it.
    fn measure<'t>(
        &self,
        assembly: AssemblyId,
        param: ParamId,
        parts: &[PartId],
        traced: &impl Fn(Quantity) -> Option<&'t Trace>,
    ) -> Option<(Axis, Option<(GripFace, GripFace)>)> {
        let source = Source::Param(crate::stops::original(self.model, param));
        let exactly = |trace: &Trace| match &trace.form {
            Form::Linear(linear) => {
                linear.terms.len() == 1
                    && linear.constant.raw() == Rational::ZERO
                    && linear.term(&source).map(|t| t.coefficient) == Some(Rational::ONE)
            }
            Form::NotLinear(_) => false,
        };
        for part in parts {
            for axis in [Axis::Across, Axis::Up, Axis::Deep] {
                let Some(trace) = traced(Quantity::PlaceOffset { part: *part, axis }) else {
                    continue;
                };
                if exactly(trace) {
                    let between = self.between(assembly, *part, axis, trace.value);
                    return Some((axis, between));
                }
            }
        }
        for part in parts {
            for axis in [PartAxis::Length, PartAxis::Width] {
                let Some(trace) = traced(Quantity::PartSize { part: *part, axis }) else {
                    continue;
                };
                let follows = match &trace.form {
                    Form::Linear(linear) => linear.term(&source).is_some(),
                    Form::NotLinear(not) => not.held.iter().any(|t| t.dependence.source == source),
                };
                if follows {
                    let world = self.design.part(*part)?.pose.orientation.direction(axis);
                    return Some((world.axis(), None));
                }
            }
        }
        None
    }

    /// The faces the offset of `part` along `axis` is measured between,
    /// from its `flush` or `against` term: the part's face that moves, and
    /// the face it is measured from, `value` away.
    fn between(
        &self,
        assembly: AssemblyId,
        part: PartId,
        axis: Axis,
        value: Amount,
    ) -> Option<(GripFace, GripFace)> {
        let model = self.model;
        let inst = model.parts.get(part.index())?;
        let PartKind::FromStock(body) = &model.defs.part(inst.def)?.kind else {
            return None;
        };
        let (face, moving) = body.place.iter().find_map(|term| {
            let (expr, flush) = match &term.kind {
                PlaceTermKind::Flush { face, .. } => (face, true),
                PlaceTermKind::Against { face, .. } => (face, false),
                _ => return None,
            };
            let face = face_of(expr)?;
            (face.axis() == axis).then_some((face, if flush { face } else { face.opposite() }))
        })?;
        let Amount::Length(offset) = value else {
            return None;
        };
        let offset = offset.to_length()?;
        let drawn = self.design.part(part)?.drawn_box();
        let to_at = drawn.face(moving);
        // A low face (left, bottom, front) moves away from a plane below it.
        let from_at = if moving.is_positive() {
            to_at.checked_add(offset)?
        } else {
            to_at.checked_sub(offset)?
        };
        let to = GripFace {
            of: FaceOf::Part(part),
            face: moving,
            at: to_at,
        };
        let [first, second] = axis.others();
        let overlaps = |bounds: &stave_model::solved::WorldBox| {
            [first, second].iter().all(|other| {
                bounds.min().get(*other) < drawn.max().get(*other)
                    && drawn.min().get(*other) < bounds.max().get(*other)
            })
        };
        let region = self
            .design
            .regions
            .iter()
            .filter(|r| r.assembly != Some(assembly))
            .find(|r| r.bounds.face(face) == from_at && overlaps(&r.bounds))
            .map(|r| FaceOf::Region(r.id));
        let owner = region.or_else(|| {
            self.design
                .parts
                .iter()
                .filter(|p| p.assembly != Some(assembly))
                .find(|p| p.drawn_box().face(face) == from_at && overlaps(&p.drawn_box()))
                .map(|p| FaceOf::Part(p.id))
        })?;
        Some((
            GripFace {
                of: owner,
                face,
                at: from_at,
            },
            to,
        ))
    }

    /// A movable grip for the plain number at `path`.
    fn movable(&self, path: &str, dependence: &Dependence) -> Result<MovableGrip, GripsError> {
        let value = dependence.value;
        let resolution = self.design.project.resolution.to_exact();
        let step = step_for(value, resolution);
        let reach = match value {
            Amount::Length(_) => self.options.reach.to_exact().value(),
            Amount::Angle(_) | Amount::Count(_) | Amount::Ratio(_) => step
                .raw()
                .checked_mul(Rational::from_int(100))
                .unwrap_or(step.raw()),
        };
        let limit = Bounds {
            low: value.with_raw(value.raw().checked_sub(reach).unwrap_or(value.raw())),
            high: value.with_raw(value.raw().checked_add(reach).unwrap_or(value.raw())),
        };
        let steps = match &dependence.domain {
            Some(ParamDomain::OneOf(values)) => {
                let mut values = values.clone();
                values.sort_by_key(|v| v.raw());
                values.dedup();
                SearchSteps::List(values)
            }
            Some(ParamDomain::Range { .. }) | None => SearchSteps::Step(step),
        };
        let (mut low, mut high) = match &dependence.domain {
            Some(ParamDomain::Range { low, high }) => (*low, *high),
            Some(ParamDomain::OneOf(values)) => (
                values
                    .iter()
                    .copied()
                    .min_by_key(|v| v.raw())
                    .unwrap_or(value),
                values
                    .iter()
                    .copied()
                    .max_by_key(|v| v.raw())
                    .unwrap_or(value),
            ),
            None => (limit.low, limit.high),
        };
        let stops = if self.options.search_stops {
            let found = find_stops(self.base, StopsRequest { path, limit }, self.options.cancel)?;
            low = found.below.last_good;
            high = found.above.last_good;
            Some((found.below, found.above))
        } else {
            None
        };
        Ok(MovableGrip {
            value,
            low,
            high,
            steps,
            freedom: dependence.freedom,
            stops,
        })
    }

    fn envelope(&self) -> Result<Vec<SelectionGrip>, GripsError> {
        let root = self.design.root_box().ok_or(GripsError::NoEnvelope)?;
        let axes = [Axis::Across, Axis::Up, Axis::Deep];
        let quantities: Vec<Quantity> = axes
            .iter()
            .map(|axis| Quantity::RegionSize {
                region: root.id,
                axis: *axis,
            })
            .collect();
        let traces = stave_solve::trace::trace_all(self.model, &quantities);
        let mut out = Vec::new();
        for (axis, trace) in axes.iter().zip(traces) {
            let Ok(trace) = trace else {
                continue;
            };
            let face = match axis {
                Axis::Across => Face::Right,
                Axis::Up => Face::Top,
                Axis::Deep => Face::Back,
            };
            let place = Some(PlaceInput::Envelope {
                region: root.id,
                face,
            });
            // One param, written as a plain number, is the size.
            let single = match &trace.form {
                Form::Linear(linear)
                    if linear.terms.len() == 1 && linear.constant.raw() == Rational::ZERO =>
                {
                    linear
                        .terms
                        .first()
                        .filter(|t| t.coefficient == Rational::ONE)
                }
                _ => None,
            };
            let param = single.and_then(|term| match term.dependence.source {
                Source::Param(param)
                    if written_number(self.model, param).is_some()
                        && term.dependence.freedom != Freedom::Locked =>
                {
                    Some((param, term))
                }
                _ => None,
            });
            let path = param.and_then(|(param, _)| path_of_param(self.model, param));
            match (param, path) {
                (Some((param, term)), Some(path)) => out.push(SelectionGrip {
                    size: GripSize::Param {
                        param,
                        path: Some(path.clone()),
                    },
                    label: path.clone(),
                    axis: Some(*axis),
                    action: GripAction::Moves(Box::new(self.movable(&path, &term.dependence)?)),
                    place,
                }),
                _ => out.push(SelectionGrip {
                    size: GripSize::RegionSize {
                        region: root.id,
                        axis: *axis,
                    },
                    label: format!("{}.{}", root.name, axis.word()),
                    axis: Some(*axis),
                    action: GripAction::Follows(Box::new(follows(self.model, &trace))),
                    place,
                }),
            }
        }
        Ok(out)
    }

    fn split(&self, region: RegionId) -> Result<Vec<SelectionGrip>, GripsError> {
        let model = self.model;
        let inst = model
            .regions
            .get(region.index())
            .ok_or(GripsError::NotInDesign)?;
        let def = model.defs.region(inst.def).ok_or(GripsError::NotInDesign)?;
        let is_split = matches!(
            def.value.kind,
            RExprKind::Builtin {
                function: Builtin::Split,
                ..
            }
        );
        if !is_split || def.array.is_none() {
            return Err(GripsError::NotASplit);
        }
        let name = def.decl.name.clone();
        let pieces: Vec<RegionId> = model
            .regions
            .iter()
            .filter(|r| r.def == inst.def && r.ctx.instance == inst.ctx.instance)
            .map(|r| r.id)
            .collect();
        let bounds = |id: RegionId| {
            self.design
                .regions
                .iter()
                .find(|r| r.id == id)
                .map(|r| (r.bounds, r.name.clone()))
        };
        let resolution = self.design.project.resolution;
        let mut out = Vec::new();
        for (lower, pair) in pieces.windows(2).enumerate() {
            let [low, high] = pair else {
                continue;
            };
            let (Some((low_box, low_name)), Some((high_box, high_name))) =
                (bounds(*low), bounds(*high))
            else {
                continue;
            };
            let Some(axis) = [Axis::Across, Axis::Up, Axis::Deep]
                .into_iter()
                .find(|axis| low_box.max().get(*axis) == high_box.min().get(*axis))
            else {
                continue;
            };
            let size = low_box.size(axis);
            let pair_size = size.checked_add(high_box.size(axis)).unwrap_or(size);
            let value = Amount::Length(size.to_exact());
            let low_end = Amount::Length(resolution.to_exact());
            let high_end = Amount::Length(
                pair_size
                    .checked_sub(resolution)
                    .unwrap_or(pair_size)
                    .to_exact(),
            );
            out.push(SelectionGrip {
                size: GripSize::Boundary {
                    region: name.clone(),
                    lower,
                    low: *low,
                    high: *high,
                },
                label: format!("{low_name} | {high_name}"),
                axis: Some(axis),
                action: GripAction::Moves(Box::new(MovableGrip {
                    value,
                    low: low_end,
                    high: high_end,
                    steps: SearchSteps::Step(Amount::Length(resolution.to_exact())),
                    freedom: Freedom::Plain,
                    stops: None,
                })),
                place: Some(PlaceInput::Share {
                    low: *low,
                    high: *high,
                    axis,
                }),
            });
        }
        Ok(out)
    }
}

/// The face a `flush` or `against` term names: `space.bottom`.
fn face_of(expr: &RExpr) -> Option<Face> {
    match &expr.kind {
        RExprKind::Member {
            member: Member::Face(face),
            ..
        } => Some(*face),
        _ => None,
    }
}

/// A param's declared name: `height`.
fn param_name(model: &Model, param: ParamId) -> Option<String> {
    let inst = model.params.get(param.index())?;
    Some(model.defs.param(inst.def)?.decl.name.clone())
}

/// A param's name with its instance: `drawer[top].top_gap`.
fn full_name(model: &Model, param: ParamId) -> Option<String> {
    Some(model.params.get(param.index())?.name.clone())
}

/// What a traced size follows.
fn follows(model: &Model, trace: &Trace) -> FollowsGrip {
    let mut list = Vec::new();
    match &trace.form {
        Form::Linear(linear) => {
            for term in &linear.terms {
                list.push(followed(model, &term.dependence, Some(term.coefficient)));
            }
        }
        Form::NotLinear(not) => {
            for term in &not.held {
                list.push(followed(model, &term.dependence, Some(term.coefficient)));
            }
            for dependence in &not.dependences {
                list.push(followed(model, dependence, None));
            }
        }
    }
    FollowsGrip {
        value: trace.value,
        trace: trace.clone(),
        follows: list,
    }
}

fn followed(
    model: &Model,
    dependence: &Dependence,
    coefficient: Option<Rational>,
) -> FollowedValue {
    let (name, path) = match &dependence.source {
        Source::Param(param) => (
            full_name(model, *param).unwrap_or_default(),
            path_of_param(model, *param).filter(|_| written_number(model, *param).is_some()),
        ),
        Source::StockField { stock, field } => (
            format!(
                "{}.{}",
                model
                    .defs
                    .stock(*stock)
                    .map(|s| s.decl.name.as_str())
                    .unwrap_or("stock"),
                field_word(*field)
            ),
            None,
        ),
        Source::KindSize { kind, name } => (
            format!(
                "{}.{name}",
                model
                    .defs
                    .kind(*kind)
                    .map(|k| k.decl.name.as_str())
                    .unwrap_or("kind")
            ),
            None,
        ),
    };
    FollowedValue {
        source: dependence.source.clone(),
        name,
        path,
        kind: FollowKind::of(dependence.freedom),
        coefficient,
        value: dependence.value,
    }
}

fn field_word(field: StockField) -> &'static str {
    match field {
        StockField::Species => "species",
        StockField::Form => "form",
        StockField::Thickness => "thickness",
        StockField::Diameter => "diameter",
        StockField::Width => "width",
        StockField::MaxWidth => "max_width",
        StockField::MaxLength => "max_length",
    }
}

// ---------------------------------------------------------------------------
// The contract
// ---------------------------------------------------------------------------

impl SelectionGrip {
    /// The contract's [`Grip`] for this grip, when it can say it: `None`
    /// for a part's or a region's size (no [`GripTarget`] for them), a
    /// param no patch can address, a grip with no axis, or a follows grip
    /// that follows nothing. A follows grip's names are the last part of
    /// each name (`top_gap` for `drawer[top].top_gap`): an [`Ident`] has
    /// no dots.
    pub fn contract(&self, model: &Model) -> Option<Grip> {
        let target = match &self.size {
            GripSize::Param { path, .. } => {
                let path = path.as_deref()?;
                match path.split_once('.') {
                    None => GripTarget::Param(ident(path)?),
                    Some((assembly, param)) => {
                        let found = model.assemblies.iter().find(|inst| {
                            inst.parent.is_none()
                                && model
                                    .defs
                                    .assembly(inst.def)
                                    .is_some_and(|d| d.decl.name == assembly)
                        })?;
                        GripTarget::AssemblyArgument {
                            assembly: found.id,
                            param: ident(param)?,
                        }
                    }
                }
            }
            GripSize::Boundary { region, lower, .. } => GripTarget::Shares {
                region: ident(region)?,
                lower: *lower,
            },
            GripSize::PartSize { .. } | GripSize::RegionSize { .. } => return None,
        };
        let handle = match &self.action {
            GripAction::Moves(movable) => Handle::Moves(movable.contract()?),
            GripAction::Follows(follows) => {
                let last = |name: &str| {
                    let tail = name.rsplit('.').next().unwrap_or(name);
                    ident(tail.split('[').next().unwrap_or(tail))
                };
                let names: Vec<Ident> = follows
                    .follows
                    .iter()
                    .filter_map(|f| last(&f.name))
                    .collect();
                let plain = follows
                    .follows
                    .iter()
                    .filter(|f| f.kind.is_candidate() && f.coefficient.is_some())
                    .filter_map(|f| last(&f.name))
                    .collect();
                Handle::Follows(Follows {
                    names: NonEmpty::new(names)?,
                    plain,
                })
            }
        };
        Some(Grip {
            target,
            axis: self.axis?,
            handle,
        })
    }
}

impl MovableGrip {
    fn contract(&self) -> Option<Movable> {
        fn stops<T: Ord + Copy>(
            steps: &SearchSteps,
            convert: impl Fn(Amount) -> Option<T>,
        ) -> Option<Stops<T>> {
            Some(match steps {
                SearchSteps::Step(step) => Stops::Step(convert(*step)?),
                SearchSteps::List(values) => Stops::List(NonEmpty::new(
                    values
                        .iter()
                        .map(|v| convert(*v))
                        .collect::<Option<Vec<T>>>()?,
                )?),
            })
        }
        match self.value {
            Amount::Length(_) => {
                let convert = |a: Amount| match a {
                    Amount::Length(l) => l.to_length(),
                    _ => None,
                };
                Some(Movable::Length(
                    GripRange::new(
                        convert(self.value)?,
                        convert(self.low)?,
                        convert(self.high)?,
                        stops(&self.steps, convert)?,
                    )
                    .ok()?,
                ))
            }
            Amount::Angle(_) => {
                let convert = |a: Amount| match a {
                    Amount::Angle(angle) => angle.to_angle(),
                    _ => None,
                };
                let range: GripRange<Angle> = GripRange::new(
                    convert(self.value)?,
                    convert(self.low)?,
                    convert(self.high)?,
                    stops(&self.steps, convert)?,
                )
                .ok()?;
                Some(Movable::Angle(range))
            }
            Amount::Count(_) | Amount::Ratio(_) => {
                let convert = |a: Amount| Some(a.raw());
                Some(Movable::Number(
                    GripRange::new(
                        self.value.raw(),
                        self.low.raw(),
                        self.high.raw(),
                        stops(&self.steps, convert)?,
                    )
                    .ok()?,
                ))
            }
        }
    }
}

fn ident(name: &str) -> Option<Ident> {
    Ident::new(name, Span::default()).ok()
}

// ---------------------------------------------------------------------------
// Moving
// ---------------------------------------------------------------------------

/// What a drag of a grip writes.
#[derive(Debug, Clone)]
pub enum GripMove {
    /// One patch.
    Set(Box<GripPatch>),
    /// Several plain params could give the size: the user picks one. Each
    /// with its patch, or why it can't give that size.
    Choose(Vec<Choice>),
}

/// A patch a drag writes, with the value it changes.
#[derive(Debug, Clone)]
pub struct GripPatch {
    /// The patch: `set divider.height = 400mm`, or a split's region
    /// declaration again for a boundary.
    pub patch: GeneratedPatch,
    /// The path set; the region's name for a boundary.
    pub path: String,
    /// The value before.
    pub before: Amount,
    /// The value after.
    pub after: Amount,
}

/// One plain param the user may pick for a follows grip.
#[derive(Debug, Clone)]
pub struct Choice {
    /// The param, as listed in the grip.
    pub followed: FollowedValue,
    /// Its patch, or why it can't give the size.
    pub result: Result<GripPatch, MoveError>,
}

/// Why a drag writes nothing.
#[derive(Debug, Clone, thiserror::Error)]
pub enum MoveError {
    /// No value the size follows can change: each is locked, a stock's or a
    /// kind's size, or not linear. Lists what it follows.
    #[error("{}", stopped_message(.0))]
    Stopped(Vec<FollowedValue>),
    /// The value is a param no patch can address.
    #[error("'{0}' can't be changed by a patch: it is set inside another component")]
    NotWritable(String),
    /// The target is not the kind of value the grip moves.
    #[error("the value is not the same kind as the size (a length for a length)")]
    Mismatch,
    /// The inversion has no answer.
    #[error(transparent)]
    Invert(#[from] InvertError),
    /// A boundary can't be moved there.
    #[error(transparent)]
    Shares(#[from] RepeatError),
    /// The patch did not parse: a bug in the writer.
    #[error(transparent)]
    Patch(#[from] PatchBuildError),
    /// The param is not one this grip follows.
    #[error("that value is not one this size follows")]
    NotFollowed,
    /// The numbers are too large.
    #[error("the numbers are too large")]
    Overflow,
    /// The value the drag needs has no plain-number text: a count or ratio
    /// with no finite decimal (a third), which `.stave` can only write as
    /// a division. A drag writes plain numbers, so it is refused.
    #[error(
        "'{path}' would have to be {numerator}/{denominator}, which has no exact decimal, so a drag can't write it"
    )]
    NotPlain {
        /// The path of the param.
        path: String,
        /// The value needed, above the line.
        numerator: i128,
        /// The value needed, below the line.
        denominator: i128,
    },
}

fn stopped_message(follows: &[FollowedValue]) -> String {
    if follows.is_empty() {
        return "this size is worked out from numbers only, so a drag has nothing to change"
            .to_owned();
    }
    let reasons: Vec<String> = follows
        .iter()
        .map(|f| match (f.kind, f.coefficient) {
            (FollowKind::Plain | FollowKind::Solved, None) => {
                format!(
                    "{} (the size is not worked out from it in a straight line)",
                    f.name
                )
            }
            (kind, _) => format!("{} ({})", f.name, kind.words()),
        })
        .collect();
    format!(
        "nothing this size follows can change: {}",
        reasons.join(", ")
    )
}

/// The patch for dragging `grip` to `to`.
///
/// - A movable grip: `set <path> = <value>`, the value snapped to the
///   project's `resolution:` (or to the nearest value of a `within` list)
///   and kept within the grip's range. A boundary: the split's shares
///   moved ([`move_shares`]).
/// - A follows grip with exactly one plain (or solved) param it is linear
///   in: the single-param inversion on that param, exact, not snapped.
///   With several, [`GripMove::Choose`]; with none, [`MoveError::Stopped`].
///
/// # Errors
///
/// See [`MoveError`].
pub fn move_grip(
    scope: &EditScope<'_>,
    design: &SolvedDesign,
    grip: &SelectionGrip,
    to: Amount,
) -> Result<GripMove, MoveError> {
    match &grip.action {
        GripAction::Moves(movable) => move_movable(scope, design, grip, movable, to)
            .map(|patch| GripMove::Set(Box::new(patch))),
        GripAction::Follows(follows) => {
            let candidates: Vec<&FollowedValue> = follows
                .follows
                .iter()
                .filter(|f| f.kind.is_candidate() && f.coefficient.is_some())
                .collect();
            match candidates.as_slice() {
                [] => Err(MoveError::Stopped(follows.follows.clone())),
                [one] => invert_one(scope.model(), follows, one, to)
                    .map(|patch| GripMove::Set(Box::new(patch))),
                several => Ok(GripMove::Choose(
                    several
                        .iter()
                        .map(|f| Choice {
                            followed: (*f).clone(),
                            result: invert_one(scope.model(), follows, f, to),
                        })
                        .collect(),
                )),
            }
        }
    }
}

/// The patch for dragging a follows grip to `to` by changing the param the
/// user picked from [`GripMove::Choose`].
///
/// # Errors
///
/// [`MoveError::NotFollowed`] for a param the grip doesn't follow or a
/// movable grip; otherwise as [`move_grip`].
pub fn move_grip_by(
    model: &Model,
    grip: &SelectionGrip,
    param: ParamId,
    to: Amount,
) -> Result<GripPatch, MoveError> {
    let GripAction::Follows(follows) = &grip.action else {
        return Err(MoveError::NotFollowed);
    };
    let followed = follows
        .follows
        .iter()
        .find(|f| f.source == Source::Param(param))
        .ok_or(MoveError::NotFollowed)?;
    invert_one(model, follows, followed, to)
}

fn invert_one(
    model: &Model,
    follows: &FollowsGrip,
    followed: &FollowedValue,
    to: Amount,
) -> Result<GripPatch, MoveError> {
    let Source::Param(param) = followed.source else {
        return Err(MoveError::Stopped(vec![followed.clone()]));
    };
    let after = stave_model::trace::invert(&follows.trace, param, to)?;
    let path = followed
        .path
        .clone()
        .or_else(|| path_of_param(model, param).filter(|_| written_number(model, param).is_some()))
        .ok_or_else(|| MoveError::NotWritable(followed.name.clone()))?;
    set_patch(&path, followed.value, after)
}

fn set_patch(path: &str, before: Amount, after: Amount) -> Result<GripPatch, MoveError> {
    let text = amount_text(after).ok_or_else(|| match after {
        Amount::Count(n) | Amount::Ratio(n) if crate::stops::decimal_places(n).is_none() => {
            MoveError::NotPlain {
                path: path.to_owned(),
                numerator: n.numerator(),
                denominator: n.denominator(),
            }
        }
        _ => MoveError::Overflow,
    })?;
    let patch = GeneratedPatch::new(
        &format!("Set {path} to {text}"),
        &[format!("set {path} = {text}")],
    )?;
    Ok(GripPatch {
        patch,
        path: path.to_owned(),
        before,
        after,
    })
}

fn move_movable(
    scope: &EditScope<'_>,
    design: &SolvedDesign,
    grip: &SelectionGrip,
    movable: &MovableGrip,
    to: Amount,
) -> Result<GripPatch, MoveError> {
    if !to.same_dimension(movable.value) {
        return Err(MoveError::Mismatch);
    }
    let snapped = match &movable.steps {
        SearchSteps::Step(step) => snap(to, *step).ok_or(MoveError::Overflow)?,
        SearchSteps::List(values) => values
            .iter()
            .copied()
            .min_by_key(|v| {
                v.raw()
                    .checked_sub(to.raw())
                    .map(|d| {
                        if d.is_negative() {
                            d.checked_neg().unwrap_or(d)
                        } else {
                            d
                        }
                    })
                    .unwrap_or(Rational::ZERO)
            })
            .unwrap_or(movable.value),
    };
    let clamped = if snapped.raw() < movable.low.raw() {
        movable.low
    } else if snapped.raw() > movable.high.raw() {
        movable.high
    } else {
        snapped
    };
    match &grip.size {
        GripSize::Param { path, .. } => {
            let path = path
                .as_deref()
                .ok_or_else(|| MoveError::NotWritable(grip.label.clone()))?;
            set_patch(path, movable.value, clamped)
        }
        GripSize::Boundary { region, lower, .. } => {
            let Amount::Length(boundary) = clamped else {
                return Err(MoveError::Mismatch);
            };
            let boundary = boundary.to_nearest_length().ok_or(MoveError::Overflow)?;
            let patch = move_shares(
                scope,
                design,
                SharesMove {
                    region,
                    lower: *lower,
                    boundary,
                },
            )?;
            Ok(GripPatch {
                patch,
                path: region.clone(),
                before: movable.value,
                after: clamped,
            })
        }
        GripSize::PartSize { .. } | GripSize::RegionSize { .. } => {
            Err(MoveError::NotWritable(grip.label.clone()))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::AtomicBool;

    use stave_lang::Code;
    use stave_lang::units::ExactLength;
    use stave_modules::MemoryFiles;

    use super::*;
    use crate::stops::SideEnd;

    /// Where the tests' copy of the cabinet lives in memory.
    const FOLDER: &str = "scratch/clothing_cabinet_from_box";

    /// A copy of `examples/clothing_cabinet_from_box`, in memory.
    fn cabinet() -> MemoryFiles {
        let example =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/clothing_cabinet_from_box");
        let mut files = MemoryFiles::new();
        for entry in std::fs::read_dir(&example).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "stave") {
                let name = path.file_name().unwrap().to_owned();
                files.insert(
                    PathBuf::from(FOLDER).join(name),
                    std::fs::read_to_string(&path).unwrap(),
                );
            }
        }
        files
    }

    fn mm(n: i64) -> Amount {
        Amount::Length(ExactLength::new(Rational::from_int(
            i128::from(n) * 1_000_000,
        )))
    }

    fn options(cancel: &AtomicBool, search_stops: bool) -> GripOptions<'_> {
        GripOptions {
            search_stops,
            reach: Length::from_mm(1000),
            cancel,
        }
    }

    fn assembly(base: &EditBase<'_>, name: &str) -> AssemblyId {
        let model = base.opened().model.as_ref().unwrap();
        model.assemblies.iter().find(|a| a.name == name).unwrap().id
    }

    fn grip<'g>(grips: &'g [SelectionGrip], label: &str) -> &'g SelectionGrip {
        grips
            .iter()
            .find(|g| g.label == label)
            .unwrap_or_else(|| panic!("no grip {label}"))
    }

    fn movable(grip: &SelectionGrip) -> &MovableGrip {
        match &grip.action {
            GripAction::Moves(movable) => movable,
            GripAction::Follows(_) => panic!("{} follows", grip.label),
        }
    }

    fn following(grip: &SelectionGrip) -> &FollowsGrip {
        match &grip.action {
            GripAction::Follows(follows) => follows,
            GripAction::Moves(_) => panic!("{} moves", grip.label),
        }
    }

    fn tried(base: &EditBase<'_>, patch: &GripPatch) -> crate::scratch::Tried {
        base.try_patch("grip.stave", &patch.patch.parse, &AtomicBool::new(false))
            .unwrap()
    }

    /// The divider's height is a plain number: a movable grip whose lower
    /// stop is where the drawers no longer fit. The space under the
    /// divider is `height` tall and split into two drawer openings, each
    /// `height / 2`; a drawer box is its opening less 15mm under and 15mm
    /// over it, and `box_is_useful` wants it at least 60mm. So
    /// `height / 2 - 30mm >= 60mm`: 180mm is the last good height, and
    /// 179.5mm (one step of the 0.5mm resolution lower) fails.
    #[test]
    fn the_divider_height_stops_where_the_drawers_no_longer_fit() {
        let files = cabinet();
        let base = EditBase::new(Path::new(FOLDER), &files).unwrap();
        let cancel = AtomicBool::new(false);
        let divider = assembly(&base, "divider");
        let grips = grips(
            &base,
            GripSelection::Assembly(divider),
            options(&cancel, true),
        )
        .unwrap();
        let height = grip(&grips, "divider.height");
        assert_eq!(height.axis, Some(Axis::Up));
        let range = movable(height);
        assert_eq!(range.value, mm(500));
        assert_eq!(range.low, mm(180));
        assert_eq!(range.freedom, Freedom::Plain);
        let (below, _above) = range.stops.as_ref().unwrap();
        let SideEnd::Stopped { at, diagnostic } = &below.end else {
            panic!("the height has a lower stop");
        };
        assert_eq!(
            *at,
            Amount::Length(ExactLength::new(Rational::from_int(179_500_000)))
        );
        assert_eq!(diagnostic.code, Code::Require);
        assert!(
            diagnostic.message.contains("box_is_useful"),
            "{}",
            diagnostic.message
        );
        // The handle sits on the divider's underside, measured from the
        // top of the case's base (80mm raise + 19mm base).
        let Some(PlaceInput::Length {
            param,
            lengths,
            between: Some((from, to)),
            ..
        }) = &height.place
        else {
            panic!(
                "the height is a length between two faces: {:?}",
                height.place
            );
        };
        assert_eq!(param, "height");
        assert!(lengths.contains(&("height".to_owned(), Length::from_mm(500))));
        assert_eq!((from.face, from.at), (Face::Bottom, Length::from_mm(99)));
        assert_eq!((to.face, to.at), (Face::Bottom, Length::from_mm(599)));
        // The contract can carry it.
        let model = base.opened().model.as_ref().unwrap();
        let contract = height.contract(model).unwrap();
        assert!(matches!(
            contract.target,
            GripTarget::AssemblyArgument { assembly, .. } if assembly == divider
        ));
    }

    /// The envelope's width stops where the two front feet meet. The base
    /// is the width less the top's 20mm overhang at each end; the feet
    /// stand 60mm in from each end of it and are 70mm across (the bun
    /// foot's widest station). They touch when `width - 40mm = 2 × 60mm +
    /// 2 × 70mm`, at 300mm; at 299.5mm they overlap.
    #[test]
    fn the_envelope_width_stops_where_the_feet_meet() {
        let files = cabinet();
        let base = EditBase::new(Path::new(FOLDER), &files).unwrap();
        let cancel = AtomicBool::new(false);
        let grips = grips(&base, GripSelection::Envelope, options(&cancel, true)).unwrap();
        let labels: Vec<&str> = grips.iter().map(|g| g.label.as_str()).collect();
        assert_eq!(labels, ["overall_width", "overall_height", "overall_depth"]);
        let width = movable(grip(&grips, "overall_width"));
        assert_eq!(width.low, mm(300));
        let (below, above) = width.stops.as_ref().unwrap();
        let SideEnd::Stopped { diagnostic, .. } = &below.end else {
            panic!("the width has a lower stop");
        };
        assert_eq!(diagnostic.code, Code::Overlap);
        assert!(
            diagnostic.message.contains("feet.foot"),
            "{}",
            diagnostic.message
        );
        // No `within`: the search went as far as the reach above.
        assert_eq!(above.last_good, mm(1940));
        assert_eq!(above.end, SideEnd::Limit);
        assert!(matches!(
            grip(&grips, "overall_width").place,
            Some(PlaceInput::Envelope {
                face: Face::Right,
                ..
            })
        ));
    }

    /// Dragging the divider writes a plain number, snapped to the 0.5mm
    /// resolution, that applies and checks.
    #[test]
    fn dragging_the_divider_gives_a_patch_that_checks() {
        let files = cabinet();
        let base = EditBase::new(Path::new(FOLDER), &files).unwrap();
        let cancel = AtomicBool::new(false);
        let grips = grips(
            &base,
            GripSelection::Assembly(assembly(&base, "divider")),
            options(&cancel, false),
        )
        .unwrap();
        let opened = base.opened();
        let scope = EditScope::new(opened).unwrap();
        let design = opened.design.as_ref().unwrap();
        let target = Amount::Length(ExactLength::new(Rational::from_int(400_200_000)));
        let GripMove::Set(patch) =
            move_grip(&scope, design, grip(&grips, "divider.height"), target).unwrap()
        else {
            panic!("a movable grip writes one patch");
        };
        assert!(
            patch.patch.text.contains("set divider.height = 400mm"),
            "{}",
            patch.patch.text
        );
        assert_eq!((patch.before, patch.after), (mm(500), mm(400)));
        let tried = tried(&base, &patch);
        assert!(tried.fits(), "{:?}", tried.shown());
        let panel = tried
            .opened()
            .and_then(|o| o.design.as_ref())
            .and_then(|d| d.part_named("divider.panel"))
            .unwrap();
        assert_eq!(panel.drawn_box().face(Face::Bottom), Length::from_mm(499));
    }

    /// A drawer's box width (its sub-front's length, between the sides)
    /// follows `overall_width`; it also follows the case's overhang, so a
    /// drag offers both, and inverting it through `overall_width` gives a
    /// box exactly as wide as asked.
    #[test]
    fn a_drawer_box_width_follows_the_overall_width_and_inverts_exactly() {
        let files = cabinet();
        let base = EditBase::new(Path::new(FOLDER), &files).unwrap();
        let cancel = AtomicBool::new(false);
        let grips = grips(
            &base,
            GripSelection::Assembly(assembly(&base, "drawer[bottom]")),
            options(&cancel, false),
        )
        .unwrap();
        let width = grip(&grips, "drawer[bottom].sub_front.length");
        assert_eq!(width.axis, Some(Axis::Across));
        let follows = following(width);
        let kinds: Vec<(&str, FollowKind)> = follows
            .follows
            .iter()
            .map(|f| (f.name.as_str(), f.kind))
            .collect();
        assert_eq!(
            kinds,
            [
                ("overall_width", FollowKind::Plain),
                ("case.overhang", FollowKind::Plain),
                ("oak_19.thickness", FollowKind::StockSize),
                ("maple_12.thickness", FollowKind::StockSize),
                ("side_mount_slide.clearance", FollowKind::KindSize),
            ]
        );
        let opened = base.opened();
        let model = opened.model.as_ref().unwrap();
        let scope = EditScope::new(opened).unwrap();
        let design = opened.design.as_ref().unwrap();
        let before = follows.value;
        let target = before.with_raw(before.raw().checked_add(mm(50).raw()).unwrap());
        let GripMove::Choose(choices) = move_grip(&scope, design, width, target).unwrap() else {
            panic!("two plain params: the user picks");
        };
        let names: Vec<&str> = choices.iter().map(|c| c.followed.name.as_str()).collect();
        assert_eq!(names, ["overall_width", "case.overhang"]);
        let overall_width = crate::stops::param_at_path(model, "overall_width").unwrap();
        let patch = move_grip_by(model, width, overall_width, target).unwrap();
        assert_eq!((patch.before, patch.after), (mm(940), mm(990)));
        assert!(patch.patch.text.contains("set overall_width = 990mm"));
        let tried = tried(&base, &patch);
        assert!(tried.fits(), "{:?}", tried.shown());
        // Solved again: the box is exactly that wide.
        let sub_front = tried
            .opened()
            .and_then(|o| o.design.as_ref())
            .and_then(|d| d.part_named("drawer[bottom].sub_front"))
            .unwrap();
        assert_eq!(
            Amount::Length(sub_front.drawn.get(PartAxis::Length).to_exact()),
            target
        );
    }

    /// A size with one plain param inverts at once; a size that follows
    /// only what stops the chain is refused, saying what stops it.
    #[test]
    fn one_candidate_inverts_and_none_is_refused() {
        let files = cabinet();
        let base = EditBase::new(Path::new(FOLDER), &files).unwrap();
        let cancel = AtomicBool::new(false);
        let grips = grips(
            &base,
            GripSelection::Assembly(assembly(&base, "drawer[bottom]")),
            options(&cancel, false),
        )
        .unwrap();
        let opened = base.opened();
        let scope = EditScope::new(opened).unwrap();
        let design = opened.design.as_ref().unwrap();
        // The front's height is half the divider's height.
        let front = grip(&grips, "drawer[bottom].front.width");
        let GripMove::Set(patch) = move_grip(&scope, design, front, mm(300)).unwrap() else {
            panic!("one plain param");
        };
        assert_eq!(patch.path, "divider.height");
        assert_eq!(patch.before, mm(500));
        let tried = tried(&base, &patch);
        assert!(tried.fits(), "{:?}", tried.shown());
        // The bottom's width follows the box stock's thickness only.
        let bottom = grip(&grips, "drawer[bottom].bottom.width");
        let error = move_grip(&scope, design, bottom, mm(400)).unwrap_err();
        let MoveError::Stopped(stops) = &error else {
            panic!("{error}");
        };
        assert_eq!(stops.first().map(|f| f.kind), Some(FollowKind::StockSize));
        assert!(
            error
                .to_string()
                .contains("maple_12.thickness (a size of the stock)"),
            "{error}"
        );
        // The drawer's own defaults are movable grips; its box height
        // follows the divider and its own gaps.
        assert!(matches!(
            grip(&grips, "drawer.top_gap").action,
            GripAction::Moves(_)
        ));
        let box_height = following(grip(&grips, "drawer.box_height"));
        let names: Vec<&str> = box_height.follows.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "divider.height",
                "drawer[bottom].bottom_gap",
                "drawer[bottom].top_gap"
            ]
        );
    }

    /// The boundary between the two drawer openings is a grip; moving it
    /// writes shares that check.
    #[test]
    fn the_boundary_between_the_drawers_moves_their_shares() {
        let files = cabinet();
        let base = EditBase::new(Path::new(FOLDER), &files).unwrap();
        let cancel = AtomicBool::new(false);
        let opened = base.opened();
        let model = opened.model.as_ref().unwrap();
        let piece = model
            .regions
            .iter()
            .find(|r| r.name == "drawer_bay[bottom]")
            .unwrap()
            .id;
        let grips = grips(&base, GripSelection::Split(piece), options(&cancel, false)).unwrap();
        let [boundary] = grips.as_slice() else {
            panic!("two pieces, one boundary: {grips:?}");
        };
        assert_eq!(boundary.axis, Some(Axis::Up));
        assert_eq!(movable(boundary).value, mm(250));
        let scope = EditScope::new(opened).unwrap();
        let design = opened.design.as_ref().unwrap();
        let GripMove::Set(patch) = move_grip(&scope, design, boundary, mm(200)).unwrap() else {
            panic!("a boundary writes one patch");
        };
        assert!(
            patch.patch.text.contains("shares: [2, 3]"),
            "{}",
            patch.patch.text
        );
        assert!(tried(&base, &patch).fits());
        let contract = boundary.contract(model).unwrap();
        assert!(matches!(
            contract.target,
            GripTarget::Shares { lower: 0, .. }
        ));
    }

    /// The cabinet with a slat whose length is a ratio of the space above
    /// the shelf, written as `share: <share>`.
    fn with_slat(share: &str) -> MemoryFiles {
        let mut files = cabinet();
        files.insert(
            PathBuf::from(FOLDER).join("slat.stave"),
            "stave 1\n\n\
             component Slat {\n\
             \x20 param space : Region\n\
             \x20 param slat_stock : Stock\n\
             \x20 param share = 0.5\n\n\
             \x20 part board : slat_stock {\n\
             \x20   length: space.across * share\n\
             \x20   width:  60mm\n\
             \x20   place {\n\
             \x20     against(space.bottom)\n\
             \x20     flush(space.left)\n\
             \x20     flush(space.front)\n\
             \x20   }\n\
             \x20 }\n\
             }\n"
            .to_owned(),
        );
        let cabinet = PathBuf::from(FOLDER).join("cabinet.stave");
        let mut text = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../examples/clothing_cabinet_from_box/cabinet.stave"),
        )
        .unwrap();
        text.push_str(&format!(
            "\nassembly slat : Slat {{\n  space: shelf.above\n  slat_stock: oak_19\n  share: {share}\n}}\n"
        ));
        files.insert(cabinet, text);
        files
    }

    /// A ratio param is dragged as a plain DECIMAL, so after the patch it
    /// is still a plain number and still a movable grip. A ratio with no
    /// finite decimal is refused rather than written as a division.
    #[test]
    fn a_ratio_grip_writes_a_decimal_and_stays_movable() {
        let ratio = |n, d| Amount::Ratio(Rational::new(n, d).unwrap());
        let files = with_slat("0.5");
        let base = EditBase::new(Path::new(FOLDER), &files).unwrap();
        let cancel = AtomicBool::new(false);
        let grips = grips(
            &base,
            GripSelection::Assembly(assembly(&base, "slat")),
            options(&cancel, false),
        )
        .unwrap();
        let share = grip(&grips, "slat.share");
        let range = movable(share);
        assert_eq!(range.value, ratio(1, 2));
        let opened = base.opened();
        let scope = EditScope::new(opened).unwrap();
        let design = opened.design.as_ref().unwrap();
        // 0.372 snaps to the hundredth.
        let GripMove::Set(patch) = move_grip(&scope, design, share, ratio(372, 1000)).unwrap()
        else {
            panic!("a movable grip writes one patch");
        };
        assert!(
            patch.patch.text.contains("set slat.share = 0.37"),
            "{}",
            patch.patch.text
        );
        assert!(!patch.patch.text.contains('/'), "{}", patch.patch.text);
        assert_eq!(patch.after, ratio(37, 100));
        let tried = tried(&base, &patch);
        assert!(tried.opened().and_then(|o| o.design.as_ref()).is_some());

        // The value as it is written after the commit: still movable.
        let files = with_slat("0.37");
        let base = EditBase::new(Path::new(FOLDER), &files).unwrap();
        let after = super::grips(
            &base,
            GripSelection::Assembly(assembly(&base, "slat")),
            options(&cancel, false),
        )
        .unwrap();
        assert_eq!(movable(grip(&after, "slat.share")).value, ratio(37, 100));

        // A third has no plain-number text.
        let refused = set_patch("slat.share", ratio(1, 2), ratio(1, 3));
        assert!(
            matches!(
                &refused,
                Err(MoveError::NotPlain {
                    numerator: 1,
                    denominator: 3,
                    ..
                })
            ),
            "{refused:?}"
        );
    }
}
