//! Splices (grammar, "Splices"): a part whose `lengths:` says
//! `spliced(by: kind, at: [...], from: end)` is made of pieces joined end
//! to end, one splice joint of `kind` between each two.
//!
//! The pieces are derived, never written. Placement, joints and hardware
//! see one part; once every joint, piece of hardware and mirrored assembly
//! is done, the part is cut into its pieces here:
//!
//! 1. each piece stands in for a part under an id of its own, as a board
//!    does (see [`crate::boards`]), so the splice kind's rules, `adds:`,
//!    made parts and cuts work on pieces exactly as on parts;
//! 2. the cuts other joints and hardware made on the part are handed to
//!    the piece they fall on, in that piece's frame. A cut that reaches
//!    into a splice, where both pieces' wood is the splice's, is
//!    `error[wall]`: a splice needs its wood whole;
//! 3. every piece must fit its stock's `max_length`;
//! 4. the pieces become parts of the solved design (the first keeps the
//!    part's id, the others are appended like made parts) and each splice
//!    a joint of its own ([`JointOrigin::Splice`]), so the checks, the
//!    scene, focus mode and the cut list treat them like any other parts
//!    and joints. [`Solver::rebind_pieces`] then points what named the
//!    part (joint pairs, hardware, assemblies, scenes) at the right piece.

use std::collections::BTreeMap;

use stave_lang::ast::{BaseKind, Span};
use stave_lang::placement::{Axis, Face, PartAxis};
use stave_lang::units::{ExactLength, Length};
use stave_lang::{Code, Diagnostic, FileId, Label};
use stave_model::defs::{DefId, StockField};
use stave_model::ids::{JointId, PartId};
use stave_model::nonempty::NonEmpty;
use stave_model::solved::{
    self, BoardIndex, BodyCarrier, Contact, Cut, CutKind, CutPlace, CutSource, Depth, End, Joint,
    JointOrigin, JointPair, PartOrigin, PieceIndex, RoleBinding, SolvedDesign, WorldBox,
};
use stave_model::value::{KindValue, Splicing, Value};
use stave_schema::CutOperation;
use stave_schema::kinds::RoleType;
use stave_schema::words::StockForm;

use crate::boards::{BOARD_ID_BASE, BoardPart, Division};
use crate::build::MadeInfo;
use crate::eval::{Env, KindEnv};
use crate::joints::{CutAt, GeomTable, PairWork, PartGeom, local_slot, round_length};
use crate::{Failure, Fault, Solved, Solver};

/// A part cut into pieces: what the solved design needs to point the
/// part's other connections at the right piece.
#[derive(Debug, Clone)]
pub(crate) struct Split {
    /// The part, which is also its first piece.
    pub whole: PartId,
    /// The part's name as written.
    pub name: String,
    /// The pieces, from the end `from:` names.
    pub pieces: Vec<PartId>,
    /// The splices, from the end `from:` names.
    pub splices: Vec<JointId>,
}

/// The tools of the work: the tables the build fills.
pub(crate) struct Tables<'t> {
    pub geoms: &'t mut GeomTable,
    pub cuts: &'t mut Vec<Vec<Cut>>,
    pub memberships: &'t mut Vec<Vec<JointId>>,
    pub made: &'t mut Vec<MadeInfo>,
    pub joints: &'t mut Vec<Option<Joint>>,
}

/// One piece while it is worked out.
struct Piece {
    /// Its stand-in id.
    stand_in: PartId,
    /// Its cut range along the part's length, in the part's own frame.
    low: ExactLength,
    high: ExactLength,
}

/// The splice kind, bound.
struct Method {
    value: KindValue,
    settings: Vec<(String, Value)>,
    glue: bool,
    methods: Vec<String>,
}

/// Where the problems of one part are reported.
#[derive(Clone, Copy)]
struct Place {
    file: FileId,
    span: Span,
    /// `false` for a mirror image: its source reports.
    report: bool,
}

fn axis_slot(axis: Axis) -> usize {
    match axis {
        Axis::Across => 0,
        Axis::Up => 1,
        Axis::Deep => 2,
    }
}

/// A cut's name in messages: its kind's name for it, or its operation's
/// word (grammar, "Named cuts").
fn cut_words(cut: &Cut) -> String {
    match &cut.name {
        Some(name) => name.as_str().replace('_', " "),
        None => match cut.kind.operation() {
            CutOperation::Pocket => "mortise".to_owned(),
            CutOperation::Drill => "hole".to_owned(),
            CutOperation::Notch => "notch".to_owned(),
            CutOperation::Slot => "slot".to_owned(),
            CutOperation::Tenon
            | CutOperation::Spigot
            | CutOperation::Round
            | CutOperation::DovetailSocket
            | CutOperation::DovetailTail
            | CutOperation::Outline => "cut".to_owned(),
        },
    }
}

fn depth_or(depth: Depth, through: Length) -> Length {
    match depth {
        Depth::Length(length) => length,
        Depth::Through => through,
    }
}

/// How far a cut on an end reaches along the part's length.
fn end_reach(kind: &CutKind, length: Length) -> Length {
    match kind {
        CutKind::Notch { depth }
        | CutKind::Slot { depth, .. }
        | CutKind::Pocket { depth, .. }
        | CutKind::Drill { depth, .. }
        | CutKind::Outline { depth, .. } => depth_or(*depth, length),
        CutKind::Tenon { length: reach, .. } | CutKind::Spigot { length: reach, .. } => *reach,
        CutKind::DovetailSocket { depth, .. } | CutKind::DovetailTail { depth, .. } => *depth,
        CutKind::Round { .. } => length,
    }
}

/// The stretch of the part's length a cut takes, in the part's frame
/// (`length` is the part's cut length). A cut measured on the whole part
/// (an operation on `side` other than an outline) takes all of it.
pub(crate) fn along_extent(cut: &Cut, length: Length) -> (Length, Length) {
    let whole = (Length::ZERO, length);
    match &cut.place {
        CutPlace::Crossing { footprint, .. } | CutPlace::Member { footprint, .. } => {
            (footprint.min.length, footprint.max.length)
        }
        CutPlace::End { .. } => {
            if cut.face.axis != PartAxis::Length {
                return whole;
            }
            let reach = end_reach(&cut.kind, length);
            match cut.face.end {
                End::Start => (Length::ZERO, reach),
                End::End => (minus(length, reach), length),
            }
        }
        CutPlace::Side { along_from, .. } => match &cut.kind {
            CutKind::Outline { marks, .. } => {
                let low = marks.iter().map(|m| m.along).min().unwrap_or(Length::ZERO);
                let high = marks.iter().map(|m| m.along).max().unwrap_or(length);
                match along_from {
                    End::Start => (low, high),
                    End::End => (minus(length, high), minus(length, low)),
                }
            }
            CutKind::Notch { .. }
            | CutKind::Slot { .. }
            | CutKind::Pocket { .. }
            | CutKind::Tenon { .. }
            | CutKind::Spigot { .. }
            | CutKind::Round { .. }
            | CutKind::Drill { .. }
            | CutKind::DovetailSocket { .. }
            | CutKind::DovetailTail { .. } => whole,
        },
    }
}

fn minus(a: Length, b: Length) -> Length {
    a.checked_sub(b).unwrap_or(Length::ZERO)
}

