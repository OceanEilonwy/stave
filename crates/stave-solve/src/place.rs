//! Placing parts and regions: orientation, drawn sizes (with `fit`), and
//! the position of each part along each world axis.
//!
//! Positions are the low end (left, bottom, front) of a part's **drawn**
//! box along each world axis, as exact fractions.

use stave_lang::ast::Span;
use stave_lang::placement::{Axis, Corner, Direction, Face, PartAxis};
use stave_lang::units::{ExactLength, Rational};
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
use crate::{Frame, Memo, Reported, Solved, Solver};

/// What is remembered about a part while it is placed.
#[derive(Debug, Clone, Default)]
pub(crate) struct PartState {
    pub orientation: Memo<Orientation>,
    pub sizes: [Memo<ExactLength>; 3],
    pub mins: [Memo<ExactLength>; 3],
    pub terms: Memo<Terms>,
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
#[derive(Debug, Clone, PartialEq)]
pub enum AxisTermKind {
    /// `against(face, gap: d)`
    Against {
        /// The face.
        face: RExpr,
        /// The gap.
        gap: Option<RExpr>,
    },
    /// `flush(face, inset: d)`
    Flush {
        /// The face.
        face: RExpr,
        /// The inset.
        inset: Option<RExpr>,
    },
    /// `center(item, axis)`
    CenterOn {
        /// The part or region.
        item: RExpr,
    },
    /// `center(from: a, to: b)`
    CenterBetween {
        /// One plane.
        from: RExpr,
        /// The other.
        to: RExpr,
    },
    /// `spaced(spacing, from: a, to: b)`
    Spaced {
        /// The spacing.
        spacing: RExpr,
        /// The first plane.
        from: RExpr,
        /// The last plane.
        to: RExpr,
    },
    /// One axis of `corners(part, inset: d)`: flush with this face of the part.
    Corner {
        /// The part.
        part: RExpr,
        /// The inset.
        inset: Option<RExpr>,
        /// The face of the part on this axis.
        face: Face,
    },
}

/// A term with where it is written.
#[derive(Debug, Clone, PartialEq)]
pub struct AxisTerm {
    /// The term.
    pub kind: AxisTermKind,
    /// Where it is written.
    pub span: Span,
}

/// A `place { }` block sorted by axis.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Terms {
    /// `true` for the ground part.
    pub ground: bool,
    /// `orient(length:, width:)` and its span.
    pub orient: Option<(RExpr, Option<RExpr>, Span)>,
    /// The term fixing each world axis (across, up, deep).
    pub axes: [Option<AxisTerm>; 3],
}

impl Terms {
    /// The term fixing an axis.
    pub fn axis(&self, axis: Axis) -> Option<&AxisTerm> {
        self.axes.get(axis_slot(axis)).and_then(Option::as_ref)
    }
}

/// How spaced elements and corner elements find their place.
pub(crate) struct Siblings {
    /// This element's index.
    pub index: usize,
    /// Every element's size along the axis.
    pub sizes: Vec<ExactLength>,
}

fn add(a: ExactLength, b: ExactLength) -> Solved<ExactLength> {
    a.checked_add(b).ok_or(Reported)
}

fn sub(a: ExactLength, b: ExactLength) -> Solved<ExactLength> {
    a.checked_sub(b).ok_or(Reported)
}

fn scale(a: ExactLength, n: usize) -> Solved<ExactLength> {
    let n = i64::try_from(n).map_err(|_| Reported)?;
    a.checked_mul(Rational::from(n)).ok_or(Reported)
}

