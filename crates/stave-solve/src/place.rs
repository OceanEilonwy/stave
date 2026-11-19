//! Placing parts and regions: orientation, drawn sizes (with `fit`), and
//! the position of each part along each world axis.
//!
//! Positions are the low end (left, bottom, front) of a part's **drawn**
//! box along each world axis, as exact fractions.

use stave_lang::ast::Span;
use stave_lang::placement::{Axis, Corner, Direction, Face, PartAxis};
use stave_lang::units::{ExactLength, Length, Rational};
use stave_lang::{Code, Diagnostic, FileId, Label};
use stave_model::defs::{
    DefId, PartBodyDef, PartKind, PlaceTerm, PlaceTermKind, RExpr, RExprKind, Size, StockField,
    Word,
};
use stave_model::ids::{AssemblyId, PartId, RegionId, StockId};
use stave_model::instance::ItemSlot;
use stave_model::solved::Orientation;
use stave_model::value::{
    AnchorOwner, AnchorValue, ExactBox, RegionOrigin, RegionValue, Spacing, Value,
};
use stave_schema::functions::Builtin;
use stave_schema::words::StockForm;

use crate::eval::{Env, midpoint};
use crate::{Accounted, Failure, Fault, Frame, Memo, Reported, Solved, Solver};

/// What is remembered about a part while it is placed.
#[derive(Debug, Clone, Default)]
pub(crate) struct PartState<'m> {
    pub orientation: Memo<Orientation>,
    pub sizes: [Memo<ExactLength>; 3],
    pub mins: [Memo<ExactLength>; 3],
    pub terms: Memo<Terms<'m>>,
    /// For an image, its source and the mirror plane; `None` for a part
    /// that is not one. Remembered once worked out: a failure is not.
    pub mirror: Option<Option<(PartId, Axis, ExactLength)>>,
}

fn axis_slot(axis: Axis) -> usize {
    match axis {
        Axis::Across => 0,
        Axis::Up => 1,
        Axis::Deep => 2,
    }
}

fn part_axis_slot(axis: PartAxis) -> usize {
    match axis {
        PartAxis::Length => 0,
        PartAxis::Width => 1,
        PartAxis::Thickness => 2,
    }
}

/// A placement term that fixes one axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AxisTermKind<'m> {
    /// `against(face, gap: d)`
    Against {
        /// The face.
        face: &'m RExpr,
        /// The gap.
        gap: Option<&'m RExpr>,
    },
    /// `flush(face, inset: d)`
    Flush {
        /// The face.
        face: &'m RExpr,
        /// The inset.
        inset: Option<&'m RExpr>,
    },
    /// `center(item, axis)`
    CenterOn {
        /// The part or region.
        item: &'m RExpr,
    },
    /// `center(from: a, to: b)`
    CenterBetween {
        /// One plane.
        from: &'m RExpr,
        /// The other.
        to: &'m RExpr,
    },
    /// `spaced(spacing, from: a, to: b)`
    Spaced {
        /// The spacing.
        spacing: &'m RExpr,
        /// The first plane.
        from: &'m RExpr,
        /// The last plane.
        to: &'m RExpr,
    },
    /// One axis of `corners(part, inset: d)`: flush with this face of the part.
    Corner {
        /// The part.
        part: &'m RExpr,
        /// The inset.
        inset: Option<&'m RExpr>,
        /// The face of the part on this axis.
        face: Face,
    },
}

/// A term with where it is written.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AxisTerm<'m> {
    /// The term.
    pub kind: AxisTermKind<'m>,
    /// Where it is written.
    pub span: Span,
}

/// A `place { }` block sorted by axis. It borrows the expressions from
/// the model, so it is cheap to copy.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Terms<'m> {
    /// `true` for the ground part.
    pub ground: bool,
    /// `orient(length:, width:)` and its span.
    pub orient: Option<(&'m RExpr, Option<&'m RExpr>, Span)>,
    /// The term fixing each world axis (across, up, deep).
    pub axes: [Option<AxisTerm<'m>>; 3],
}

impl<'m> Terms<'m> {
    /// The term fixing an axis.
    pub fn axis(&self, axis: Axis) -> Option<&AxisTerm<'m>> {
        self.axes.get(axis_slot(axis)).and_then(Option::as_ref)
    }
}

/// How spaced elements and corner elements find their place.
pub(crate) struct Siblings {
    /// This element's index.
    pub index: usize,
    /// Every element's size along the axis.
    pub sizes: Vec<ExactLength>,
    /// The part being placed (`None` for hardware).
    pub part: Option<PartId>,
}

pub(crate) fn add(a: ExactLength, b: ExactLength) -> Solved<ExactLength> {
    a.checked_add(b).ok_or(Failure::Fault(Fault::Overflow))
}

pub(crate) fn sub(a: ExactLength, b: ExactLength) -> Solved<ExactLength> {
    a.checked_sub(b).ok_or(Failure::Fault(Fault::Overflow))
}

pub(crate) fn scale(a: ExactLength, n: usize) -> Solved<ExactLength> {
    let n = i64::try_from(n).map_err(|_| Fault::Overflow)?;
    a.checked_mul(Rational::from(n))
        .ok_or(Failure::Fault(Fault::Overflow))
}

pub(crate) fn divide(a: ExactLength, n: usize) -> Solved<ExactLength> {
    let n = i64::try_from(n).map_err(|_| Fault::Overflow)?;
    a.checked_div(Rational::from(n))
        .ok_or(Failure::Fault(Fault::Overflow))
}

/// `true` for `box(across:, up:, deep:)`, the root region.
pub(crate) fn is_region_box(value: &RExpr) -> bool {
    matches!(
        value.kind,
        RExprKind::Builtin {
            function: Builtin::RegionBox,
            ..
        }
    )
}

/// Reflects an extent through a plane: `low..high` becomes
/// `2p - high .. 2p - low`.
pub(crate) fn reflect(
    low: ExactLength,
    high: ExactLength,
    plane: ExactLength,
) -> Solved<(ExactLength, ExactLength)> {
    let twice = add(plane, plane)?;
    Ok((sub(twice, high)?, sub(twice, low)?))
}

impl<'m> Solver<'m> {
    // -----------------------------------------------------------------------
    // Project checks
    // -----------------------------------------------------------------------

    /// Exactly one root: one part of the project placed with `ground`, or
    /// one region made with `box(across:, up:, deep:)` (grammar,
    /// "Placement rules", rule 1). A box inside a component, or anywhere
    /// but as the whole value of a region, is reported by the resolver.
    pub(crate) fn check_ground(&mut self) -> Solved<()> {
        let mut grounded: Vec<(PartId, FileId, Span)> = Vec::new();
        for part in &self.model.parts {
            let Some(def) = self.model.defs.part(part.def) else {
                continue;
            };
            let PartKind::FromStock(body) = &def.kind else {
                continue;
            };
            if part.mirror_of.is_some() {
                continue;
            }
            for term in &body.place {
                if term.kind == PlaceTermKind::Ground {
                    if part.assembly.is_some() {
                        self.report(
                            Diagnostic::error(Code::Ground, format!("'{}' is placed with 'ground' inside a component", part.name), Label::new(def.decl.file, term.span))
                                .with_help("parts inside components are placed against what the component is handed, a Region or a Part param; only the project has a root"),
                        );
                        continue;
                    }
                    grounded.push((part.id, def.decl.file, term.span));
                }
            }
        }
        let boxes = self.root_boxes();
        match (grounded.as_slice(), boxes.as_slice()) {
            ([], [])
                if self
                    .model
                    .defs
                    .broken
                    .get(&self.model.root)
                    .is_some_and(|names| !names.is_empty()) =>
            {
                // The root may be an item with a syntax error.
                Err(self.skip().into())
            }
            ([], []) => {
                let reported = self.report(
                    Diagnostic::error(
                        Code::Ground,
                        "nothing is the root: no part is placed with 'ground' and no region is made with 'box'",
                        Label::new(self.project_file, self.project_span),
                    )
                    .with_help("add a root box with the overall sizes, region envelope = box(across: ..., up: ..., deep: ...), or place the lowest part with place { ground }"),
                );
                Err(reported.into())
            }
            ([_], []) | ([], [_]) => Ok(()),
            ([(first, ..), (second, file, span), ..], _) => {
                let (a, b) = (self.part_name(*first), self.part_name(*second));
                let reported = self.report(
                    Diagnostic::error(
                        Code::Ground,
                        format!("'{a}' and '{b}' are both placed with 'ground'"),
                        Label::new(*file, *span),
                    )
                    .with_help(format!("place '{b}' against another part instead")),
                );
                Err(reported.into())
            }
            ([(part, file, span)], [(region, ..), ..]) => {
                let part = self.part_name(*part);
                let region = self.region_name(*region);
                let reported = self.report(
                    Diagnostic::error(
                        Code::Ground,
                        format!("'{part}' is placed with 'ground', but '{region}' is made with 'box'; only one can be the root"),
                        Label::new(*file, *span),
                    )
                    .with_help(format!("place '{part}' against the box instead: in place of ground, flush({region}.bottom), flush({region}.left) and flush({region}.front)")),
                );
                Err(reported.into())
            }
            ([], [(first, ..), (second, file, span), ..]) => {
                let (a, b) = (self.region_name(*first), self.region_name(*second));
                let reported = self.report(
                    Diagnostic::error(
                        Code::Ground,
                        format!("'{a}' and '{b}' are both made with 'box'"),
                        Label::new(*file, *span),
                    )
                    .with_help(format!("make '{b}' from '{a}' instead, with split(...) or opening(...) and its faces")),
                );
                Err(reported.into())
            }
        }
    }