/// A cut of the whole part moved onto a piece whose frame starts `offset`
/// along the part's length and is `piece` long (`whole` is the part's cut
/// length).
fn shifted(mut cut: Cut, offset: Length, piece: Length, whole: Length) -> Solved<Cut> {
    let shift = |value: Length| value.checked_sub(offset).ok_or(Fault::Overflow);
    match &mut cut.place {
        CutPlace::Crossing { footprint, .. } => {
            footprint.min.length = shift(footprint.min.length)?;
            footprint.max.length = shift(footprint.max.length)?;
        }
        CutPlace::Member {
            footprint, shape, ..
        } => {
            footprint.min.length = shift(footprint.min.length)?;
            footprint.max.length = shift(footprint.max.length)?;
            shape.min.length = shift(shape.min.length)?;
            shape.max.length = shift(shape.max.length)?;
        }
        CutPlace::End { .. } => {}
        CutPlace::Side { along_from, .. } => {
            // Marks measured from the start move with the piece's start;
            // marks measured from the end, with its end.
            let by = match along_from {
                End::Start => offset,
                End::End => whole
                    .checked_sub(offset)
                    .and_then(|x| x.checked_sub(piece))
                    .ok_or(Fault::Overflow)?,
            };
            if let CutKind::Outline { marks, .. } = &mut cut.kind {
                for mark in marks {
                    mark.along = mark.along.checked_sub(by).ok_or(Fault::Overflow)?;
                }
            }
        }
    }
    Ok(cut)
}

/// The face of a world axis on its positive or negative side.
fn face_of(axis: Axis, positive: bool) -> Face {
    match (axis, positive) {
        (Axis::Across, true) => Face::Right,
        (Axis::Across, false) => Face::Left,
        (Axis::Up, true) => Face::Top,
        (Axis::Up, false) => Face::Bottom,
        (Axis::Deep, true) => Face::Back,
        (Axis::Deep, false) => Face::Front,
    }
}