fn divide(a: ExactLength, n: usize) -> Solved<ExactLength> {
    let n = i64::try_from(n).map_err(|_| Reported)?;
    a.checked_div(Rational::from(n)).ok_or(Reported)
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

impl Solver<'_> {
    // -----------------------------------------------------------------------
    // Project checks
    // -----------------------------------------------------------------------

    /// Exactly one part of the project is placed with `ground`.
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
                                .with_help("parts inside components are placed against what the component is handed; only one part of the project is ground"),
                        );
                        continue;
                    }
                    grounded.push((part.id, def.decl.file, term.span));
                }
            }
        }
        match grounded.as_slice() {
            [] if self
                .model
                .defs
                .broken
                .get(&self.model.root)
                .is_some_and(|names| !names.is_empty()) =>
            {
                // The ground part may be one with a syntax error.
                Err(self.skip())
            }
            [] => {
                self.report(
                    Diagnostic::error(
                        Code::Ground,
                        "no part is placed with 'ground'",
                        Label::new(self.project_file, self.project_span),
                    )
                    .with_help("place the part everything stands on with place { ground }"),
                );
                Err(Reported)
            }
            [_] => Ok(()),
            [(first, ..), (second, file, span), ..] => {
                let (a, b) = (self.part_name(*first), self.part_name(*second));
                self.report(
                    Diagnostic::error(
                        Code::Ground,
                        format!("'{a}' and '{b}' are both placed with 'ground'"),
                        Label::new(*file, *span),
                    )
                    .with_help(format!("place '{b}' against another part instead")),
                );
                Err(Reported)
            }
        }
    }

    // -----------------------------------------------------------------------
    // Part definitions and environments
    // -----------------------------------------------------------------------

    pub(crate) fn part_body(&self, part: PartId) -> Option<&PartBodyDef> {
        let inst = self.model.parts.get(part.index())?;
        match &self.model.defs.part(inst.def)?.kind {
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
        let Some(inst) = self.model.parts.get(part.index()).cloned() else {
            return Err(Reported);
        };
        if self.poisoned(DefId::Part(inst.def)) {
            return Err(self.skip());
        }
        if let Some(source) = inst.mirror_of {
            let Some(assembly) = inst.assembly else {
                return Err(Reported);
            };
            let (axis, plane) = self.assembly_mirror_plane(assembly)?;
            return Ok(Some((source, axis, plane)));
        }
        let Some(def) = self.model.defs.part(inst.def) else {
            return Err(Reported);
        };
        let PartKind::Mirror(mirror) = &def.kind else {
            return Ok(None);
        };
        let mirror = mirror.clone();
        let Some(env) = self.part_env(part) else {
            return Err(Reported);
        };
        let source = self.eval_part(&mirror.source, &env)?;
        if source == part {
            return Err(self.error(
                Code::Mirror,
                format!(
                    "{} has no counterpart on the other side of the mirror",
                    inst.name
                ),
                def.decl.file,
                mirror.source.span,
                "mirror another part",
            ));
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
            let Some(inst) = self.model.assemblies.get(current.index()).cloned() else {
                return Err(Reported);
            };
            if inst.mirror_of.is_some() && self.is_mirror_declaration(current) {
                break;
            }
            match inst.parent {
                Some(parent) => current = parent,
                None => return Err(Reported),
            }
        }
        match self.mirror_planes.get(&current) {
            Some(Memo::Done(plane)) => return Ok(*plane),
            Some(Memo::Failed | Memo::Busy) => return Err(Reported),
            _ => {}
        }
        let Some(inst) = self.model.assemblies.get(current.index()).cloned() else {
            return Err(Reported);
        };
        let Some(def) = self.model.defs.assembly(inst.def) else {
            return Err(Reported);
        };
        let stave_model::defs::AssemblyKind::Mirror(mirror) = &def.kind else {
            return Err(Reported);
        };
        let about = mirror.about.clone();
        let env = Env {
            ctx: inst.ctx,
            file: def.decl.file,
            kind: None,
            array: None,
        };
        self.mirror_planes.insert(current, Memo::Busy);
        let result = self.eval_anchor(&about, &env).map(|a| (a.axis, a.position));
        self.mirror_planes.insert(
            current,
            match result {
                Ok(plane) => Memo::Done(plane),
                Err(_) => Memo::Failed,
            },
        );
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
        if let Some((source, _, _)) = self.mirror_source(part)? {
            return self.part_stock(source);
        }
        let Some(body) = self.part_body(part).cloned() else {
            return Err(Reported);
        };
        let Some(env) = self.part_env(part) else {
            return Err(Reported);
        };
        match self.eval(&body.stock, &env)? {
            Value::Stock(stock) => Ok(stock),
            other => Err(self.error(
                Code::TypeMismatch,
                format!("this needs a stock, found {}", other.description()),
                env.file,
                body.stock.span,
                "name a stock item",
            )),
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
        let Some(body) = self.part_body(part).cloned() else {
            return Err(Reported);
        };
        let Some(shape) = &body.shape else {
            return Ok(None);
        };
        let Some(env) = self.part_env(part) else {
            return Err(Reported);
        };
        match self.eval(shape, &env)? {
            Value::Turned(profile) => Ok(Some(profile)),
            other => Err(self.error(
                Code::TypeMismatch,
                format!("'shape' needs a shape, found {}", other.description()),
                env.file,
                shape.span,
                "write shape: turned(profile)",
            )),
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
            return stations.iter().map(|(_, d)| *d).max().ok_or(Reported);
        }
        let stock = self.part_stock(part)?;
        match self.stock_length(stock, StockField::Diameter)? {
            Some(diameter) => Ok(diameter),
            None => {
                let name = self.part_name(part);
                Err(self.error(
                    Code::UnknownField,
                    "a part has no field 'diameter'".to_string(),
                    env.file,
                    span,
                    format!("{name} is not round; only dowels and turned parts have a diameter"),
                ))
            }
        }
    }

    pub(crate) fn part_orientation(&mut self, part: PartId) -> Solved<Orientation> {
        match self.parts.get(part.index()).map(|s| &s.orientation) {
            Some(Memo::Done(o)) => return Ok(*o),
            Some(Memo::Failed) => return Err(Reported),
            Some(Memo::Busy) => return Err(self.cycle(Frame::Orientation(part))),
            _ => {}
        }
        self.set_orientation(part, Memo::Busy);
        self.enter(Frame::Orientation(part), None);
        let result = self.compute_orientation(part);
        self.leave();
        self.set_orientation(
            part,
            match &result {
                Ok(o) => Memo::Done(*o),
                Err(_) => Memo::Failed,
            },
        );
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
            return Err(Reported);
        };
        let Some((length, width, span)) = terms.orient else {
            return Orientation::from_length_and_width(Direction::Right, Direction::Back)
                .map_err(|_| Reported);
        };
        let direction = |this: &mut Self, expr: &RExpr| -> Solved<Direction> {
            match this.eval(expr, &env)? {
                Value::Word(Word::Direction(d)) => Ok(d),
                other => Err(this.error(
                    Code::TypeMismatch,
                    format!("'orient' needs a direction, found {}", other.description()),
                    env.file,
                    expr.span,
                    "write up, down, left, right, front or back",
                )),
            }
        };
        let length_direction = direction(self, &length)?;
        let width_direction = match &width {
            Some(width) => direction(self, width)?,
            None => {
                if !self.is_round(part)? {
                    let name = self.part_name(part);
                    return Err(self.error(Code::MissingField, "'orient' is missing 'width'".to_string(), env.file, span, format!("{name} is not round, so say where its width runs: orient(length: ..., width: ...)")));
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
        })
    }

    // -----------------------------------------------------------------------
    // Terms
    // -----------------------------------------------------------------------

    pub(crate) fn part_terms(&mut self, part: PartId) -> Solved<Terms> {
        match self.parts.get(part.index()).map(|s| &s.terms) {
            Some(Memo::Done(terms)) => return Ok(terms.clone()),
            Some(Memo::Failed) => return Err(Reported),
            _ => {}
        }
        let result = self.compute_part_terms(part);
        if let Some(state) = self.parts.get_mut(part.index()) {
            state.terms = match &result {
                Ok(terms) => Memo::Done(terms.clone()),
                Err(_) => Memo::Failed,
            };
        }
        result
    }

    fn compute_part_terms(&mut self, part: PartId) -> Solved<Terms> {
        if let Some(inst) = self.model.parts.get(part.index())
            && self.poisoned(DefId::Part(inst.def))
        {
            return Err(self.skip());
        }
        let Some(body) = self.part_body(part).cloned() else {
            return Err(Reported);
        };
        let Some(env) = self.part_env(part) else {
            return Err(Reported);
        };
        let name = self.part_name(part);
        let place_span = body.place_span.unwrap_or(body.span);
        let label = self.element_label(part);
        let terms = self.sort_terms(&body.place, &env, &name, label.as_deref(), false)?;
        if !terms.ground {
            for axis in Axis::ALL {
                if terms.axis(axis).is_none() {
                    return Err(self.axis_not_set(&name, axis, env.file, place_span));
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
        place: &[PlaceTerm],
        env: &Env<'_>,
        name: &str,
        label: Option<&str>,
        hardware: bool,
    ) -> Solved<Terms> {
        let mut terms = Terms::default();
        let mut first_spans: [Option<Span>; 3] = [None, None, None];
        let mut assign =
            |this: &mut Self, terms: &mut Terms, axis: Axis, term: AxisTerm| -> Solved<()> {
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
                    this.report(diagnostic.with_help(help));
                    return Err(Reported);
                }
                if let Some(span) = first_spans.get_mut(slot) {
                    *span = Some(term.span);
                }
                if let Some(entry) = terms.axes.get_mut(slot) {
                    *entry = Some(term);
                }
                Ok(())
            };
        let mut failed = false;
        for term in place {
            let span = term.span;
            let result: Solved<()> = match &term.kind {
                PlaceTermKind::Ground => {
                    if terms.axes.iter().any(Option::is_some) {
                        let axis = Axis::ALL
                            .into_iter()
                            .find(|a| terms.axis(*a).is_some())
                            .unwrap_or(Axis::Up);
                        self.report(
                            Diagnostic::error(
                                Code::AxisSetTwice,
                                format!("{name}: position along '{}' is set twice", axis.word()),
                                Label::new(env.file, span),
                            )
                            .with_help("'ground' fixes every axis; remove the other terms"),
                        );
                        Err(Reported)
                    } else {
                        terms.ground = true;
                        Ok(())
                    }
                }
                PlaceTermKind::Orient { length, width } => {
                    if terms.orient.is_some() {
                        self.report(
                            Diagnostic::error(
                                Code::AxisSetTwice,
                                format!("{name}: 'orient' is written twice"),
                                Label::new(env.file, span),
                            )
                            .with_help("keep one orient term"),
                        );
                        Err(Reported)
                    } else {
                        terms.orient = Some((length.clone(), width.clone(), span));
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
                                    face: face.clone(),
                                    gap: gap.clone(),
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
                                    face: face.clone(),
                                    inset: inset.clone(),
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
                            kind: AxisTermKind::CenterOn { item: item.clone() },
                            span,
                        },
                    ),
                    Ok(_) | Err(_) => Err(Reported),
                },
                PlaceTermKind::CenterBetween { from, to } => {
                    let from_axis = self.anchor_axis(from, env);
                    let to_axis = self.anchor_axis(to, env);
                    match (from_axis, to_axis) {
                        (Ok(a), Ok(b)) if a == b => assign(self, &mut terms, a, AxisTerm { kind: AxisTermKind::CenterBetween { from: from.clone(), to: to.clone() }, span }),
                        (Ok(a), Ok(b)) => Err(self.error(Code::TypeMismatch, format!("'center' needs two parallel planes, found one across '{}' and one across '{}'", a.word(), b.word()), env.file, span, "name two faces across the same axis")),
                        _ => Err(Reported),
                    }
                }
                PlaceTermKind::Spaced { spacing, from, to } => {
                    if label.is_none() {
                        Err(self.error(
                            Code::TypeMismatch,
                            format!("'spaced' needs an array, but '{name}' is a single item"),
                            env.file,
                            span,
                            "declare an array: part name[a, b] or name[3]",
                        ))
                    } else {
                        let from_axis = self.anchor_axis(from, env);
                        let to_axis = self.anchor_axis(to, env);
                        match (from_axis, to_axis) {
                            (Ok(a), Ok(b)) if a == b => assign(
                                self,
                                &mut terms,
                                a,
                                AxisTerm {
                                    kind: AxisTermKind::Spaced {
                                        spacing: spacing.clone(),
                                        from: from.clone(),
                                        to: to.clone(),
                                    },
                                    span,
                                },
                            ),
                            (Ok(_), Ok(_)) => Err(self.error(
                                Code::TypeMismatch,
                                "'spaced' needs two parallel planes".to_owned(),
                                env.file,
                                span,
                                "name two faces across the same axis",
                            )),
                            _ => Err(Reported),
                        }
                    }
                }
                PlaceTermKind::Corners { part, inset } => {
                    if hardware {
                        Err(Reported)
                    } else {
                        match label.and_then(Corner::from_word) {
                            Some(corner) => assign(self, &mut terms, Axis::Across, AxisTerm { kind: AxisTermKind::Corner { part: part.clone(), inset: inset.clone(), face: corner.across_face() }, span })
                                .and_then(|()| assign(self, &mut terms, Axis::Deep, AxisTerm { kind: AxisTermKind::Corner { part: part.clone(), inset: inset.clone(), face: corner.deep_face() }, span })),
                            None => Err(self.error(
                                Code::TypeMismatch,
                                format!("'corners' needs the labels front_left, front_right, back_left and back_right, found '{}'", label.unwrap_or(name)),
                                env.file,
                                span,
                                "declare the array with the four corner labels",
                            )),
                        }
                    }
                }
            };
            failed |= result.is_err();
        }
        if failed { Err(Reported) } else { Ok(terms) }
    }

    // -----------------------------------------------------------------------
    // Sizes
    // -----------------------------------------------------------------------

    /// A part's drawn size along one of its own axes.
    pub(crate) fn drawn_size(&mut self, part: PartId, axis: PartAxis) -> Solved<ExactLength> {
        let slot = part_axis_slot(axis);
        match self.parts.get(part.index()).and_then(|s| s.sizes.get(slot)) {
            Some(Memo::Done(size)) => return Ok(*size),
            Some(Memo::Failed) => return Err(Reported),
            Some(Memo::Busy) => return Err(self.cycle(Frame::Size(part, axis))),
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
                Err(self.error(
                    Code::TypeMismatch,
                    format!(
                        "{name}: its {} works out as {}, which is not more than zero",
                        axis.word(),
                        stave_model::value::format_length(size)
                    ),
                    file,
                    span,
                    "check the sizes it is worked out from",
                ))
            } else {
                Ok(size)
            }
        });
        self.set_size(
            part,
            slot,
            match &result {
                Ok(size) => Memo::Done(*size),
                Err(_) => Memo::Failed,
            },
        );
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
        let Some(body) = self.part_body(part).cloned() else {
            return Err(Reported);
        };
        let Some(env) = self.part_env(part) else {
            return Err(Reported);
        };
        let profile = self.part_profile(part)?;
        let largest = match profile {
            Some(profile) => Some(
                self.profile_stations(profile)?
                    .iter()
                    .map(|(_, d)| *d)
                    .max()
                    .ok_or(Reported)?,
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
                            ));
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
                    Some(Size::Expr(expr)) => self.eval_length(expr, &env),
                    Some(Size::Fit(span)) => self.fit(part, PartAxis::Width, *span),
                    None => {
                        let stock = self.part_stock(part)?;
                        match self.stock_length(stock, StockField::Width)? {
                            Some(width) => Ok(width),
                            None => {
                                let name = self.part_name(part);
                                Err(self.error(
                                    Code::MissingField,
                                    format!("part '{name}' is missing 'width'"),
                                    env.file,
                                    body.span,
                                    "add width: ..., or cut it from stock with a fixed width",
                                ))
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
                    Some(expr) => {
                        let thickness = self.eval_length(expr, &env)?;
                        let stock = self.part_stock(part)?;
                        if let Some(stock_thickness) =
                            self.stock_length(stock, StockField::Thickness)?
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
                            ));
                        }
                        Ok(thickness)
                    }
                    None => {
                        let stock = self.part_stock(part)?;
                        self.stock_length(stock, StockField::Thickness)?
                            .ok_or(Reported)
                    }
                }
            }
        }
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
            let Ok(binding) = self.joint_binding(joint) else {
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
            return Err(self.skip());
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
            !mirrored
                && self
                    .joint_binding(stave_model::ids::JointId::from_usize(index))
                    .is_err()
        })
    }

    fn decide_fit(&mut self, part: PartId, axis: PartAxis, span: Span) -> Solved<ExactLength> {
        let name = self.part_name(part);
        let Some(env) = self.part_env(part) else {
            return Err(Reported);
        };
        let orientation = self.part_orientation(part)?;
        let world = orientation.direction(axis).axis();
        let terms = self.part_terms(part)?;
        let Some(term) = terms.axis(world).cloned() else {
            return Err(self.fit_undecided(
                &name,
                axis,
                env.file,
                span,
                "nothing places it along that axis",
            ));
        };
        let partners = self.joined_into(part);
        if partners.is_empty() {
            return Err(self.fit_undecided(
                &name,
                axis,
                env.file,
                span,
                "it is not the first part of any joint",
            ));
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
                    (None, _) => Err(self.fit_undecided(
                        &name,
                        axis,
                        env.file,
                        span,
                        &format!("no part it is joined into is wholly {low_side} its centre"),
                    )),
                    (_, None) => Err(self.fit_undecided(
                        &name,
                        axis,
                        env.file,
                        span,
                        &format!("no part it is joined into is wholly {high_side} its centre"),
                    )),
                }
            }
            AxisTermKind::Against { .. }
            | AxisTermKind::Flush { .. }
            | AxisTermKind::Corner { .. } => {
                let (known, is_low) = self.known_end(&term, world, &env)?;
                if is_low {
                    match nearest_above(known) {
                        Some(end) => sub(end, known),
                        None => Err(self.fit_undecided(
                            &name,
                            axis,
                            env.file,
                            span,
                            &format!("no part it is joined into is {high_side} it"),
                        )),
                    }
                } else {
                    match nearest_below(known) {
                        Some(end) => sub(known, end),
                        None => Err(self.fit_undecided(
                            &name,
                            axis,
                            env.file,
                            span,
                            &format!("no part it is joined into is {low_side} it"),
                        )),
                    }
                }
            }
            AxisTermKind::Spaced { .. } => Err(self.fit_undecided(
                &name,
                axis,
                env.file,
                span,
                "a spaced part's size must be written",
            )),
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
        );
        Reported
    }

    /// The centre a `center` term puts a part at.
    fn term_centre(&mut self, term: &AxisTerm, axis: Axis, env: &Env<'_>) -> Solved<ExactLength> {
        match &term.kind {
            AxisTermKind::CenterOn { item } => {
                let bounds = match self.eval(item, env)? {
                    Value::Part(part) => self.drawn_box(part)?,
                    Value::Region(region) => self.region_value(region)?.bounds,
                    other => {
                        return Err(self.error(
                            Code::TypeMismatch,
                            format!(
                                "'center' needs a part or a region, found {}",
                                other.description()
                            ),
                            env.file,
                            item.span,
                            "name a part or region",
                        ));
                    }
                };
                midpoint(bounds.min(axis), bounds.max(axis)).ok_or(Reported)
            }
            AxisTermKind::CenterBetween { from, to } => {
                let a = self.eval_anchor(from, env)?;
                let b = self.eval_anchor(to, env)?;
                midpoint(a.position, b.position).ok_or(Reported)
            }
            _ => Err(Reported),
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
            None => Err(self.error(
                Code::TypeMismatch,
                format!("'{head}' needs a face, found a mid-plane"),
                env.file,
                expr.span,
                "use center(...) for a mid-plane",
            )),
        }
    }

    fn optional_length(&mut self, expr: &Option<RExpr>, env: &Env<'_>) -> Solved<ExactLength> {
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
                let gap = self.optional_length(gap, env)?;
                if face.is_positive() {
                    Ok((add(anchor.position, gap)?, true))
                } else {
                    Ok((sub(anchor.position, gap)?, false))
                }
            }
            AxisTermKind::Flush { face, inset } => {
                let (anchor, face) = self.face_anchor(face, env, "flush")?;
                let inset = self.optional_length(inset, env)?;
                if face.is_positive() {
                    Ok((sub(anchor.position, inset)?, false))
                } else {
                    Ok((add(anchor.position, inset)?, true))
                }
            }
            AxisTermKind::Corner { part, inset, face } => {
                let other = self.eval_part(part, env)?;
                let position = self.part_anchor(other, *face)?.position;
                let inset = self.optional_length(inset, env)?;
                if face.is_positive() {
                    Ok((sub(position, inset)?, false))
                } else {
                    Ok((add(position, inset)?, true))
                }
            }
            _ => Err(Reported),
        }
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
                    return Err(Reported);
                };
                let spacing = match self.eval(spacing, env)? {
                    Value::Spacing(spacing) => spacing,
                    Value::Word(Word::Evenly) => Spacing::Evenly,
                    Value::Word(Word::Ends) => Spacing::Ends(ExactLength::ZERO),
                    other => {
                        return Err(self.error(
                            Code::TypeMismatch,
                            format!("'spaced' needs a spacing, found {}", other.description()),
                            env.file,
                            spacing.span,
                            "write evenly, ends, ends(inset: d), pitch(d) or gap(d)",
                        ));
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
                if forward {
                    add(a, offset)
                } else {
                    sub(sub(a, offset)?, own)
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Positions
    // -----------------------------------------------------------------------

    /// The low end of a part's drawn box along a world axis.
    pub(crate) fn part_min(&mut self, part: PartId, axis: Axis) -> Solved<ExactLength> {
        let slot = axis_slot(axis);
        match self.parts.get(part.index()).and_then(|s| s.mins.get(slot)) {
            Some(Memo::Done(v)) => return Ok(*v),
            Some(Memo::Failed) => return Err(Reported),
            Some(Memo::Busy) => return Err(self.cycle(Frame::Position(part, axis))),
            _ => {}
        }
        self.set_min(part, slot, Memo::Busy);
        let at = self.term_location(part, axis);
        self.enter(Frame::Position(part, axis), at);
        let result = self.compute_min(part, axis);
        self.leave();
        self.set_min(
            part,
            slot,
            match &result {
                Ok(v) => Memo::Done(*v),
                Err(_) => Memo::Failed,
            },
        );
        result
    }

    fn term_location(&mut self, part: PartId, axis: Axis) -> Option<(FileId, Span)> {
        let env = self.part_env(part)?;
        let terms = match self.parts.get(part.index()).map(|s| &s.terms) {
            Some(Memo::Done(terms)) => terms.clone(),
            _ => {
                return self
                    .part_body(part)
                    .map(|b| (env.file, b.place_span.unwrap_or(b.span)));
            }
        };
        terms.axis(axis).map(|t| (env.file, t.span))
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
            return Err(Reported);
        };
        let Some(term) = terms.axis(axis).cloned() else {
            return Err(Reported);
        };
        let size = self.size_along(part, axis)?;
        let siblings = match &term.kind {
            AxisTermKind::Spaced { .. } => Some(self.part_siblings(part, axis)?),
            _ => None,
        };
        self.term_low(&term, axis, size, &env, siblings.as_ref())
    }

    fn part_siblings(&mut self, part: PartId, axis: Axis) -> Solved<Siblings> {
        let Some(inst) = self.model.parts.get(part.index()).cloned() else {
            return Err(Reported);
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
        Ok(Siblings { index, sizes })
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
            return Err(Reported);
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
            Some(Memo::Failed) => return Err(Reported),
            Some(Memo::Busy) => return Err(self.cycle(Frame::Region(region))),
            _ => {}
        }
        if let Some(slot) = self.regions.get_mut(region.index()) {
            *slot = Memo::Busy;
        }
        let Some(inst) = self.model.regions.get(region.index()).cloned() else {
            return Err(Reported);
        };
        let Some(def) = self.model.defs.region(inst.def).cloned() else {
            return Err(Reported);
        };
        self.enter(Frame::Region(region), Some((def.decl.file, def.decl.span)));
        let result = self.compute_region(region, &inst, &def);
        self.leave();
        if let Some(slot) = self.regions.get_mut(region.index()) {
            *slot = match &result {
                Ok(v) => Memo::Done(*v),
                Err(_) => Memo::Failed,
            };
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
            return Err(self.skip());
        }
        if let Some(source) = inst.mirror_of {
            let Some(assembly) = inst.assembly else {
                return Err(Reported);
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
                    return Err(self.error(
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
                    ));
                }
                let value = list.get(index).copied().ok_or(Reported)?;
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
            other => Err(self.error(
                Code::TypeMismatch,
                format!(
                    "'{}' needs a region, found {}",
                    def.decl.name,
                    other.description()
                ),
                def.decl.file,
                def.value.span,
                "write opening(...), split(...) or cover(...)",
            )),
        }
    }
}