    /// The project's regions made with `box(across:, up:, deep:)`, in
    /// declaration order, with where each box is written. Regions in
    /// components are never the root.
    pub(crate) fn root_boxes(&self) -> Vec<(RegionId, FileId, Span)> {
        self.model
            .regions
            .iter()
            .filter(|region| region.assembly.is_none() && region.mirror_of.is_none())
            .filter_map(|region| {
                let def = self.model.defs.region(region.def)?;
                is_region_box(&def.value).then_some((region.id, def.decl.file, def.value.span))
            })
            .collect()
    }

    fn region_name(&self, region: RegionId) -> String {
        self.model
            .regions
            .get(region.index())
            .map(|r| r.name.clone())
            .unwrap_or_default()
    }

    // -----------------------------------------------------------------------
    // Part definitions and environments
    // -----------------------------------------------------------------------

    pub(crate) fn part_body(&self, part: PartId) -> Option<&'m PartBodyDef> {
        let model = self.model;
        let inst = model.parts.get(part.index())?;
        match &model.defs.part(inst.def)?.kind {
            PartKind::FromStock(body) => Some(body),
            PartKind::Mirror(_) => None,
        }
    }

    pub(crate) fn part_env(&self, part: PartId) -> Option<Env<'static>> {
        let inst = self.model.parts.get(part.index())?;
        let def = self.model.defs.part(inst.def)?;
        let array = def.array.as_ref().map(|_| {
            let def_id = DefId::Part(inst.def);
            (def_id, self.array_len(def_id, inst.ctx))
        });
        Some(Env {
            ctx: inst.ctx,
            file: def.decl.file,
            kind: None,
            array,
        })
    }

    /// For a part that is an image: its source and the mirror plane.
    pub(crate) fn mirror_source(
        &mut self,
        part: PartId,
    ) -> Solved<Option<(PartId, Axis, ExactLength)>> {
        if let Some(Some(mirror)) = self.parts.get(part.index()).map(|s| s.mirror) {
            return Ok(mirror);
        }
        let mirror = self.compute_mirror_source(part)?;
        if let Some(state) = self.parts.get_mut(part.index()) {
            state.mirror = Some(mirror);
        }
        Ok(mirror)
    }

    fn compute_mirror_source(
        &mut self,
        part: PartId,
    ) -> Solved<Option<(PartId, Axis, ExactLength)>> {
        let Some(inst) = self.model.parts.get(part.index()) else {
            return Err(Fault::Internal("a part").into());
        };
        if self.poisoned(DefId::Part(inst.def)) {
            return Err(self.skip().into());
        }
        if let Some(source) = inst.mirror_of {
            let Some(assembly) = inst.assembly else {
                return Err(Fault::Internal("the component a mirrored item is in").into());
            };
            let (axis, plane) = self.assembly_mirror_plane(assembly)?;
            return Ok(Some((source, axis, plane)));
        }
        let Some(def) = self.model.defs.part(inst.def) else {
            return Err(Fault::Internal("a part's declaration").into());
        };
        let PartKind::Mirror(mirror) = &def.kind else {
            return Ok(None);
        };
        let Some(env) = self.part_env(part) else {
            return Err(Fault::Internal("where a part is declared").into());
        };
        let source = self.eval_part(&mirror.source, &env)?;
        if source == part {
            return Err(self
                .error(
                    Code::Mirror,
                    format!(
                        "{} has no counterpart on the other side of the mirror",
                        inst.name
                    ),
                    def.decl.file,
                    mirror.source.span,
                    "mirror another part",
                )
                .into());
        }
        let plane = self.eval_anchor(&mirror.about, &env)?;
        Ok(Some((source, plane.axis, plane.position)))
    }

    /// The plane a mirrored assembly is reflected through (walking up to
    /// the mirrored assembly an assembly is inside).
    pub(crate) fn assembly_mirror_plane(
        &mut self,
        assembly: AssemblyId,
    ) -> Solved<(Axis, ExactLength)> {
        let mut current = assembly;
        loop {
            let Some(inst) = self.model.assemblies.get(current.index()) else {
                return Err(Fault::Internal("a component").into());
            };
            if inst.mirror_of.is_some() && self.is_mirror_declaration(current) {
                break;
            }
            match inst.parent {
                Some(parent) => current = parent,
                None => return Err(Fault::Internal("the mirror a component is inside").into()),
            }
        }
        let Some(inst) = self.model.assemblies.get(current.index()) else {
            return Err(Fault::Internal("a component").into());
        };
        let Some(def) = self.model.defs.assembly(inst.def) else {
            return Err(Fault::Internal("a component's declaration").into());
        };
        match self.mirror_planes.get(&current) {
            Some(Memo::Done(plane)) => return Ok(*plane),
            Some(Memo::Failed(accounted)) => return Err((*accounted).into()),
            Some(Memo::Busy) => {
                return Err(self
                    .error(
                        Code::PlacementCycle,
                        format!("the mirror of {} is placed against itself", inst.name),
                        def.decl.file,
                        def.decl.span,
                        "mirror about a face of something outside the mirrored component",
                    )
                    .into());
            }
            Some(Memo::Todo) | None => {}
        }
        let stave_model::defs::AssemblyKind::Mirror(mirror) = &def.kind else {
            return Err(Fault::Internal("the plane of a mirror").into());
        };
        let about = &mirror.about;
        let env = Env {
            ctx: inst.ctx,
            file: def.decl.file,
            kind: None,
            array: None,
        };
        self.mirror_planes.insert(current, Memo::Busy);
        let result = self.eval_anchor(about, &env).map(|a| (a.axis, a.position));
        let (memo, result) = self.remember(result, def.decl.file, def.decl.span);
        self.mirror_planes.insert(current, memo);
        result
    }

    /// `true` if the assembly is declared with `mirror(...)` (rather than
    /// being nested inside one).
    pub(crate) fn is_mirror_declaration(&self, assembly: AssemblyId) -> bool {
        self.model
            .assemblies
            .get(assembly.index())
            .and_then(|a| self.model.defs.assembly(a.def))
            .is_some_and(|d| matches!(d.kind, stave_model::defs::AssemblyKind::Mirror(_)))
    }

    // -----------------------------------------------------------------------
    // Stock, roundness and orientation
    // -----------------------------------------------------------------------

    pub(crate) fn part_stock(&mut self, part: PartId) -> Solved<StockId> {
        if let Some(board) = self.boards.get(&part) {
            let owner = board.part;
            return self.part_stock(owner);
        }
        if let Some((source, _, _)) = self.mirror_source(part)? {
            return self.part_stock(source);
        }
        let Some(body) = self.part_body(part) else {
            return Err(Fault::Internal("a part's description").into());
        };
        let Some(env) = self.part_env(part) else {
            return Err(Fault::Internal("where a part is declared").into());
        };
        match self.eval(&body.stock, &env)? {
            Value::Stock(stock) => Ok(stock),
            other => Err(self
                .error(
                    Code::TypeMismatch,
                    format!("this needs a stock, found {}", other.description()),
                    env.file,
                    body.stock.span,
                    "name a stock item",
                )
                .into()),
        }
    }

    /// The profile of a turned part.
    pub(crate) fn part_profile(
        &mut self,
        part: PartId,
    ) -> Solved<Option<stave_model::ids::ProfileId>> {
        if let Some((source, _, _)) = self.mirror_source(part)? {
            return self.part_profile(source);
        }
        let Some(body) = self.part_body(part) else {
            return Err(Fault::Internal("a part's description").into());
        };
        let Some(shape) = &body.shape else {
            return Ok(None);
        };
        let Some(env) = self.part_env(part) else {
            return Err(Fault::Internal("where a part is declared").into());
        };
        match self.eval(shape, &env)? {
            Value::Turned(profile) => Ok(Some(profile)),
            other => Err(self
                .error(
                    Code::TypeMismatch,
                    format!("'shape' needs a shape, found {}", other.description()),
                    env.file,
                    shape.span,
                    "write shape: turned(profile)",
                )
                .into()),
        }
    }

    pub(crate) fn is_round(&mut self, part: PartId) -> Solved<bool> {
        if self.part_profile(part)?.is_some() {
            return Ok(true);
        }
        let stock = self.part_stock(part)?;
        Ok(self
            .model
            .defs
            .stock(stock)
            .is_some_and(|s| s.form == StockForm::Dowel))
    }

    pub(crate) fn part_diameter(
        &mut self,
        part: PartId,
        env: &Env<'_>,
        span: Span,
    ) -> Solved<ExactLength> {
        if let Some(profile) = self.part_profile(part)? {
            let stations = self.profile_stations(profile)?;
            return Ok(stations
                .iter()
                .map(|(_, d)| *d)
                .max()
                .ok_or(Fault::Earlier)?);
        }
        let stock = self.part_stock(part)?;
        match self.stock_length(stock, StockField::Diameter)? {
            Some(diameter) => Ok(diameter),
            None => {
                let name = self.part_name(part);
                Err(self
                    .error(
                        Code::UnknownField,
                        "a part has no field 'diameter'".to_string(),
                        env.file,
                        span,
                        format!(
                            "{name} is not round; only dowels and turned parts have a diameter"
                        ),
                    )
                    .into())
            }
        }
    }

    pub(crate) fn part_orientation(&mut self, part: PartId) -> Solved<Orientation> {
        if let Some(board) = self.boards.get(&part) {
            return Ok(board.orientation);
        }
        match self.parts.get(part.index()).map(|s| &s.orientation) {
            Some(Memo::Done(o)) => return Ok(*o),
            Some(Memo::Failed(accounted)) => return Err((*accounted).into()),
            Some(Memo::Busy) => return Err(self.cycle(Frame::Orientation(part)).into()),
            _ => {}
        }
        self.set_orientation(part, Memo::Busy);
        self.enter(Frame::Orientation(part), None);
        let result = self.compute_orientation(part);
        self.leave();
        let (file, span) = self.part_place(part);
        let (memo, result) = self.remember(result, file, span);
        self.set_orientation(part, memo);
        result
    }

    fn set_orientation(&mut self, part: PartId, memo: Memo<Orientation>) {
        if let Some(state) = self.parts.get_mut(part.index()) {
            state.orientation = memo;
        }
    }

    fn compute_orientation(&mut self, part: PartId) -> Solved<Orientation> {
        if let Some((source, axis, _)) = self.mirror_source(part)? {
            return Ok(self.part_orientation(source)?.mirrored(axis));
        }
        let terms = self.part_terms(part)?;
        let Some(env) = self.part_env(part) else {
            return Err(Fault::Internal("where a part is declared").into());
        };
        let Some((length, width, span)) = terms.orient else {
            return Orientation::from_length_and_width(Direction::Right, Direction::Back)
                .map_err(|_| Fault::Internal("a part's orientation").into());
        };
        let direction = |this: &mut Self, expr: &RExpr| -> Solved<Direction> {
            match this.eval(expr, &env)? {
                Value::Word(Word::Direction(d)) => Ok(d),
                other => Err(this
                    .error(
                        Code::TypeMismatch,
                        format!("'orient' needs a direction, found {}", other.description()),
                        env.file,
                        expr.span,
                        "write up, down, left, right, front or back",
                    )
                    .into()),
            }
        };
        let length_direction = direction(self, length)?;
        let width_direction = match &width {
            Some(width) => direction(self, width)?,
            None => {
                if !self.is_round(part)? {
                    let name = self.part_name(part);
                    return Err(self.error(Code::MissingField, "'orient' is missing 'width'".to_string(), env.file, span, format!("{name} is not round, so say where its width runs: orient(length: ..., width: ...)")).into());
                }
                match length_direction.axis() {
                    Axis::Up | Axis::Deep => Direction::Right,
                    Axis::Across => Direction::Back,
                }
            }
        };
        Orientation::from_length_and_width(length_direction, width_direction).map_err(|_| {
            self.error(
                Code::TypeMismatch,
                format!(
                    "'orient' needs a length and width along different axes, found {} and {}",
                    length_direction.word(),
                    width_direction.word()
                ),
                env.file,
                span,
                "give the width a direction across the length",
            )
            .into()
        })
    }

    // -----------------------------------------------------------------------
    // Terms
    // -----------------------------------------------------------------------

    pub(crate) fn part_terms(&mut self, part: PartId) -> Solved<Terms<'m>> {
        match self.parts.get(part.index()).map(|s| &s.terms) {
            Some(Memo::Done(terms)) => return Ok(*terms),
            Some(Memo::Failed(accounted)) => return Err((*accounted).into()),
            _ => {}
        }
        let result = self.compute_part_terms(part);
        let (file, span) = self.part_place(part);
        let (memo, result) = self.remember(result, file, span);
        if let Some(state) = self.parts.get_mut(part.index()) {
            state.terms = memo;
        }
        result
    }

    fn compute_part_terms(&mut self, part: PartId) -> Solved<Terms<'m>> {
        if let Some(inst) = self.model.parts.get(part.index())
            && self.poisoned(DefId::Part(inst.def))
        {
            return Err(self.skip().into());
        }
        let Some(body) = self.part_body(part) else {
            return Err(Fault::Internal("a part's description").into());
        };
        let Some(env) = self.part_env(part) else {
            return Err(Fault::Internal("where a part is declared").into());
        };
        let name = self.part_name(part);
        let place_span = body.place_span.unwrap_or(body.span);
        let label = self.element_label(part);
        let terms = self.sort_terms(&body.place, &env, &name, label.as_deref(), false)?;
        if !terms.ground {
            for axis in Axis::ALL {
                if terms.axis(axis).is_none() {
                    return Err(self.axis_not_set(&name, axis, env.file, place_span).into());
                }
            }
        }
        Ok(terms)
    }

    pub(crate) fn axis_not_set(
        &mut self,
        name: &str,
        axis: Axis,
        file: FileId,
        span: Span,
    ) -> Reported {
        let example = match axis {
            Axis::Up => "against(base.top) or flush(base.bottom)",
            Axis::Across => "center(base, across) or flush(base.left)",
            Axis::Deep => "flush(base.front) or center(base, deep)",
        };
        self.error(
            Code::AxisNotSet,
            format!("{name}: position along '{}' is not set", axis.word()),
            file,
            span,
            format!("add a term that fixes '{}', such as {example}", axis.word()),
        )
    }

    /// The label of an array element, for `corners`.
    fn element_label(&self, part: PartId) -> Option<String> {
        let inst = self.model.parts.get(part.index())?;
        let element = inst.ctx.element?;
        match self.model.defs.array(DefId::Part(inst.def))? {
            stave_model::defs::ArrayDef::Labels(labels) => {
                labels.get(element).map(|(l, _)| l.clone())
            }
            stave_model::defs::ArrayDef::Count(_) => Some(element.to_string()),
        }
    }

    /// The world axis a face or plane expression is across, without
    /// placing anything when it can be read from the expression.
    fn anchor_axis(&mut self, expr: &RExpr, env: &Env<'_>) -> Solved<Axis> {
        match &expr.kind {
            RExprKind::Member {
                member: stave_model::defs::Member::Face(face),
                ..
            } => Ok(face.axis()),
            RExprKind::Builtin {
                function: Builtin::Mid,
                args,
            } => {
                match args
                    .iter()
                    .find(|a| a.name == "axis")
                    .map(|a| &a.value.kind)
                {
                    Some(RExprKind::Word(Word::Axis(axis))) => Ok(*axis),
                    _ => Ok(self.eval_anchor(expr, env)?.axis),
                }
            }
            _ => Ok(self.eval_anchor(expr, env)?.axis),
        }
    }

    /// Sorts placement terms by the axis each fixes, reporting a second
    /// term on an axis.
    pub(crate) fn sort_terms(
        &mut self,
        place: &'m [PlaceTerm],
        env: &Env<'_>,
        name: &str,
        label: Option<&str>,
        hardware: bool,
    ) -> Solved<Terms<'m>> {
        let mut terms = Terms::default();
        let mut first_spans: [Option<Span>; 3] = [None, None, None];
        let mut assign = |this: &mut Self,
                          terms: &mut Terms<'m>,
                          axis: Axis,
                          term: AxisTerm<'m>|
         -> Solved<()> {
            let slot = axis_slot(axis);
            if terms.ground || terms.axes.get(slot).is_some_and(Option::is_some) {
                let mut diagnostic = Diagnostic::error(
                    Code::AxisSetTwice,
                    format!("{name}: position along '{}' is set twice", axis.word()),
                    Label::new(env.file, term.span),
                );
                if let Some(Some(first)) = first_spans.get(slot) {
                    diagnostic = diagnostic.with_label(Label::with_message(
                        env.file,
                        *first,
                        "first set here",
                    ));
                }
                let help = if terms.ground {
                    "'ground' already fixes every axis; remove this term".to_owned()
                } else {
                    format!("keep one term for '{}' and remove the other", axis.word())
                };
                let reported = this.report(diagnostic.with_help(help));
                return Err(reported.into());
            }
            if let Some(span) = first_spans.get_mut(slot) {
                *span = Some(term.span);
            }
            if let Some(entry) = terms.axes.get_mut(slot) {
                *entry = Some(term);
            }
            Ok(())
        };
        let mut failed: Option<Accounted> = None;
        for term in place {
            let span = term.span;
            let result: Solved<()> = match &term.kind {
                PlaceTermKind::Ground => {
                    if terms.axes.iter().any(Option::is_some) {
                        let axis = Axis::ALL
                            .into_iter()
                            .find(|a| terms.axis(*a).is_some())
                            .unwrap_or(Axis::Up);
                        let reported = self.report(
                            Diagnostic::error(
                                Code::AxisSetTwice,
                                format!("{name}: position along '{}' is set twice", axis.word()),
                                Label::new(env.file, span),
                            )
                            .with_help("'ground' fixes every axis; remove the other terms"),
                        );
                        Err(reported.into())
                    } else {
                        terms.ground = true;
                        Ok(())
                    }
                }
                PlaceTermKind::Orient { length, width } => {
                    if terms.orient.is_some() {
                        let reported = self.report(
                            Diagnostic::error(
                                Code::AxisSetTwice,
                                format!("{name}: 'orient' is written twice"),
                                Label::new(env.file, span),
                            )
                            .with_help("keep one orient term"),
                        );
                        Err(reported.into())
                    } else {
                        terms.orient = Some((length, width.as_ref(), span));
                        Ok(())
                    }
                }
                PlaceTermKind::Against { face, gap } => {
                    self.anchor_axis(face, env).and_then(|axis| {
                        assign(
                            self,
                            &mut terms,
                            axis,
                            AxisTerm {
                                kind: AxisTermKind::Against {
                                    face,
                                    gap: gap.as_ref(),
                                },
                                span,
                            },
                        )
                    })
                }
                PlaceTermKind::Flush { face, inset } => {
                    self.anchor_axis(face, env).and_then(|axis| {
                        assign(
                            self,
                            &mut terms,
                            axis,
                            AxisTerm {
                                kind: AxisTermKind::Flush {
                                    face,
                                    inset: inset.as_ref(),
                                },
                                span,
                            },
                        )
                    })
                }
                PlaceTermKind::CenterOn { item, axis } => match self.eval(axis, env) {
                    Ok(Value::Word(Word::Axis(axis))) => assign(
                        self,
                        &mut terms,
                        axis,
                        AxisTerm {
                            kind: AxisTermKind::CenterOn { item },
                            span,
                        },
                    ),
                    Ok(_) => Err(Fault::Earlier.into()),
                    Err(failure) => Err(failure),
                },
                PlaceTermKind::CenterBetween { from, to } => {
                    let from_axis = self.anchor_axis(from, env);
                    let to_axis = self.anchor_axis(to, env);
                    match (from_axis, to_axis) {
                        (Ok(a), Ok(b)) if a == b => assign(self, &mut terms, a, AxisTerm { kind: AxisTermKind::CenterBetween { from, to }, span }),
                        (Ok(a), Ok(b)) => Err(self.error(Code::TypeMismatch, format!("'center' needs two parallel planes, found one across '{}' and one across '{}'", a.word(), b.word()), env.file, span, "name two faces across the same axis").into()),
                        (Err(failure), _) | (_, Err(failure)) => Err(failure),
                    }
                }
                PlaceTermKind::Spaced { spacing, from, to } => {
                    if label.is_none() {
                        Err(self
                            .error(
                                Code::TypeMismatch,
                                format!("'spaced' needs an array, but '{name}' is a single item"),
                                env.file,
                                span,
                                "declare an array: part name[a, b] or name[3]",
                            )
                            .into())
                    } else {
                        let from_axis = self.anchor_axis(from, env);
                        let to_axis = self.anchor_axis(to, env);
                        match (from_axis, to_axis) {
                            (Ok(a), Ok(b)) if a == b => assign(
                                self,
                                &mut terms,
                                a,
                                AxisTerm {
                                    kind: AxisTermKind::Spaced { spacing, from, to },
                                    span,
                                },
                            ),
                            (Ok(_), Ok(_)) => Err(self
                                .error(
                                    Code::TypeMismatch,
                                    "'spaced' needs two parallel planes".to_owned(),
                                    env.file,
                                    span,
                                    "name two faces across the same axis",
                                )
                                .into()),
                            (Err(failure), _) | (_, Err(failure)) => Err(failure),
                        }
                    }
                }
                PlaceTermKind::Corners { part, inset } => {
                    if hardware {
                        Err(Fault::Earlier.into())
                    } else {
                        match label.and_then(Corner::from_word) {
                            Some(corner) => assign(self, &mut terms, Axis::Across, AxisTerm { kind: AxisTermKind::Corner { part, inset: inset.as_ref(), face: corner.across_face() }, span })
                                .and_then(|()| assign(self, &mut terms, Axis::Deep, AxisTerm { kind: AxisTermKind::Corner { part, inset: inset.as_ref(), face: corner.deep_face() }, span })),
                            None => Err(self.error(
                                Code::TypeMismatch,
                                format!("'corners' needs the labels front_left, front_right, back_left and back_right, found '{}'", label.unwrap_or(name)),
                                env.file,
                                span,
                                "declare the array with the four corner labels",
                            ).into()),
                        }
                    }
                }
            };
            if let Err(failure) = result {
                let accounted = self.settle(failure, env.file, span);
                failed.get_or_insert(accounted);
            }
        }
        match failed {
            Some(accounted) => Err(accounted.into()),
            None => Ok(terms),
        }
    }

    // -----------------------------------------------------------------------
    // Sizes
    // -----------------------------------------------------------------------

    /// A part's drawn size along one of its own axes.
    pub(crate) fn drawn_size(&mut self, part: PartId, axis: PartAxis) -> Solved<ExactLength> {
        let slot = part_axis_slot(axis);
        if let Some(board) = self.boards.get(&part) {
            return Ok(board
                .drawn
                .get(slot)
                .copied()
                .ok_or(Fault::Internal("a board's size"))?);
        }
        match self.parts.get(part.index()).and_then(|s| s.sizes.get(slot)) {
            Some(Memo::Done(size)) => return Ok(*size),
            Some(Memo::Failed(accounted)) => return Err((*accounted).into()),
            Some(Memo::Busy) => return Err(self.cycle(Frame::Size(part, axis)).into()),
            _ => {}
        }
        self.set_size(part, slot, Memo::Busy);
        let at = self
            .part_body(part)
            .zip(self.part_env(part))
            .map(|(body, env)| (env.file, body.span));
        self.enter(Frame::Size(part, axis), at);
        let result = self.compute_size(part, axis);
        self.leave();
        let result = result.and_then(|size| {
            if size <= ExactLength::ZERO {
                let name = self.part_name(part);
                let (file, span) = at.unwrap_or((self.project_file, self.project_span));
                Err(self
                    .error(
                        Code::TypeMismatch,
                        format!(
                            "{name}: its {} works out as {}, which is not more than zero",
                            axis.word(),
                            stave_model::value::format_length(size)
                        ),
                        file,
                        span,
                        "check the sizes it is worked out from",
                    )
                    .into())
            } else {
                Ok(size)
            }
        });
        let (file, span) = at.unwrap_or_else(|| self.part_place(part));
        let (memo, result) = self.remember(result, file, span);
        self.set_size(part, slot, memo);
        result
    }

    fn set_size(&mut self, part: PartId, slot: usize, memo: Memo<ExactLength>) {
        if let Some(entry) = self
            .parts
            .get_mut(part.index())
            .and_then(|s| s.sizes.get_mut(slot))
        {
            *entry = memo;
        }
    }

    fn compute_size(&mut self, part: PartId, axis: PartAxis) -> Solved<ExactLength> {
        if let Some((source, _, _)) = self.mirror_source(part)? {
            return self.drawn_size(source, axis);
        }
        let Some(body) = self.part_body(part) else {
            return Err(Fault::Internal("a part's description").into());
        };
        let Some(env) = self.part_env(part) else {
            return Err(Fault::Internal("where a part is declared").into());
        };
        let profile = self.part_profile(part)?;
        let largest = match profile {
            Some(profile) => Some(
                self.profile_stations(profile)?
                    .iter()
                    .map(|(_, d)| *d)
                    .max()
                    .ok_or(Fault::Earlier)?,
            ),
            None => None,
        };
        match axis {
            PartAxis::Length => match &body.length {
                Size::Expr(expr) => {
                    let length = self.eval_length(expr, &env)?;
                    if let Some(profile) = profile {
                        let turned = self.profile_length(profile)?;
                        if length != turned {
                            let name = self.part_name(part);
                            let profile_name = self
                                .model
                                .defs
                                .profile(profile)
                                .map(|p| p.decl.name.clone())
                                .unwrap_or_default();
                            return Err(self.error(
                                Code::TypeMismatch,
                                format!(
                                    "{name}: its length is {}, but its profile '{profile_name}' is {} long",
                                    stave_model::value::format_length(length),
                                    stave_model::value::format_length(turned)
                                ),
                                env.file,
                                expr.span,
                                "a turned part is as long as its profile: write length: fit",
                            ).into());
                        }
                    }
                    Ok(length)
                }
                Size::Fit(span) => match profile {
                    Some(profile) => self.profile_length(profile),
                    None => self.fit(part, PartAxis::Length, *span),
                },
            },
            PartAxis::Width => {
                if let Some(largest) = largest {
                    return Ok(largest);
                }
                match &body.width {
                    Some(Size::Expr(expr)) => {
                        let width = self.eval_length(expr, &env)?;
                        self.check_fixed_width(part, width, env.file, expr.span)?;
                        Ok(width)
                    }
                    Some(Size::Fit(span)) => {
                        let width = self.fit(part, PartAxis::Width, *span)?;
                        self.check_fixed_width(part, width, env.file, *span)?;
                        Ok(width)
                    }
                    None => {
                        let stock = self.part_stock(part)?;
                        match self.stock_length(stock, StockField::Width)? {
                            Some(width) => Ok(width),
                            None => {
                                let name = self.part_name(part);
                                Err(self
                                    .error(
                                        Code::MissingField,
                                        format!("part '{name}' is missing 'width'"),
                                        env.file,
                                        body.span,
                                        "add width: ..., or cut it from stock with a fixed width",
                                    )
                                    .into())
                            }
                        }
                    }
                }
            }
            PartAxis::Thickness => {
                if let Some(largest) = largest {
                    return Ok(largest);
                }
                match &body.thickness {
                    Some(expr) => self.written_thickness(part, expr, &env),
                    None => {
                        let stock = self.part_stock(part)?;
                        Ok(self
                            .stock_length(stock, StockField::Thickness)?
                            .ok_or(Fault::Earlier)?)
                    }
                }
            }
        }
    }

    /// The name a stock is declared with.
    pub(crate) fn stock_name(&self, stock: StockId) -> String {
        self.model
            .defs
            .stock(stock)
            .map(|s| s.decl.name.clone())
            .unwrap_or_default()
    }

    /// A part's width against stock sold at a fixed `width:` (grammar,
    /// "stock"): narrower is ripped from it, wider is an error at `span`.
    pub(crate) fn check_fixed_width(
        &mut self,
        part: PartId,
        width: ExactLength,
        file: FileId,
        span: Span,
    ) -> Solved<()> {
        let stock = self.part_stock(part)?;
        let fixed = self
            .model
            .defs
            .stock(stock)
            .is_some_and(|def| def.width.is_some());
        if !fixed {
            return Ok(());
        }
        let Some(stock_width) = self.stock_length(stock, StockField::Width)? else {
            return Ok(());
        };
        if width <= stock_width {
            return Ok(());
        }
        let name = self.part_name(part);
        let stock_name = self.stock_name(stock);
        let reason = format!(
            "its width is {}, but {stock_name} is sold {} wide",
            stave_model::value::format_length(width),
            stave_model::value::format_length(stock_width)
        );
        if self.noted_with_earlier_part(Code::TypeMismatch, file, span, &reason, &name) {
            return Err(self.skip().into());
        }
        Err(self
            .error(
                Code::TypeMismatch,
                format!("{name}: {reason}"),
                file,
                span,
                format!(
                    "leave width out to use the width of {stock_name}, write a narrower width to rip it, or cut the part from wider stock"
                ),
            )
            .into())
    }

    /// A part's written `thickness:`, which may be planed thinner than its
    /// stock but never thicker (grammar, "part"). A sheet (plywood, MDF)
    /// can't be planed: its parts are the sheet's thickness.
    fn written_thickness(
        &mut self,
        part: PartId,
        expr: &RExpr,
        env: &Env<'_>,
    ) -> Solved<ExactLength> {
        let thickness = self.eval_length(expr, env)?;
        let stock = self.part_stock(part)?;
        let form = self
            .model
            .defs
            .stock(stock)
            .map(|def| def.form)
            .ok_or(Fault::Internal("a stock's declaration"))?;
        let sheet = match form {
            StockForm::Plywood | StockForm::Mdf => true,
            StockForm::Solid | StockForm::Dowel => false,
        };
        if sheet
            && let Some(stock_thickness) = self.stock_length(stock, StockField::Thickness)?
            && thickness != stock_thickness
        {
            let name = self.part_name(part);
            let stock_name = self.stock_name(stock);
            let sheet_word = match form {
                StockForm::Mdf => "MDF",
                StockForm::Plywood | StockForm::Solid | StockForm::Dowel => "plywood",
            };
            let reason = format!(
                "its thickness is {}, but {sheet_word} can't be planed; {stock_name} is {} thick",
                stave_model::value::format_length(thickness),
                stave_model::value::format_length(stock_thickness)
            );
            if self.noted_with_earlier_part(Code::TypeMismatch, env.file, expr.span, &reason, &name)
            {
                return Err(self.skip().into());
            }
            return Err(self
                .error(
                    Code::TypeMismatch,
                    format!("{name}: {reason}"),
                    env.file,
                    expr.span,
                    format!(
                        "leave thickness out to use the sheet's, or declare a {} sheet stock and cut the part from it",
                        stave_model::value::format_length(thickness)
                    ),
                )
                .into());
        }
        if let Some(stock_thickness) = self.stock_length(stock, StockField::Thickness)?
            && thickness > stock_thickness
        {
            let name = self.part_name(part);
            let stock_name = self
                .model
                .defs
                .stock(stock)
                .map(|s| s.decl.name.clone())
                .unwrap_or_default();
            return Err(self.error(
                Code::TypeMismatch,
                format!(
                    "{name}: its thickness works out as {}, which is more than its stock's {}",
                    stave_model::value::format_length(thickness),
                    stave_model::value::format_length(stock_thickness)
                ),
                env.file,
                expr.span,
                format!(
                    "a part can be planed thinner than its stock '{stock_name}', never thicker; choose thicker stock"
                ),
            ).into());
        }
        Ok(thickness)
    }

    /// One mistake, one error: when another part declared in the same
    /// place (an element of an array, a part of a component used twice)
    /// already has the error `<part>: <reason>` there, `name` becomes a
    /// note on it and `true` is returned.
    pub(crate) fn noted_with_earlier_part(
        &mut self,
        code: Code,
        file: FileId,
        span: Span,
        reason: &str,
        name: &str,
    ) -> bool {
        let tail = format!(": {reason}");
        let earlier = self.diagnostics.iter_mut().find(|diagnostic| {
            diagnostic.code == code
                && diagnostic.message.ends_with(&tail)
                && diagnostic
                    .primary()
                    .is_some_and(|label| label.file == file && label.span == span)
        });
        let Some(earlier) = earlier else {
            return false;
        };
        let note = format!("the same for {name}");
        if !earlier.notes.contains(&note) {
            earlier.notes.push(note);
        }
        true
    }

    /// A turned part is turned from a blank: its `width:` and `thickness:`
    /// (each defaulting to the stock's). Its profile must fit inside the
    /// blank at its widest (grammar, "part"); the drawn box stays the
    /// profile's largest diameter.
    pub(crate) fn check_turned_blank(&mut self, part: PartId) -> Solved<()> {
        if self.mirror_source(part)?.is_some()
            || self
                .model
                .parts
                .get(part.index())
                .is_some_and(|p| p.mirror_of.is_some())
        {
            return Ok(());
        }
        let Some(profile) = self.part_profile(part)? else {
            return Ok(());
        };
        let Some(body) = self.part_body(part) else {
            return Err(Fault::Internal("a part's description").into());
        };
        let Some(env) = self.part_env(part) else {
            return Err(Fault::Internal("where a part is declared").into());
        };
        let largest = self
            .profile_stations(profile)?
            .iter()
            .map(|(_, d)| *d)
            .max()
            .ok_or(Fault::Earlier)?;
        let stock = self.part_stock(part)?;
        let thickness = match &body.thickness {
            Some(expr) => Some(self.written_thickness(part, expr, &env)?),
            None => self.stock_length(stock, StockField::Thickness)?,
        };
        let width = match &body.width {
            Some(Size::Expr(expr)) => {
                let width = self.eval_length(expr, &env)?;
                self.check_fixed_width(part, width, env.file, expr.span)?;
                Some(width)
            }
            // A blank's width can't be fitted between other parts; the
            // turned part's box is its profile, so there is nothing to
            // measure the profile against.
            Some(Size::Fit(_)) => None,
            None => self.stock_length(stock, StockField::Width)?,
        };
        let (smaller, word) = match (width, thickness) {
            (Some(w), Some(t)) if w == t => (w, "square"),
            (Some(w), Some(t)) if w < t => (w, "wide"),
            (Some(_), Some(t)) | (None, Some(t)) => (t, "thick"),
            (Some(w), None) => (w, "wide"),
            (None, None) => return Ok(()),
        };
        if largest <= smaller {
            return Ok(());
        }
        let name = self.part_name(part);
        let stock_name = self
            .model
            .defs
            .stock(stock)
            .map(|s| s.decl.name.clone())
            .unwrap_or_default();
        let span = body.shape.as_ref().map_or(body.span, |shape| shape.span);
        let (largest, smaller) = (
            stave_model::value::format_length(largest),
            stave_model::value::format_length(smaller),
        );
        let reason = format!(
            "its profile is {largest} across at its widest, but its blank of {stock_name} is only {smaller} {word}"
        );
        if self.noted_with_earlier_part(Code::TypeMismatch, env.file, span, &reason, &name) {
            return Err(self.skip().into());
        }
        Err(self
            .error(
                Code::TypeMismatch,
                format!("{name}: {reason}"),
                env.file,
                span,
                format!(
                    "turn it from thicker stock, or make the profile no more than {smaller} across"
                ),
            )
            .into())
    }

    /// A part's drawn size along a world axis.
    pub(crate) fn size_along(&mut self, part: PartId, axis: Axis) -> Solved<ExactLength> {
        let orientation = self.part_orientation(part)?;
        self.drawn_size(part, orientation.part_axis(axis))
    }

    /// The parts a part is joined into, as the first role of a joint.
    fn joined_into(&mut self, part: PartId) -> Vec<PartId> {
        let mut partners = Vec::new();
        for index in 0..self.model.joints.len() {
            let joint = stave_model::ids::JointId::from_usize(index);
            if self
                .model
                .joints
                .get(index)
                .is_some_and(|j| j.mirror_of.is_some())
            {
                continue;
            }
            let binding = self.joint_binding(joint);
            let (file, span) = self.joint_place(joint);
            let Some(binding) = self.settled(binding, file, span) else {
                continue;
            };
            let Some(first) = binding.roles.first() else {
                continue;
            };
            if first.parts().is_some_and(|parts| parts.contains(&part)) {
                for other in binding.roles.iter().skip(1) {
                    for candidate in other.parts().unwrap_or_default() {
                        if candidate != part && !partners.contains(&candidate) {
                            partners.push(candidate);
                        }
                    }
                }
            }
        }
        partners
    }

    /// Works out a `fit` size (grammar, "Sizes").
    /// `fit` decided by the joints the part is the first role of (grammar,
    /// "Sizes"). When one of the project's joints couldn't be worked out
    /// (it, or something it depends on, has an error of its own), a size
    /// nothing decides isn't reported: that joint may be the one that
    /// decides it, so the one mistake stays one error.
    fn fit(&mut self, part: PartId, axis: PartAxis, span: Span) -> Solved<ExactLength> {
        let before = self.diagnostics.len();
        let result = self.decide_fit(part, axis, span);
        if result.is_err() && self.joints_incomplete() {
            let mut index = before;
            while index < self.diagnostics.len() {
                if self
                    .diagnostics
                    .get(index)
                    .is_some_and(|d| d.code == Code::FitUndecided)
                {
                    self.diagnostics.remove(index);
                } else {
                    index += 1;
                }
            }
            return Err(self.skip().into());
        }
        result
    }

    /// `true` when some joint of the project couldn't be worked out: an item
    /// was left out for a syntax error, or a joint's binding failed.
    fn joints_incomplete(&mut self) -> bool {
        if self
            .model
            .defs
            .broken
            .values()
            .any(|names| !names.is_empty())
        {
            return true;
        }
        (0..self.model.joints.len()).any(|index| {
            let mirrored = self
                .model
                .joints
                .get(index)
                .is_some_and(|j| j.mirror_of.is_some());
            if mirrored {
                return false;
            }
            let joint = stave_model::ids::JointId::from_usize(index);
            let binding = self.joint_binding(joint);
            let (file, span) = self.joint_place(joint);
            self.settled(binding, file, span).is_none()
        })
    }

    fn decide_fit(&mut self, part: PartId, axis: PartAxis, span: Span) -> Solved<ExactLength> {
        let name = self.part_name(part);
        let Some(env) = self.part_env(part) else {
            return Err(Fault::Internal("where a part is declared").into());
        };
        let orientation = self.part_orientation(part)?;
        let world = orientation.direction(axis).axis();
        let terms = self.part_terms(part)?;
        let Some(term) = terms.axis(world).copied() else {
            return Err(self
                .fit_undecided(
                    &name,
                    axis,
                    env.file,
                    span,
                    "nothing places it along that axis",
                )
                .into());
        };
        let partners = self.joined_into(part);
        if partners.is_empty() {
            return Err(self
                .fit_undecided(
                    &name,
                    axis,
                    env.file,
                    span,
                    "it is not the first part of any joint",
                )
                .into());
        }
        let mut extents = Vec::new();
        for partner in partners {
            let low = self.part_min(partner, world)?;
            let size = self.size_along(partner, world)?;
            extents.push((low, add(low, size)?));
        }
        let (low_side, high_side) = match world {
            Axis::Across => ("to the left of", "to the right of"),
            Axis::Up => ("below", "above"),
            Axis::Deep => ("in front of", "behind"),
        };
        let nearest_below = |at: ExactLength| {
            extents
                .iter()
                .filter(|(_, high)| *high <= at)
                .map(|(_, high)| *high)
                .max()
        };
        let nearest_above = |at: ExactLength| {
            extents
                .iter()
                .filter(|(low, _)| *low >= at)
                .map(|(low, _)| *low)
                .min()
        };
        match &term.kind {
            AxisTermKind::CenterOn { .. } | AxisTermKind::CenterBetween { .. } => {
                let centre = self.term_centre(&term, world, &env)?;
                let below = nearest_below(centre);
                let above = nearest_above(centre);
                match (below, above) {
                    (Some(below), Some(above)) => sub(above, below),
                    (None, _) => Err(self
                        .fit_undecided(
                            &name,
                            axis,
                            env.file,
                            span,
                            &format!("no part it is joined into is wholly {low_side} its centre"),
                        )
                        .into()),
                    (_, None) => Err(self
                        .fit_undecided(
                            &name,
                            axis,
                            env.file,
                            span,
                            &format!("no part it is joined into is wholly {high_side} its centre"),
                        )
                        .into()),
                }
            }
            AxisTermKind::Against { .. }
            | AxisTermKind::Flush { .. }
            | AxisTermKind::Corner { .. } => {
                let (known, is_low) = self.known_end(&term, world, &env)?;
                if is_low {
                    match nearest_above(known) {
                        Some(end) => sub(end, known),
                        None => Err(self
                            .fit_undecided(
                                &name,
                                axis,
                                env.file,
                                span,
                                &format!("no part it is joined into is {high_side} it"),
                            )
                            .into()),
                    }
                } else {
                    match nearest_below(known) {
                        Some(end) => sub(known, end),
                        None => Err(self
                            .fit_undecided(
                                &name,
                                axis,
                                env.file,
                                span,
                                &format!("no part it is joined into is {low_side} it"),
                            )
                            .into()),
                    }
                }
            }
            AxisTermKind::Spaced { .. } => Err(self
                .fit_undecided(
                    &name,
                    axis,
                    env.file,
                    span,
                    "a spaced part's size must be written",
                )
                .into()),
        }
    }

    fn fit_undecided(
        &mut self,
        name: &str,
        axis: PartAxis,
        file: FileId,
        span: Span,
        note: &str,
    ) -> Reported {
        self.report(
            Diagnostic::error(
                Code::FitUndecided,
                format!("{name}: nothing decides its {}", axis.word()),
                Label::new(file, span),
            )
            .with_note(note.to_owned())
            .with_help(format!(
                "measure it instead: {}: between(a, b)",
                axis.word()
            )),
        )
    }

    /// The centre a `center` term puts a part at.
    fn term_centre(&mut self, term: &AxisTerm, axis: Axis, env: &Env<'_>) -> Solved<ExactLength> {
        match &term.kind {
            AxisTermKind::CenterOn { item } => {
                let bounds = match self.eval(item, env)? {
                    Value::Part(part) => self.drawn_box(part)?,
                    Value::Region(region) => self.region_value(region)?.bounds,
                    other => {
                        return Err(self
                            .error(
                                Code::TypeMismatch,
                                format!(
                                    "'center' needs a part or a region, found {}",
                                    other.description()
                                ),
                                env.file,
                                item.span,
                                "name a part or region",
                            )
                            .into());
                    }
                };
                midpoint(bounds.min(axis), bounds.max(axis)).ok_or(Failure::Fault(Fault::Overflow))
            }
            AxisTermKind::CenterBetween { from, to } => {
                let a = self.eval_anchor(from, env)?;
                let b = self.eval_anchor(to, env)?;
                midpoint(a.position, b.position).ok_or(Failure::Fault(Fault::Overflow))
            }
            _ => Err(Fault::Internal("the centre a term puts a part at").into()),
        }
    }

    fn face_anchor(
        &mut self,
        expr: &RExpr,
        env: &Env<'_>,
        head: &str,
    ) -> Solved<(AnchorValue, Face)> {
        let anchor = self.eval_anchor(expr, env)?;
        match anchor.face {
            Some(face) => Ok((anchor, face)),
            None => Err(self
                .error(
                    Code::TypeMismatch,
                    format!("'{head}' needs a face, found a mid-plane"),
                    env.file,
                    expr.span,
                    "use center(...) for a mid-plane",
                )
                .into()),
        }
    }

    fn optional_length(&mut self, expr: Option<&RExpr>, env: &Env<'_>) -> Solved<ExactLength> {
        match expr {
            Some(expr) => self.eval_length(expr, env),
            None => Ok(ExactLength::ZERO),
        }
    }

    /// For `against`, `flush` and corners: the face of the item that the
    /// term puts in place, as (position, is it the low face).
    fn known_end(
        &mut self,
        term: &AxisTerm,
        _axis: Axis,
        env: &Env<'_>,
    ) -> Solved<(ExactLength, bool)> {
        match &term.kind {
            AxisTermKind::Against { face, gap } => {
                let (anchor, face) = self.face_anchor(face, env, "against")?;
                let gap = self.optional_length(*gap, env)?;
                if face.is_positive() {
                    Ok((add(anchor.position, gap)?, true))
                } else {
                    Ok((sub(anchor.position, gap)?, false))
                }
            }
            AxisTermKind::Flush { face, inset } => {
                let (anchor, face) = self.face_anchor(face, env, "flush")?;
                let inset = self.optional_length(*inset, env)?;
                if face.is_positive() {
                    Ok((sub(anchor.position, inset)?, false))
                } else {
                    Ok((add(anchor.position, inset)?, true))
                }
            }
            AxisTermKind::Corner { part, inset, face } => {
                let other = self.eval_part(part, env)?;
                let position = self.part_anchor(other, *face)?.position;
                let inset = self.optional_length(*inset, env)?;
                if face.is_positive() {
                    Ok((sub(position, inset)?, false))
                } else {
                    Ok((add(position, inset)?, true))
                }
            }
            _ => Err(Fault::Internal("the face a term puts a part against").into()),
        }
    }

    /// Records an element spaced with `pitch` or `gap` whose far edge
    /// lies past the `to` anchor; layer 4 reports it.
    #[allow(clippy::too_many_arguments)]
    fn note_overrun(
        &mut self,
        part: PartId,
        to: &stave_model::value::AnchorValue,
        spacing: Spacing,
        low: ExactLength,
        own: ExactLength,
        forward: bool,
        siblings: &Siblings,
        span: ExactLength,
    ) -> Solved<()> {
        let fixed = match spacing {
            Spacing::Pitch(pitch) => {
                stave_model::solved::FixedSpacing::Pitch(crate::joints::round_length(pitch)?)
            }
            Spacing::Gap(gap) => {
                stave_model::solved::FixedSpacing::Gap(crate::joints::round_length(gap)?)
            }
            Spacing::Evenly | Spacing::Ends(_) => return Ok(()),
        };
        let past = if forward {
            sub(add(low, own)?, to.position)?
        } else {
            sub(to.position, low)?
        };
        if past <= ExactLength::ZERO {
            return Ok(());
        }
        let n = siblings.sizes.len();
        let largest = if n <= 1 {
            None
        } else {
            let first = siblings.sizes.first().copied().unwrap_or(own);
            let last = siblings.sizes.last().copied().unwrap_or(own);
            let room = match spacing {
                Spacing::Pitch(_) => sub(sub(span, divide(first, 2)?)?, divide(last, 2)?)?,
                _ => {
                    let total = siblings
                        .sizes
                        .iter()
                        .try_fold(ExactLength::ZERO, |acc, s| add(acc, *s))?;
                    sub(span, total)?
                }
            };
            let value = divide(room, n - 1)?;
            // Rounded down, so the value it suggests always fits.
            let nm = value.value().floor();
            (value > ExactLength::ZERO)
                .then(|| i64::try_from(nm).ok().map(Length::from_nm))
                .flatten()
        };
        let item = match to.owner {
            stave_model::value::AnchorOwner::Part(id) => stave_model::solved::AnchorItem::Part(id),
            stave_model::value::AnchorOwner::Region(id) => {
                stave_model::solved::AnchorItem::Region(id)
            }
        };
        self.overruns.push(stave_model::solved::Overrun {
            part,
            to: item,
            face: to.face,
            spacing: fixed,
            past: crate::joints::round_length(past)?,
            largest,
        });
        Ok(())
    }

    /// The low end along `axis` of an item of size `size` placed by `term`.
    pub(crate) fn term_low(
        &mut self,
        term: &AxisTerm,
        axis: Axis,
        size: ExactLength,
        env: &Env<'_>,
        siblings: Option<&Siblings>,
    ) -> Solved<ExactLength> {
        match &term.kind {
            AxisTermKind::Against { .. }
            | AxisTermKind::Flush { .. }
            | AxisTermKind::Corner { .. } => {
                let (known, is_low) = self.known_end(term, axis, env)?;
                if is_low { Ok(known) } else { sub(known, size) }
            }
            AxisTermKind::CenterOn { .. } | AxisTermKind::CenterBetween { .. } => {
                let centre = self.term_centre(term, axis, env)?;
                sub(centre, divide(size, 2)?)
            }
            AxisTermKind::Spaced { spacing, from, to } => {
                let Some(siblings) = siblings else {
                    return Err(Fault::Internal("the other parts of a spaced array").into());
                };
                let spacing = match self.eval(spacing, env)? {
                    Value::Spacing(spacing) => spacing,
                    Value::Word(Word::Evenly) => Spacing::Evenly,
                    Value::Word(Word::Ends) => Spacing::Ends(ExactLength::ZERO),
                    other => {
                        return Err(self
                            .error(
                                Code::TypeMismatch,
                                format!("'spaced' needs a spacing, found {}", other.description()),
                                env.file,
                                spacing.span,
                                "write evenly, ends, ends(inset: d), pitch(d) or gap(d)",
                            )
                            .into());
                    }
                };
                let a = self.eval_anchor(from, env)?.position;
                let b = self.eval_anchor(to, env)?.position;
                let forward = b >= a;
                let span = if forward { sub(b, a)? } else { sub(a, b)? };
                let n = siblings.sizes.len();
                let index = siblings.index;
                let total = siblings
                    .sizes
                    .iter()
                    .try_fold(ExactLength::ZERO, |acc, s| add(acc, *s))?;
                let before = siblings
                    .sizes
                    .iter()
                    .take(index)
                    .try_fold(ExactLength::ZERO, |acc, s| add(acc, *s))?;
                let own = siblings.sizes.get(index).copied().unwrap_or(size);
                let offset = match spacing {
                    Spacing::Evenly => {
                        let gap = divide(sub(span, total)?, n + 1)?;
                        add(scale(gap, index + 1)?, before)?
                    }
                    Spacing::Ends(inset) => {
                        if n <= 1 {
                            inset
                        } else {
                            let free = sub(sub(span, scale(inset, 2)?)?, total)?;
                            let gap = divide(free, n - 1)?;
                            add(add(inset, scale(gap, index)?)?, before)?
                        }
                    }
                    Spacing::Pitch(pitch) => {
                        let first = siblings.sizes.first().copied().unwrap_or(size);
                        let centre = add(divide(first, 2)?, scale(pitch, index)?)?;
                        sub(centre, divide(own, 2)?)?
                    }
                    Spacing::Gap(gap) => add(scale(gap, index)?, before)?,
                };
                let low = if forward {
                    add(a, offset)?
                } else {
                    sub(sub(a, offset)?, own)?
                };
                if let Some(part) = siblings.part {
                    let to = self.eval_anchor(to, env)?;
                    self.note_overrun(part, &to, spacing, low, own, forward, siblings, span)?;
                }
                Ok(low)
            }
        }
    }

    // -----------------------------------------------------------------------
    // Positions
    // -----------------------------------------------------------------------

    /// The low end of a part's drawn box along a world axis.
    pub(crate) fn part_min(&mut self, part: PartId, axis: Axis) -> Solved<ExactLength> {
        let slot = axis_slot(axis);
        if let Some(board) = self.boards.get(&part) {
            return Ok(board
                .min
                .get(slot)
                .copied()
                .ok_or(Fault::Internal("a board's position"))?);
        }
        match self.parts.get(part.index()).and_then(|s| s.mins.get(slot)) {
            Some(Memo::Done(v)) => return Ok(*v),
            Some(Memo::Failed(accounted)) => return Err((*accounted).into()),
            Some(Memo::Busy) => return Err(self.cycle(Frame::Position(part, axis)).into()),
            _ => {}
        }
        self.set_min(part, slot, Memo::Busy);
        let at = self.term_location(part, axis);
        self.enter(Frame::Position(part, axis), at);
        let result = self.compute_min(part, axis);
        self.leave();
        let (file, span) = at.unwrap_or_else(|| self.part_place(part));
        let (memo, result) = self.remember(result, file, span);
        self.set_min(part, slot, memo);
        result
    }

    fn term_location(&self, part: PartId, axis: Axis) -> Option<(FileId, Span)> {
        let env = self.part_env(part)?;
        if let Some(Memo::Done(terms)) = self.parts.get(part.index()).map(|s| &s.terms) {
            return terms.axis(axis).map(|t| (env.file, t.span));
        }
        self.part_body(part)
            .map(|b| (env.file, b.place_span.unwrap_or(b.span)))
    }

    fn set_min(&mut self, part: PartId, slot: usize, memo: Memo<ExactLength>) {
        if let Some(entry) = self
            .parts
            .get_mut(part.index())
            .and_then(|s| s.mins.get_mut(slot))
        {
            *entry = memo;
        }
    }

    fn compute_min(&mut self, part: PartId, axis: Axis) -> Solved<ExactLength> {
        if let Some((source, plane_axis, plane)) = self.mirror_source(part)? {
            let low = self.part_min(source, axis)?;
            if axis != plane_axis {
                return Ok(low);
            }
            let size = self.size_along(source, axis)?;
            let (new_low, _) = reflect(low, add(low, size)?, plane)?;
            return Ok(new_low);
        }
        let terms = self.part_terms(part)?;
        if terms.ground {
            return Ok(ExactLength::ZERO);
        }
        let Some(env) = self.part_env(part) else {
            return Err(Fault::Internal("where a part is declared").into());
        };
        let Some(term) = terms.axis(axis).copied() else {
            return Err(Fault::Internal("the term that places a part").into());
        };
        let size = self.size_along(part, axis)?;
        let siblings = match &term.kind {
            AxisTermKind::Spaced { .. } => Some(self.part_siblings(part, axis)?),
            _ => None,
        };
        self.term_low(&term, axis, size, &env, siblings.as_ref())
    }

    fn part_siblings(&mut self, part: PartId, axis: Axis) -> Solved<Siblings> {
        let Some(inst) = self.model.parts.get(part.index()) else {
            return Err(Fault::Internal("a part").into());
        };
        let index = inst.ctx.element.unwrap_or(0);
        let all = match self.model.item(DefId::Part(inst.def), inst.ctx) {
            Some(ItemSlot::Part(slot)) => slot.all(),
            _ => vec![part],
        };
        let mut sizes = Vec::new();
        for sibling in all {
            sizes.push(self.size_along(sibling, axis)?);
        }
        Ok(Siblings {
            index,
            sizes,
            part: Some(part),
        })
    }

    /// A part's drawn box in world coordinates.
    pub(crate) fn drawn_box(&mut self, part: PartId) -> Solved<ExactBox> {
        let mut extents = Vec::new();
        for axis in [Axis::Across, Axis::Up, Axis::Deep] {
            let low = self.part_min(part, axis)?;
            let size = self.size_along(part, axis)?;
            extents.push((low, add(low, size)?));
        }
        let [across, up, deep] = extents.as_slice() else {
            return Err(Fault::Internal("a part's box").into());
        };
        Ok(ExactBox::new(*across, *up, *deep))
    }

    /// A face of a part as an anchor.
    pub(crate) fn part_anchor(&mut self, part: PartId, face: Face) -> Solved<AnchorValue> {
        let axis = face.axis();
        let low = self.part_min(part, axis)?;
        let position = if face.is_positive() {
            add(low, self.size_along(part, axis)?)?
        } else {
            low
        };
        Ok(AnchorValue {
            axis,
            position,
            face: Some(face),
            owner: AnchorOwner::Part(part),
        })
    }

    // -----------------------------------------------------------------------
    // Regions
    // -----------------------------------------------------------------------

    /// A region's box and origin.
    pub(crate) fn region_value(&mut self, region: RegionId) -> Solved<RegionValue> {
        match self.regions.get(region.index()) {
            Some(Memo::Done(v)) => return Ok(*v),
            Some(Memo::Failed(accounted)) => return Err((*accounted).into()),
            Some(Memo::Busy) => return Err(self.cycle(Frame::Region(region)).into()),
            _ => {}
        }
        let Some(inst) = self.model.regions.get(region.index()) else {
            return Err(Fault::Internal("a region").into());
        };
        let Some(def) = self.model.defs.region(inst.def) else {
            return Err(Fault::Internal("a region's declaration").into());
        };
        // Busy only once nothing below can return before the memo is set.
        if let Some(slot) = self.regions.get_mut(region.index()) {
            *slot = Memo::Busy;
        }
        self.enter(Frame::Region(region), Some((def.decl.file, def.decl.span)));
        let result = self.compute_region(region, inst, def);
        self.leave();
        let (memo, result) = self.remember(result, def.decl.file, def.decl.span);
        if let Some(slot) = self.regions.get_mut(region.index()) {
            *slot = memo;
        }
        result
    }

    fn compute_region(
        &mut self,
        region: RegionId,
        inst: &stave_model::instance::RegionInst,
        def: &stave_model::defs::RegionDef,
    ) -> Solved<RegionValue> {
        if self.poisoned(DefId::Region(inst.def)) {
            let _ = region;
            return Err(self.skip().into());
        }
        if let Some(source) = inst.mirror_of {
            let Some(assembly) = inst.assembly else {
                return Err(Fault::Internal("the component a mirrored item is in").into());
            };
            let (axis, plane) = self.assembly_mirror_plane(assembly)?;
            let value = self.region_value(source)?;
            let (low, high) = reflect(value.bounds.min(axis), value.bounds.max(axis), plane)?;
            let _ = region;
            return Ok(RegionValue {
                bounds: value.bounds.with(axis, low, high),
                origin: value.origin,
            });
        }
        let array = def.array.as_ref().map(|_| {
            let id = DefId::Region(inst.def);
            (id, self.array_len(id, inst.ctx))
        });
        let env = Env {
            ctx: inst.ctx,
            file: def.decl.file,
            kind: None,
            array,
        };
        match self.eval(&def.value, &env)? {
            Value::Region(other) => self.region_value(other),
            Value::Regions(list) => {
                let index = inst.ctx.element.unwrap_or(0);
                let expected = array.map_or(1, |(_, n)| n);
                if list.len() != expected {
                    return Err(self
                        .error(
                            Code::TypeMismatch,
                            format!(
                                "'{}' needs {} regions, found {}",
                                def.decl.name,
                                expected,
                                list.len()
                            ),
                            def.decl.file,
                            def.value.span,
                            "give the array as many labels as the expression makes regions",
                        )
                        .into());
                }
                let value = list
                    .get(index)
                    .copied()
                    .ok_or(Fault::Internal("a region of an array"))?;
                // A piece of a split with nothing between keeps its place
                // in the split; anything else is plain.
                Ok(match value.origin {
                    RegionOrigin::Split { .. } => value,
                    RegionOrigin::Plain => RegionValue {
                        bounds: value.bounds,
                        origin: RegionOrigin::Plain,
                    },
                })
            }
            other => Err(self
                .error(
                    Code::TypeMismatch,
                    format!(
                        "'{}' needs a region, found {}",
                        def.decl.name,
                        other.description()
                    ),
                    def.decl.file,
                    def.value.span,
                    "write opening(...), split(...) or cover(...)",
                )
                .into()),
        }
    }
}