impl Solver<'_> {
    /// `warning[missing-field]` once for each stock a declared part is cut
    /// from that doesn't say how long its boards or sheets come
    /// (`max_length`), at the stock: without it no part's length can be
    /// checked (grammar, "stock"). Round stock is left out.
    pub(crate) fn warn_missing_max_length(&mut self) {
        let mut stocks = std::collections::BTreeSet::new();
        for index in 0..self.model.parts.len() {
            let part = PartId::from_usize(index);
            let (file, span) = self.part_place(part);
            let source = self.mirror_source(part);
            if !matches!(self.settled(source, file, span), Some(None)) {
                continue;
            }
            let stock = self.part_stock(part);
            if let Some(stock) = self.settled(stock, file, span) {
                stocks.insert(stock);
            }
        }
        for stock in stocks {
            let Some(def) = self.model.defs.stock(stock) else {
                continue;
            };
            if def.max_length.is_some() {
                continue;
            }
            let (what, example) = match def.form {
                StockForm::Solid => ("boards", "2400mm"),
                StockForm::Plywood | StockForm::Mdf => ("sheets", "2440mm"),
                StockForm::Dowel => continue,
            };
            let name = def.decl.name.clone();
            let label = Label::new(def.decl.file, def.decl.name_span);
            self.report(
                Diagnostic::warning(
                    Code::MissingField,
                    format!("{name} doesn't say how long its {what} are, so Stave can't check part lengths"),
                    label,
                )
                .with_help(format!(
                    "add the longest {} you can buy to {name}, for example max_length: {example}",
                    match def.form {
                        StockForm::Plywood | StockForm::Mdf => "sheet",
                        StockForm::Solid | StockForm::Dowel => "board",
                    }
                )),
            );
        }
    }

    /// Where a part's `lengths:` is written (its body source's).
    fn lengths_place(&self, source: PartId) -> Option<(FileId, Span)> {
        let body = self.part_body(source)?;
        let env = self.part_env(source)?;
        Some((env.file, body.lengths.as_ref()?.span))
    }

    /// A suggestion for a part longer than its stock: where to splice it
    /// so every piece comes within `max_length` (before the splices add to
    /// them), measured from the start of its length.
    pub(crate) fn splice_suggestion(
        &mut self,
        part: PartId,
        drawn: ExactLength,
        max_length: ExactLength,
    ) -> Option<String> {
        let orientation = self.part_orientation(part).ok()?;
        let direction = orientation.direction(PartAxis::Length);
        let from = face_of(direction.axis(), !direction.is_positive());
        let ratio = drawn.checked_ratio(max_length)?;
        let pieces = if ratio.is_integer() {
            ratio.floor()
        } else {
            ratio.floor().checked_add(1)?
        };
        let pieces = i64::try_from(pieces.max(2)).ok()?;
        let drawn = drawn.to_nearest_length()?;
        let millimetre = Length::from_mm(1).nm();
        let mut at = Vec::new();
        for k in 1..pieces {
            let nm = drawn.nm().checked_mul(k)?.checked_div(pieces)?;
            at.push(
                Length::from_nm(nm.checked_div(millimetre)?.checked_mul(millimetre)?).to_string(),
            );
        }
        let at = match at.as_slice() {
            [one] => one.clone(),
            _ => format!("[{}]", at.join(", ")),
        };
        Some(format!(
            "lengths: spliced(by: <an end joint>, at: {at}, from: {})",
            from.word()
        ))
    }

    /// Every spliced part cut into its pieces, with its splices. Runs once
    /// every joint, piece of hardware and mirrored assembly is done.
    pub(crate) fn make_pieces(&mut self, tables: &mut Tables<'_>) -> Vec<Split> {
        let mut splits = Vec::new();
        for index in 0..self.model.parts.len() {
            let part = PartId::from_usize(index);
            let made_before = tables.made.len();
            let joints_before = tables.joints.len();
            let saved_cuts = tables.cuts.get(index).cloned();
            match self.part_pieces(part, tables) {
                Ok(Some(split)) => splits.push(split),
                Ok(None) => {}
                Err(failure) => {
                    let (file, span) = self.part_place(part);
                    let _ = self.settle(failure, file, span);
                    // Nothing half-made is kept.
                    tables.made.truncate(made_before);
                    tables.joints.truncate(joints_before);
                    if let (Some(saved), Some(slot)) = (saved_cuts, tables.cuts.get_mut(index)) {
                        *slot = saved;
                    }
                }
            }
            let stand_ins: Vec<PartId> = self
                .boards
                .iter()
                .filter(|(_, board)| board.division == Division::Piece)
                .map(|(id, _)| *id)
                .collect();
            for stand_in in stand_ins {
                self.boards.remove(&stand_in);
                tables.geoms.boards.remove(&stand_in);
            }
        }
        splits
    }

    /// The splicing a part's `lengths:` says, with the part it is written
    /// on; `None` when it says none.
    fn splicing(&mut self, source: PartId) -> Solved<Option<Splicing>> {
        let (Some(expr), Some(env)) = (
            self.part_body(source).and_then(|b| b.lengths.clone()),
            self.part_env(source),
        ) else {
            return Ok(None);
        };
        match self.eval(&expr, &env)? {
            Value::Splices(splicing) => Ok(Some(splicing)),
            other => Err(self
                .error(
                    Code::TypeMismatch,
                    format!(
                        "'lengths' needs spliced(by: ..., at: ..., from: ...), found {}",
                        other.description()
                    ),
                    env.file,
                    expr.span,
                    "write lengths: spliced(by: kind_name, at: 1200mm, from: left)",
                )
                .into()),
        }
    }

    /// The pieces of one part, or `None` if it isn't spliced.
    fn part_pieces(&mut self, part: PartId, tables: &mut Tables<'_>) -> Solved<Option<Split>> {
        let source = self.body_source(part)?;
        let Some(splicing) = self.splicing(source)? else {
            return Ok(None);
        };
        let Some((file, span)) = self.lengths_place(source) else {
            return Err(Fault::Internal("where a part's lengths are written").into());
        };
        let place = Place {
            file,
            span,
            report: source == part,
        };
        let Some(geom) = tables.geoms.get(part).cloned() else {
            return Err(Fault::Earlier.into());
        };
        let name = self.part_name(part);
        self.check_spliceable(part, &name, place)?;
        let method = self.splice_method(&splicing.by, place)?;

        // Where the splices are, on the drawn length, measured from the
        // start of the part's own length.
        let source_orientation = self.part_orientation(source)?;
        let direction = source_orientation.direction(PartAxis::Length);
        if splicing.from.axis() != direction.axis() {
            let ends = [
                face_of(direction.axis(), false).word(),
                face_of(direction.axis(), true).word(),
            ];
            return Err(self.fail(
                place,
                Code::TypeMismatch,
                format!(
                    "'from' needs an end of {name}, but {} is one of its sides",
                    splicing.from.word()
                ),
                format!(
                    "{name}'s length runs {}, so its ends are {} and {}: write from: {}",
                    direction.axis().word(),
                    ends[0],
                    ends[1],
                    ends[0]
                ),
            ));
        }
        let from_start = splicing.from.is_positive() != direction.is_positive();
        let slot = local_slot(PartAxis::Length);
        let drawn = geom
            .drawn
            .get(slot)
            .copied()
            .ok_or(Fault::Internal("a part's length"))?;
        let start = geom
            .start
            .get(slot)
            .copied()
            .ok_or(Fault::Internal("a part's length"))?;
        let total = geom.cut_size(PartAxis::Length)?;
        let mut positions: Vec<ExactLength> = splicing.at.iter().copied().collect();
        positions.sort();
        for (i, at) in positions.iter().enumerate() {
            if *at <= ExactLength::ZERO || *at >= drawn {
                return Err(self.fail(
                    place,
                    Code::OutOfRange,
                    format!(
                        "the splice at {} is not inside {name}, which is {} long",
                        stave_model::value::format_length(*at),
                        stave_model::value::format_length(drawn)
                    ),
                    format!(
                        "measure each splice from {name}'s {} end: more than 0mm and less than {}",
                        splicing.from.word(),
                        stave_model::value::format_length(drawn)
                    ),
                ));
            }
            if i > 0 && positions.get(i - 1) == Some(at) {
                return Err(self.fail(
                    place,
                    Code::DuplicateName,
                    format!(
                        "two splices of {name} are at {}",
                        stave_model::value::format_length(*at)
                    ),
                    "give each splice its own place",
                ));
            }
        }
        // The cut-frame boundaries, from the `from` end.
        let mut boundaries = vec![if from_start { ExactLength::ZERO } else { total }];
        for at in &positions {
            let local = if from_start {
                start.checked_add(*at)
            } else {
                start.checked_add(drawn).and_then(|x| x.checked_sub(*at))
            }
            .ok_or(Fault::Overflow)?;
            boundaries.push(local);
        }
        boundaries.push(if from_start { total } else { ExactLength::ZERO });
        let mut pieces = self.lay_out_pieces(part, &geom, &boundaries, tables.geoms)?;

        // The splices' rules and what they add.
        let first_joint = tables.joints.len();
        let mut works = Vec::new();
        let mut extra: BTreeMap<PartId, [ExactLength; 2]> = BTreeMap::new();
        for splice in 0..positions.len() {
            let joint = JointId::from_usize(first_joint + splice);
            let (Some(a), Some(b)) = (pieces.get(splice), pieces.get(splice + 1)) else {
                return Err(Fault::Internal("the pieces either side of a splice").into());
            };
            let parts = vec![a.stand_in, b.stand_in];
            let work =
                self.splice_pass_a(joint, &name, splice, &method, parts, place, &mut extra)?;
            works.push(work);
        }
        for piece in &mut pieces {
            let Some([low_add, high_add]) = extra.get(&piece.stand_in).copied() else {
                continue;
            };
            let Some(mut piece_geom) = tables.geoms.get(piece.stand_in).cloned() else {
                return Err(Fault::Internal("a piece of a spliced part").into());
            };
            if let (Some(s), Some(e)) =
                (piece_geom.start.get_mut(slot), piece_geom.end.get_mut(slot))
            {
                *s = s.checked_add(low_add).ok_or(Fault::Overflow)?;
                *e = e.checked_add(high_add).ok_or(Fault::Overflow)?;
            }
            piece.low = piece.low.checked_sub(low_add).ok_or(Fault::Overflow)?;
            piece.high = piece.high.checked_add(high_add).ok_or(Fault::Overflow)?;
            let (world_low, world_high) = to_world(&geom, piece.low, piece.high)?;
            let world = geom.orientation.direction(PartAxis::Length).axis();
            piece_geom.cut_box = piece_geom.cut_box.with(world, world_low, world_high);
            tables.geoms.set(piece.stand_in, piece_geom);
        }

        // Every piece within the stock's length.
        let stock = self.part_stock(part)?;
        if let Some(max_length) = self.stock_length(stock, StockField::MaxLength)? {
            for piece in &pieces {
                let length = piece.high.checked_sub(piece.low).ok_or(Fault::Overflow)?;
                if length > max_length {
                    let piece_name = self.part_name(piece.stand_in);
                    let stock_name = self.stock_name(stock);
                    return Err(self.fail(
                        place,
                        Code::TypeMismatch,
                        format!(
                            "{piece_name}: its cut length works out as {}, which is more than its stock's max_length of {}",
                            stave_model::value::format_length(length),
                            stave_model::value::format_length(max_length)
                        ),
                        format!(
                            "stock '{stock_name}' comes no longer than {}, and each piece includes the wood its splices add: move the splices with at:, add another splice, or buy longer stock",
                            stave_model::value::format_length(max_length)
                        ),
                    ));
                }
            }
        }

        // The part's other cuts, each onto the piece it falls on.
        let foreign = tables
            .cuts
            .get_mut(part.index())
            .map(std::mem::take)
            .unwrap_or_default();
        let mut piece_cuts: BTreeMap<PartId, Vec<Cut>> = BTreeMap::new();
        self.hand_out_cuts(
            &name,
            &splicing,
            from_start,
            start,
            total,
            &pieces,
            foreign,
            place,
            &mut piece_cuts,
        )?;

        // The splices' made parts, cuts and steps.
        let mut splice_joints = Vec::new();
        for (splice, mut work) in works.into_iter().enumerate() {
            let joint = JointId::from_usize(first_joint + splice);
            let solved = self.splice_pass_b(
                joint,
                part,
                &name,
                splice,
                &method,
                &mut work,
                tables,
                &mut piece_cuts,
            )?;
            splice_joints.push(solved);
        }

        // The pieces become parts: the first keeps the part's id.
        let assembly = self.model.parts.get(part.index()).and_then(|p| p.assembly);
        let mut ids = BTreeMap::new();
        let mut piece_ids = Vec::new();
        for (number, piece) in pieces.iter().enumerate() {
            let index = u32::try_from(number + 1)
                .ok()
                .and_then(PieceIndex::new)
                .ok_or(Fault::Overflow)?;
            let id = if number == 0 {
                part
            } else {
                let id = PartId::from_usize(self.model.parts.len() + tables.made.len());
                tables.made.push(MadeInfo {
                    origin: PartOrigin::Piece { part, index },
                    assembly,
                    name: format!("{name}.piece[{}]", index.get()),
                    stock,
                    note: None,
                });
                id
            };
            ids.insert(piece.stand_in, id);
            piece_ids.push(id);
        }
        let real = |id: PartId| -> PartId { ids.get(&id).copied().unwrap_or(id) };
        for piece in &pieces {
            let id = real(piece.stand_in);
            let geom = tables
                .geoms
                .get(piece.stand_in)
                .cloned()
                .ok_or(Fault::Internal("a piece of a spliced part"))?;
            tables.geoms.set(id, geom);
            if tables.cuts.len() <= id.index() {
                tables.cuts.resize(id.index() + 1, Vec::new());
                tables.memberships.resize(id.index() + 1, Vec::new());
            }
            let cuts: Vec<Cut> = piece_cuts
                .remove(&piece.stand_in)
                .unwrap_or_default()
                .into_iter()
                .map(|cut| remap_cut(cut, &real))
                .collect();
            if let Some(slot) = tables.cuts.get_mut(id.index()) {
                *slot = cuts;
            }
        }
        // Cuts the splices made on their made parts name pieces too.
        for (stand_in_or_made, cuts) in piece_cuts {
            if let Some(slot) = tables.cuts.get_mut(real(stand_in_or_made).index()) {
                slot.extend(cuts.into_iter().map(|cut| remap_cut(cut, &real)));
            }
        }
        let mut splices = Vec::new();
        for mut joint in splice_joints {
            for pair in &mut joint.pairs {
                for role in &mut pair.roles {
                    role.part = real(role.part);
                }
                for extension in &mut pair.extensions {
                    extension.part = real(extension.part);
                }
                for step in &mut pair.insert {
                    step.part = real(step.part);
                }
                for id in &mut pair.cut_order {
                    *id = real(*id);
                }
                for role in &pair.roles {
                    if let Some(list) = tables.memberships.get_mut(role.part.index())
                        && !list.contains(&joint.id)
                    {
                        list.push(joint.id);
                    }
                }
            }
            splices.push(joint.id);
            tables.joints.push(Some(joint));
        }
        Ok(Some(Split {
            whole: part,
            name,
            pieces: piece_ids,
            splices,
        }))
    }

    /// Reports `message` at a part's `lengths:` (unless it is a mirror
    /// image, whose source reports).
    fn fail(
        &mut self,
        place: Place,
        code: Code,
        message: String,
        help: impl Into<String>,
    ) -> Failure {
        if place.report {
            self.error(code, message, place.file, place.span, help)
                .into()
        } else {
            Fault::Earlier.into()
        }
    }

    /// Only a solid board can be spliced.
    fn check_spliceable(&mut self, part: PartId, name: &str, place: Place) -> Solved<()> {
        if self.part_profile(part)?.is_some() {
            return Err(self.fail(
                place,
                Code::TypeMismatch,
                format!("{name} is turned, so it can't be spliced"),
                "a turned part is made from one blank: remove lengths:, or buy a longer blank",
            ));
        }
        let stock = self.part_stock(part)?;
        let form = self
            .model
            .defs
            .stock(stock)
            .map(|s| s.form)
            .ok_or(Fault::Internal("a stock's declaration"))?;
        let stock_name = self.stock_name(stock);
        match form {
            StockForm::Solid => Ok(()),
            StockForm::Plywood | StockForm::Mdf => Err(self.fail(
                place,
                Code::TypeMismatch,
                format!("{name} is cut from a sheet, so it can't be spliced"),
                format!(
                    "a sheet isn't spliced end to end: choose a sheet longer than '{stock_name}', or make the part from several parts"
                ),
            )),
            StockForm::Dowel => Err(self.fail(
                place,
                Code::TypeMismatch,
                format!("{name} is cut from round stock, so it can't be spliced"),
                format!("buy '{stock_name}' in a longer length, or remove lengths:"),
            )),
        }
    }

    /// The splice kind: a joint kind with two roles that are parts.
    fn splice_method(&mut self, value: &KindValue, place: Place) -> Solved<Method> {
        if self.model.defs.is_poisoned(DefId::Kind(value.kind)) {
            return Err(self.skip().into());
        }
        let Some(kind) = self.model.defs.kind(value.kind).cloned() else {
            return Err(Fault::Internal("a kind's declaration").into());
        };
        if kind.base != BaseKind::Joint {
            return Err(self.fail(
                place,
                Code::TypeMismatch,
                format!("'{}' is not a joint kind", value.name),
                "pieces are spliced by joint kinds whose two roles are parts: spliced(by: kind_name, ...)",
            ));
        }
        let roles: Vec<RoleType> = kind.roles.iter().map(|r| r.ty).collect();
        let pairwise = matches!(
            roles.as_slice(),
            [first, second] if *first != RoleType::Region && *second != RoleType::Region
        );
        if !pairwise {
            return Err(self.fail(
                place,
                Code::TypeMismatch,
                format!(
                    "'{}' can't splice pieces: it has {} roles",
                    value.name,
                    roles.len()
                ),
                "a splice joins two pieces, so its kind has two roles that are parts",
            ));
        }
        let mut glue = true;
        let mut methods = Vec::new();
        let mut settings = Vec::new();
        for (name, setting) in &value.settings {
            match name.as_str() {
                "glue" => glue = *setting == Value::Bool(true),
                "methods" => {
                    if let Value::List(items) = setting {
                        for item in items {
                            if let Value::Recipe(recipe) = item
                                && let Some(def) = self.model.defs.recipe(*recipe)
                            {
                                methods.push(def.decl.name.clone());
                            }
                        }
                    }
                }
                _ => settings.push((name.clone(), setting.clone())),
            }
        }
        Ok(Method {
            value: value.clone(),
            settings,
            glue,
            methods,
        })
    }

    /// Cuts a part into pieces between `boundaries` (in its cut frame,
    /// from the `from` end), each standing in for a part.
    fn lay_out_pieces(
        &mut self,
        part: PartId,
        geom: &PartGeom,
        boundaries: &[ExactLength],
        geoms: &mut GeomTable,
    ) -> Solved<Vec<Piece>> {
        let slot = local_slot(PartAxis::Length);
        let drawn_start = geom
            .start
            .get(slot)
            .copied()
            .ok_or(Fault::Internal("a part's length"))?;
        let drawn_end = drawn_start
            .checked_add(geom.drawn.get(slot).copied().ok_or(Fault::Overflow)?)
            .ok_or(Fault::Overflow)?;
        let world = geom.orientation.direction(PartAxis::Length).axis();
        let first = self
            .boards
            .keys()
            .next_back()
            .map_or(BOARD_ID_BASE, |last| last.get().saturating_add(1));
        let mut pieces = Vec::new();
        for (k, pair) in boundaries.windows(2).enumerate() {
            let [a, b] = pair else {
                return Err(Fault::Internal("the pieces of a spliced part").into());
            };
            let (low, high) = if a <= b { (*a, *b) } else { (*b, *a) };
            // The drawn stretch: what isn't added at the part's own ends.
            let drawn_low = low.max(drawn_start);
            let drawn_high = high.min(drawn_end);
            let mut piece_geom = geom.clone();
            if let (Some(d), Some(s), Some(e)) = (
                piece_geom.drawn.get_mut(slot),
                piece_geom.start.get_mut(slot),
                piece_geom.end.get_mut(slot),
            ) {
                *d = drawn_high.checked_sub(drawn_low).ok_or(Fault::Overflow)?;
                *s = drawn_low.checked_sub(low).ok_or(Fault::Overflow)?;
                *e = high.checked_sub(drawn_high).ok_or(Fault::Overflow)?;
            }
            let (cut_low, cut_high) = to_world(geom, low, high)?;
            let (seen_low, seen_high) = to_world(geom, drawn_low, drawn_high)?;
            piece_geom.cut_box = geom.cut_box.with(world, cut_low, cut_high);
            piece_geom.drawn_box = geom.drawn_box.with(world, seen_low, seen_high);
            let k = u32::try_from(k).map_err(|_| Fault::Overflow)?;
            let id = PartId::from_usize(
                usize::try_from(first.saturating_add(k)).map_err(|_| Fault::Overflow)?,
            );
            let index = k
                .checked_add(1)
                .and_then(BoardIndex::new)
                .ok_or(Fault::Overflow)?;
            let mut min = [ExactLength::ZERO; 3];
            for each in Axis::ALL {
                if let Some(slot) = min.get_mut(axis_slot(each)) {
                    *slot = piece_geom.drawn_box.min(each);
                }
            }
            self.boards.insert(
                id,
                BoardPart {
                    part,
                    division: Division::Piece,
                    index,
                    orientation: geom.orientation,
                    drawn: piece_geom.drawn,
                    min,
                },
            );
            geoms.set(id, piece_geom);
            pieces.push(Piece {
                stand_in: id,
                low,
                high,
            });
        }
        Ok(pieces)
    }

    /// Pass A of one splice: its rules, and what it adds to each piece.
    #[allow(clippy::too_many_arguments)]
    fn splice_pass_a(
        &mut self,
        joint: JointId,
        name: &str,
        splice: usize,
        method: &Method,
        parts: Vec<PartId>,
        place: Place,
        extra: &mut BTreeMap<PartId, [ExactLength; 2]>,
    ) -> Solved<SpliceWork> {
        let kind_id = method.value.kind;
        let Some(kind) = self.model.defs.kind(kind_id).cloned() else {
            return Err(Fault::Internal("a kind's declaration").into());
        };
        let roles: Vec<Value> = parts.iter().map(|p| Value::Part(*p)).collect();
        let env = KindEnv::new(kind_id, roles, method.settings.clone(), kind.params.len());
        let Some(ctx) = self.module_ctx(DefId::Kind(kind_id)) else {
            return Err(Fault::Internal("the file an item is declared in").into());
        };
        let kind_env = Env {
            ctx,
            file: kind.decl.file,
            kind: Some(&env),
            array: None,
        };
        let joint_name = format!("{name}.splice[{}]", splice + 1);
        let source = CutSource::Joint { joint, pair: 0 };
        let mut requires = Vec::new();
        let mut failure: Option<Failure> = None;
        for (index, require) in kind.requires.iter().enumerate() {
            if self.defer_wall_rule(source, kind_id, index, require, &kind_env)? {
                continue;
            }
            let holds = self.eval_bool(&require.condition, &kind_env)?;
            let message = self.format_text(&require.message, &kind_env)?;
            if !holds && place.report {
                let names: Vec<String> = parts.iter().map(|p| self.part_name(*p)).collect();
                let mut diagnostic = Diagnostic::error(
                    Code::Require,
                    format!("'{}' fails: {message}", require.decl.name),
                    Label::new(place.file, place.span),
                )
                .with_label(Label::with_message(
                    require.decl.file,
                    require.decl.span,
                    "the kind's rule",
                ))
                .with_note(format!("for {} in {joint_name}", names.join(" and ")));
                if let Some(note) = self.measures_note(&require.condition, &kind_env) {
                    diagnostic = diagnostic.with_note(note);
                }
                if let Some(note) = self.meeting_note(&require.condition, &kind_env) {
                    diagnostic = diagnostic.with_note(note);
                }
                let help = self.require_help(&require.condition, &kind_env, "joint");
                let reported = self.report(diagnostic.with_help(help));
                failure.get_or_insert(reported.into());
            } else if !holds {
                // A mirror image: its source reports.
                failure.get_or_insert(Fault::Earlier.into());
            }
            requires.push(solved::RequireResult {
                name: format!("{joint_name}.{}", require.decl.name),
                holds,
                message,
            });
        }
        let mut extensions = Vec::new();
        if let Some(joint_def) = &kind.joint {
            for extend in &joint_def.adds {
                let by = self.eval_length(&extend.by, &kind_env)?;
                let Some(piece) = parts.get(extend.role).copied() else {
                    continue;
                };
                for other in parts.iter().copied().filter(|o| *o != piece) {
                    let (a, b) = (self.drawn_box(piece)?, self.drawn_box(other)?);
                    let Some(world) = a.touching_axis(&b) else {
                        continue;
                    };
                    let orientation = self.part_orientation(piece)?;
                    if orientation.part_axis(world) != PartAxis::Length {
                        continue;
                    }
                    let toward_positive = a.max(world) == b.min(world);
                    let along = orientation.direction(PartAxis::Length).is_positive();
                    let end = if along == toward_positive {
                        End::End
                    } else {
                        End::Start
                    };
                    let entry = extra.entry(piece).or_insert([ExactLength::ZERO; 2]);
                    let slot = match end {
                        End::Start => &mut entry[0],
                        End::End => &mut entry[1],
                    };
                    *slot = slot.checked_add(by).ok_or(Fault::Overflow)?;
                    extensions.push(solved::Extension {
                        part: piece,
                        axis: PartAxis::Length,
                        end,
                        by: round_length(by)?,
                    });
                }
            }
            if place.report && splice == 0 {
                let has_members = kind
                    .bodies
                    .iter()
                    .any(|body| matches!(body.with, stave_model::defs::Carrier::Contact(_)));
                if has_members || !joint_def.uses.is_empty() {
                    self.report(
                        Diagnostic::warning(
                            Code::NotSupported,
                            format!(
                                "placing or listing the fasteners of '{}' at a splice is not supported yet",
                                method.value.name
                            ),
                            Label::new(place.file, place.span),
                        )
                        .with_note(format!(
                            "{name} is spliced; the pieces its splices place or use are not drawn, drilled or listed"
                        ))
                        .with_help("lay out the fasteners at each splice by hand, and add them to your shopping list"),
                    );
                }
            }
        }
        if let Some(failure) = failure {
            return Err(failure);
        }
        let contact = match parts.as_slice() {
            [p, q] => match self.drawn_box(*p)?.touching_axis(&self.drawn_box(*q)?) {
                Some(axis) => Contact::Butt { axis },
                None => Contact::Cross,
            },
            _ => Contact::Connection,
        };
        Ok(SpliceWork {
            pair: PairWork {
                parts,
                across: false,
                contact,
                env,
                extensions,
                making: crate::joints::Making::Made,
            },
            requires,
        })
    }

    /// Hands the cuts other joints and hardware made on a spliced part to
    /// the pieces they fall on; a cut that reaches into a splice is an
    /// error.
    #[allow(clippy::too_many_arguments)]
    fn hand_out_cuts(
        &mut self,
        name: &str,
        splicing: &Splicing,
        from_start: bool,
        drawn_start: ExactLength,
        total: ExactLength,
        pieces: &[Piece],
        foreign: Vec<Cut>,
        place: Place,
        piece_cuts: &mut BTreeMap<PartId, Vec<Cut>>,
    ) -> Solved<()> {
        let whole = round_length(total)?;
        let ranges: Vec<(Length, Length)> = pieces
            .iter()
            .map(|p| Ok((round_length(p.low)?, round_length(p.high)?)))
            .collect::<Solved<_>>()?;
        // Where the pieces share wood: each splice.
        let zones: Vec<(Length, Length)> = ranges
            .windows(2)
            .map(|pair| match pair {
                [(a_low, a_high), (b_low, b_high)] => {
                    ((*a_low).max(*b_low), (*a_high).min(*b_high))
                }
                _ => (Length::ZERO, Length::ZERO),
            })
            .collect();
        for cut in foreign {
            let (low, high) = along_extent(&cut, whole);
            if let Some((splice, zone)) = zones
                .iter()
                .enumerate()
                .find(|(_, zone)| high > zone.0 && low < zone.1)
            {
                return Err(self.cut_in_splice(
                    name,
                    splicing,
                    from_start,
                    drawn_start,
                    total,
                    &cut,
                    (low, high),
                    splice,
                    *zone,
                    place,
                ));
            }
            let middle = Length::from_nm(low.nm().saturating_add(high.nm()).div_euclid(2));
            let found = ranges
                .iter()
                .position(|(a, b)| *a <= middle && middle <= *b)
                .unwrap_or(0);
            let (Some(piece), Some((offset, end))) = (pieces.get(found), ranges.get(found)) else {
                return Err(Fault::Internal("the piece a cut falls in").into());
            };
            let length = end.checked_sub(*offset).ok_or(Fault::Overflow)?;
            piece_cuts
                .entry(piece.stand_in)
                .or_default()
                .push(shifted(cut, *offset, length, whole)?);
        }
        Ok(())
    }

    /// `error[wall]` for a cut that reaches into a splice.
    #[allow(clippy::too_many_arguments)]
    fn cut_in_splice(
        &mut self,
        name: &str,
        splicing: &Splicing,
        from_start: bool,
        drawn_start: ExactLength,
        total: ExactLength,
        cut: &Cut,
        (low, high): (Length, Length),
        splice: usize,
        zone: (Length, Length),
        place: Place,
    ) -> Failure {
        if !place.report {
            // A mirror image: its source reports.
            return Fault::Earlier.into();
        }
        // Distances along the part from the `from` end, on its drawn
        // length.
        let measured = |at: Length| -> ExactLength {
            let at = at.to_exact();
            let value = if from_start {
                at.checked_sub(drawn_start)
            } else {
                total
                    .checked_sub(drawn_start)
                    .and_then(|x| x.checked_sub(at))
            };
            value.unwrap_or(ExactLength::ZERO)
        };
        let span_of = |a: Length, b: Length| -> String {
            let (x, y) = (measured(a), measured(b));
            let (x, y) = if x <= y { (x, y) } else { (y, x) };
            format!(
                "{} to {}",
                stave_model::value::format_length(x),
                stave_model::value::format_length(y)
            )
        };
        let (owner, owner_place) = match cut.source {
            CutSource::Joint { joint, .. } => (
                self.model
                    .joints
                    .get(joint.index())
                    .map(|j| j.name.clone())
                    .unwrap_or_default(),
                Some(self.joint_place(joint)),
            ),
            CutSource::Hardware { hardware } => {
                let inst = self.model.hardware.get(hardware.index());
                (
                    inst.map(|h| h.name.clone()).unwrap_or_default(),
                    inst.and_then(|h| self.model.defs.hardware_def(h.def))
                        .map(|def| (def.decl.file, def.call.span)),
                )
            }
            CutSource::Boards { part, .. } => (format!("{}'s boards", self.part_name(part)), None),
        };
        let what = cut_words(cut);
        let mut diagnostic = Diagnostic::error(
            Code::Wall,
            format!(
                "{owner}: its {what} in {name} crosses the splice between {name}.piece[{}] and {name}.piece[{}]",
                splice + 1,
                splice + 2
            ),
            Label::new(place.file, place.span),
        )
        .with_note(format!(
            "measured from {name}'s {} end, the {what} runs from {} and the splice from {}",
            splicing.from.word(),
            span_of(low, high),
            span_of(zone.0, zone.1)
        ))
        .with_help(format!(
            "a splice needs its wood whole: move the splice with at: in {name}'s lengths: so it clears the {what}, or move {owner}"
        ));
        if let Some((file, span)) = owner_place {
            diagnostic = diagnostic.with_label(Label::with_message(
                file,
                span,
                format!("the {what} is cut here"),
            ));
        }
        self.report(diagnostic).into()
    }

    /// Pass B of one splice: the parts it makes, its cuts and steps, as a
    /// joint of the solved design (its pieces still stand-ins).
    #[allow(clippy::too_many_arguments)]
    fn splice_pass_b(
        &mut self,
        joint: JointId,
        part: PartId,
        name: &str,
        splice: usize,
        method: &Method,
        work: &mut SpliceWork,
        tables: &mut Tables<'_>,
        piece_cuts: &mut BTreeMap<PartId, Vec<Cut>>,
    ) -> Solved<Joint> {
        let kind_id = method.value.kind;
        let Some(kind) = self.model.defs.kind(kind_id).cloned() else {
            return Err(Fault::Internal("a kind's declaration").into());
        };
        let Some(ctx) = self.module_ctx(DefId::Kind(kind_id)) else {
            return Err(Fault::Internal("the file an item is declared in").into());
        };
        let joint_def = kind.joint.clone().unwrap_or_default();
        let joint_name = format!("{name}.splice[{}]", splice + 1);
        let assembly = self.model.parts.get(part.index()).and_then(|p| p.assembly);
        let pair = &mut work.pair;
        let mut made_ids = Vec::new();
        for made_def in &joint_def.makes {
            let id = PartId::from_usize(self.model.parts.len() + tables.made.len());
            let made_name = format!("{joint_name}.{}", made_def.name);
            let env = Env {
                ctx,
                file: kind.decl.file,
                kind: Some(&pair.env),
                array: None,
            };
            let placed = self.place_made_part(made_def, pair, tables.geoms, &env, &made_name)?;
            let note = match &made_def.body.note {
                Some(text) => Some(self.format_text(text, &env)?),
                None => None,
            };
            tables.geoms.set(id, placed.geom);
            if tables.cuts.len() <= id.index() {
                tables.cuts.resize(id.index() + 1, Vec::new());
                tables.memberships.resize(id.index() + 1, Vec::new());
            }
            if let Some(list) = tables.memberships.get_mut(id.index()) {
                list.push(joint);
            }
            tables.made.push(MadeInfo {
                origin: PartOrigin::Made { joint, pair: 0 },
                assembly,
                name: made_name,
                stock: placed.stock,
                note,
            });
            made_ids.push(id);
        }
        pair.env.made = made_ids.clone();
        let env = Env {
            ctx,
            file: kind.decl.file,
            kind: Some(&pair.env),
            array: None,
        };
        let source = CutSource::Joint { joint, pair: 0 };
        for cut_def in &joint_def.cuts {
            let (file, span) = self.part_place(part);
            let at = CutAt {
                owner: &joint_name,
                file,
                span,
            };
            for (target, face, place, cut_kind) in
                self.pair_cuts(cut_def, pair, tables.geoms, &env, at)?
            {
                piece_cuts.entry(target).or_default().push(Cut {
                    source,
                    name: cut_def.name.clone(),
                    face,
                    place,
                    kind: cut_kind,
                });
            }
        }
        let insert = self.pair_steps(kind_id, pair, &env)?;
        let mut cut_order = Vec::new();
        if !joint_def.cut_first.is_empty() {
            for role in &joint_def.cut_first {
                if let Some(part) = pair.parts.get(*role)
                    && !cut_order.contains(part)
                {
                    cut_order.push(*part);
                }
            }
            for part in &pair.parts {
                if !cut_order.contains(part) {
                    cut_order.push(*part);
                }
            }
            cut_order.extend(made_ids.iter().copied());
        }
        let note = match &joint_def.note {
            Some(text) => Some(self.format_text(text, &env)?),
            None => None,
        };
        let roles = kind
            .roles
            .iter()
            .zip(&pair.parts)
            .map(|(role, part)| RoleBinding {
                role: role.name.clone(),
                part: *part,
            })
            .collect();
        let index = u32::try_from(splice + 1)
            .ok()
            .and_then(PieceIndex::new)
            .ok_or(Fault::Overflow)?;
        Ok(Joint {
            id: joint,
            name: joint_name,
            kind: method.value.name.clone(),
            kind_id,
            assembly,
            glue: method.glue,
            methods: method.methods.clone(),
            pairs: vec![JointPair {
                roles,
                contact: pair.contact,
                extensions: pair.extensions.clone(),
                cut_order,
                insert,
                made_parts: made_ids,
                requires: std::mem::take(&mut work.requires),
                members: Vec::new(),
            }],
            unmatched: Vec::new(),
            note,
            origin: JointOrigin::Splice { part, index },
        })
    }

    /// A part both spliced and wider than its stock: its pieces stay single
    /// boards for now.
    pub(crate) fn spliced_boards(&mut self, split: &Split, geoms: &GeomTable) {
        let (file, span) = self.part_place(split.whole);
        let source = self.body_source(split.whole);
        let Some(source) = self.settled(source, file, span) else {
            return;
        };
        if source != split.whole {
            return;
        }
        let stock = self.part_stock(split.whole);
        let Some(stock) = self.settled(stock, file, span) else {
            return;
        };
        let max_width = self.stock_length(stock, StockField::MaxWidth);
        let Some(Some(max_width)) = self.settled(max_width, file, span) else {
            return;
        };
        let mut wide = false;
        for piece in &split.pieces {
            let Some(geom) = geoms.get(*piece) else {
                continue;
            };
            let width = geom.cut_size(PartAxis::Width);
            if self
                .settled(width, file, span)
                .is_some_and(|width| width > max_width)
            {
                wide = true;
            }
        }
        if !wide {
            return;
        }
        if let Some((file, span)) = self.lengths_place(source) {
            self.report(
                Diagnostic::warning(
                    Code::NotSupported,
                    format!(
                        "a spliced part wider than its stock is not supported yet: {}'s pieces are drawn as single boards",
                        split.name
                    ),
                    Label::new(file, span),
                )
                .with_help("glue up each piece from boards by hand, or make the part from separate parts"),
            );
        }
    }
}

/// A splice's pair while it is worked out.
struct SpliceWork {
    pair: PairWork,
    requires: Vec<solved::RequireResult>,
}

/// World coordinates along a part's length of a stretch of its own frame.
fn to_world(
    geom: &PartGeom,
    low: ExactLength,
    high: ExactLength,
) -> Solved<(ExactLength, ExactLength)> {
    let direction = geom.orientation.direction(PartAxis::Length);
    let world = direction.axis();
    if direction.is_positive() {
        let base = geom.cut_box.min(world);
        Ok((
            base.checked_add(low).ok_or(Fault::Overflow)?,
            base.checked_add(high).ok_or(Fault::Overflow)?,
        ))
    } else {
        let base = geom.cut_box.max(world);
        Ok((
            base.checked_sub(high).ok_or(Fault::Overflow)?,
            base.checked_sub(low).ok_or(Fault::Overflow)?,
        ))
    }
}

/// A cut with the stand-ins it names replaced by parts.
fn remap_cut(mut cut: Cut, real: &impl Fn(PartId) -> PartId) -> Cut {
    cut.place = match cut.place {
        CutPlace::End { toward } => CutPlace::End {
            toward: real(toward),
        },
        CutPlace::Side {
            from,
            along_from,
            across_axis,
            across_from,
        } => CutPlace::Side {
            from: real(from),
            along_from,
            across_axis,
            across_from,
        },
        CutPlace::Crossing { other, footprint } => CutPlace::Crossing {
            other: real(other),
            footprint,
        },
        member @ CutPlace::Member { .. } => member,
    };
    cut
}

// ---------------------------------------------------------------------------
// Pointing the solved design at the pieces
// ---------------------------------------------------------------------------

/// How much of `b` lies against or inside `a`: the sum of the overlaps
/// along the three axes, or `None` when they are apart.
fn closeness(a: &WorldBox, b: &WorldBox) -> Option<i64> {
    let mut sum: i64 = 0;
    for axis in Axis::ALL {
        let low = a.min().get(axis).max(b.min().get(axis));
        let high = a.max().get(axis).min(b.max().get(axis));
        let overlap = high.checked_sub(low)?;
        if overlap < Length::ZERO {
            return None;
        }
        sum = sum.saturating_add(overlap.nm());
    }
    Some(sum)
}

impl Solver<'_> {
    /// Points everything in the solved design that named a spliced part at
    /// the piece it belongs to: the joint pairs and hardware that meet the
    /// part (by the cuts they made on it, or else by where they are), the
    /// cuts of other parts that name it, the assemblies and scenes that
    /// list it. The first piece is renamed `<part>.piece[1]` and lists the
    /// pieces.
    pub(crate) fn rebind_pieces(&self, design: &mut SolvedDesign, splits: &[Split]) {
        for split in splits {
            rebind_split(design, split);
        }
    }
}

fn rebind_split(design: &mut SolvedDesign, split: &Split) {
    let whole = split.whole;
    let boxes: Vec<(PartId, WorldBox)> = split
        .pieces
        .iter()
        .filter_map(|id| design.part(*id).map(|p| (*id, p.cut_box())))
        .collect();
    // The piece that holds a cut from `source`, if any.
    let piece_with = |design: &SolvedDesign, source: CutSource| -> Option<PartId> {
        split.pieces.iter().copied().find(|id| {
            design
                .part(*id)
                .is_some_and(|p| p.cuts.iter().any(|c| c.source == source))
        })
    };
    // The piece nearest a set of boxes.
    let nearest = |others: &[WorldBox]| -> PartId {
        let mut best: Option<(i64, PartId)> = None;
        for (id, piece) in &boxes {
            let score = others.iter().filter_map(|o| closeness(piece, o)).max();
            if let Some(score) = score
                && best.is_none_or(|(b, _)| score > b)
            {
                best = Some((score, *id));
            }
        }
        best.map_or(whole, |(_, id)| id)
    };
    // The piece at each end of the part's own length.
    let (start_piece, end_piece) = {
        let lows: Vec<(Length, PartId)> = split
            .pieces
            .iter()
            .filter_map(|id| {
                let part = design.part(*id)?;
                let whole_part = design.part(whole)?;
                let direction = whole_part.pose.orientation.direction(PartAxis::Length);
                let axis = direction.axis();
                let low = if direction.is_positive() {
                    part.cut_box().min().get(axis)
                } else {
                    part.cut_box().max().get(axis).checked_neg()?
                };
                Some((low, *id))
            })
            .collect();
        (
            lows.iter().min().map_or(whole, |(_, id)| *id),
            lows.iter().max().map_or(whole, |(_, id)| *id),
        )
    };
    let mut chosen: BTreeMap<(JointId, usize), PartId> = BTreeMap::new();
    for joint_index in 0..design.joints.len() {
        let Some(joint) = design.joints.get(joint_index) else {
            continue;
        };
        if split.splices.contains(&joint.id) {
            continue;
        }
        let id = joint.id;
        for pair_index in 0..joint.pairs.len() {
            let Some(pair) = design
                .joints
                .get(joint_index)
                .and_then(|j| j.pairs.get(pair_index))
            else {
                continue;
            };
            if !pair.roles.iter().any(|r| r.part == whole) {
                continue;
            }
            let others: Vec<WorldBox> = pair
                .roles
                .iter()
                .filter(|r| r.part != whole)
                .filter_map(|r| design.part(r.part).map(|p| p.cut_box()))
                .collect();
            let piece = piece_with(
                design,
                CutSource::Joint {
                    joint: id,
                    pair: pair_index,
                },
            )
            .unwrap_or_else(|| nearest(&others));
            chosen.insert((id, pair_index), piece);
            let Some(pair) = design
                .joints
                .get_mut(joint_index)
                .and_then(|j| j.pairs.get_mut(pair_index))
            else {
                continue;
            };
            for role in &mut pair.roles {
                if role.part == whole {
                    role.part = piece;
                }
            }
            for extension in &mut pair.extensions {
                if extension.part == whole {
                    extension.part = match (extension.axis, extension.end) {
                        (PartAxis::Length, End::Start) => start_piece,
                        (PartAxis::Length, End::End) => end_piece,
                        (PartAxis::Width | PartAxis::Thickness, _) => piece,
                    };
                }
            }
            for step in &mut pair.insert {
                if step.part == whole {
                    step.part = piece;
                }
            }
            for part in &mut pair.cut_order {
                if *part == whole {
                    *part = piece;
                }
            }
            for member in &mut pair.members {
                if member.carrier == BodyCarrier::Part(whole) {
                    member.carrier = BodyCarrier::Part(piece);
                }
            }
        }
    }
    // Hardware.
    for index in 0..design.hardware.len() {
        let Some(hardware) = design.hardware.get(index) else {
            continue;
        };
        let names_whole = hardware.piece == whole
            || hardware.roles.iter().any(|(_, value)| match value {
                solved::RoleValue::Part(part) => *part == whole,
                solved::RoleValue::Region(_) => false,
            })
            || hardware
                .members
                .iter()
                .any(|m| m.carrier == BodyCarrier::Part(whole));
        if !names_whole {
            continue;
        }
        let bounds = hardware.bounds;
        let piece = piece_with(
            design,
            CutSource::Hardware {
                hardware: hardware.id,
            },
        )
        .unwrap_or_else(|| nearest(&[bounds]));
        let Some(hardware) = design.hardware.get_mut(index) else {
            continue;
        };
        if hardware.piece == whole {
            hardware.piece = piece;
        }
        for (_, value) in &mut hardware.roles {
            match value {
                solved::RoleValue::Part(part) if *part == whole => *part = piece,
                solved::RoleValue::Part(_) | solved::RoleValue::Region(_) => {}
            }
        }
        for member in &mut hardware.members {
            if member.carrier == BodyCarrier::Part(whole) {
                member.carrier = BodyCarrier::Part(piece);
            }
        }
    }
    // Cuts on other parts that name the part.
    for part in &mut design.parts {
        if split.pieces.contains(&part.id) {
            continue;
        }
        for cut in &mut part.cuts {
            let piece = match cut.source {
                CutSource::Joint { joint, pair } => chosen.get(&(joint, pair)).copied(),
                CutSource::Hardware { .. } | CutSource::Boards { .. } => None,
            };
            let Some(piece) = piece else {
                continue;
            };
            match &mut cut.place {
                CutPlace::End { toward } if *toward == whole => *toward = piece,
                CutPlace::Side { from, .. } if *from == whole => *from = piece,
                CutPlace::Crossing { other, .. } if *other == whole => *other = piece,
                CutPlace::End { .. }
                | CutPlace::Side { .. }
                | CutPlace::Crossing { .. }
                | CutPlace::Member { .. } => {}
            }
        }
    }
    // Which joints each piece is in.
    for id in &split.pieces {
        let mut joints: Vec<JointId> = design
            .joints
            .iter()
            .filter(|j| {
                j.pairs
                    .iter()
                    .any(|pair| pair.roles.iter().any(|r| r.part == *id))
            })
            .map(|j| j.id)
            .collect();
        joints.sort();
        joints.dedup();
        if let Some(part) = design.parts.get_mut(id.index()) {
            part.joints = joints;
        }
    }
    // Assemblies and scenes list every piece where they listed the part.
    for assembly in &mut design.assemblies {
        if let Some(at) = assembly.parts.iter().position(|p| *p == whole) {
            for (offset, id) in split.pieces.iter().skip(1).enumerate() {
                if !assembly.parts.contains(id) {
                    assembly.parts.insert(at + 1 + offset, *id);
                }
            }
        }
    }
    for scene in &mut design.scenes {
        let lists: Vec<&mut Vec<solved::SceneItem>> = match &mut scene.show {
            Some(show) => vec![show, &mut scene.hide],
            None => vec![&mut scene.hide],
        };
        for list in lists {
            if let Some(at) = list
                .iter()
                .position(|item| *item == solved::SceneItem::Part(whole))
            {
                for (offset, id) in split.pieces.iter().skip(1).enumerate() {
                    list.insert(at + 1 + offset, solved::SceneItem::Part(*id));
                }
            }
        }
    }
    // The first piece.
    let pieces = NonEmpty::new(split.pieces.clone());
    if let Some(part) = design.parts.get_mut(whole.index()) {
        part.name = format!("{}.piece[1]", split.name);
        part.pieces = pieces.map(|parts| solved::Pieces {
            name: split.name.clone(),
            parts,
            splices: split.splices.clone(),
        });
    }
}
