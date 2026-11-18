//! Boards (grammar, "Boards"): a `solid` part wider across the grain than
//! its stock's `max_width` is made of equal boards, joined edge to edge by
//! the joint kinds its `boards:` names.
//!
//! The boards are derived, never written. While a part's board joints are
//! worked out, each board stands in for a part under an id of its own,
//! from [`BOARD_ID_BASE`] up, so the joint kind's expressions, selectors
//! and made parts work on boards exactly as on parts. Those ids never
//! leave this module: the solved design names a neighbouring board by the
//! part it belongs to, and the cuts on each board are measured in the
//! board's own frame.
//!
//! Simplifications, recorded here as the plan asks: boards are of equal
//! width; the cuts other joints make on a part made of boards stay on the
//! part, and are assigned to its boards when the work plan exists.

use std::collections::BTreeMap;

use stave_lang::ast::{BaseKind, Span};
use stave_lang::placement::{Axis, PartAxis};
use stave_lang::units::{ExactLength, Rational};
use stave_lang::{Code, Diagnostic, FileId, Label};
use stave_model::defs::{DefId, StockField};
use stave_model::ids::{JointId, KindId, PartId};
use stave_model::instance::Ctx;
use stave_model::nonempty::NonEmpty;
use stave_model::solved::{
    self, BoardIndex, BoardMethod, Contact, Cut, CutKind, CutPlace, CutSource, End, Orientation,
    PartOrigin, RequireResult, Seam,
};
use stave_model::value::{ExactBox, KindValue, Value};
use stave_schema::CutOperation;
use stave_schema::kinds::RoleType;
use stave_schema::words::{Grain, StockForm};

use crate::build::MadeInfo;
use crate::eval::{Env, KindEnv};
use crate::joints::{GeomTable, PairWork, PartGeom, local_slot, round_length};
use crate::{Reported, Solved, Solver};

/// The first id that stands for a board rather than a part. Parts, made
/// parts included, come from source files and never number this many.
pub(crate) const BOARD_ID_BASE: u32 = 1 << 31;

/// `true` if `part` stands for a board.
pub(crate) fn is_board(part: PartId) -> bool {
    part.get() >= BOARD_ID_BASE
}

/// What a stand-in is a share of its part along: its width (a board) or
/// its length (a piece of a spliced part, see [`crate::pieces`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Division {
    /// A board, edge to edge with its neighbours.
    Board,
    /// A piece, end to end with its neighbours.
    Piece,
}

/// A board standing in for a part while its part's board joints are worked
/// out: what placement would know about a part. A piece of a spliced part
/// stands in the same way while its splices are worked out.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct BoardPart {
    /// The part it belongs to.
    pub part: PartId,
    /// Board or piece.
    pub division: Division,
    /// Its number.
    pub index: BoardIndex,
    /// The part's orientation.
    pub orientation: Orientation,
    /// Its drawn size per local axis: its share of the part's width, the
    /// part's cut length and thickness.
    pub drawn: [ExactLength; 3],
    /// Its low corner per world axis (across, up, deep).
    pub min: [ExactLength; 3],
}

/// One joint kind a part's boards are joined by, bound.
struct Method {
    kind: KindId,
    name: String,
    settings: Vec<(String, Value)>,
    glue: bool,
    /// `true` for a kind with one `Parts` role.
    across: bool,
}

/// The ids of a part's boards and where its seams are.
struct Layout {
    ids: Vec<PartId>,
    seams: Vec<Seam>,
}

/// One application of a board joint: a seam, or all the boards.
struct Application {
    method: usize,
    seam: Option<usize>,
    pair: PairWork,
}

fn world_slot(axis: Axis) -> usize {
    match axis {
        Axis::Across => 0,
        Axis::Up => 1,
        Axis::Deep => 2,
    }
}

impl Solver<'_> {
    /// The boards of every part wider than its stock, with the parts their
    /// joints make. Runs once every joint and mirrored assembly is done.
    pub(crate) fn make_boards(
        &mut self,
        geoms: &mut GeomTable,
        cuts: &mut Vec<Vec<Cut>>,
        memberships: &mut Vec<Vec<JointId>>,
        made: &mut Vec<MadeInfo>,
        spliced: &[crate::pieces::Split],
    ) -> Vec<Option<solved::Boards>> {
        let count = self.model.parts.len();
        let mut boards = vec![None; count];
        for index in 0..count {
            let part = PartId::from_usize(index);
            if let Some(split) = spliced.iter().find(|split| split.whole == part) {
                self.spliced_boards(split, geoms);
                continue;
            }
            let Some(geom) = geoms.get(part).cloned() else {
                continue;
            };
            let made_before = made.len();
            match self.part_boards(part, &geom, geoms, cuts, memberships, made) {
                Ok(Some(found)) => {
                    if let Some(slot) = boards.get_mut(index) {
                        *slot = Some(found);
                    }
                }
                Ok(None) => {}
                Err(Reported) => {
                    // Nothing half-made is kept.
                    made.truncate(made_before);
                }
            }
            for board in std::mem::take(&mut self.boards).into_keys() {
                geoms.boards.remove(&board);
            }
        }
        boards
    }

    /// The boards of one part, or `None` if it isn't made of boards.
    #[allow(clippy::too_many_arguments)]
    fn part_boards(
        &mut self,
        part: PartId,
        geom: &PartGeom,
        geoms: &mut GeomTable,
        cuts: &mut Vec<Vec<Cut>>,
        memberships: &mut Vec<Vec<JointId>>,
        made: &mut Vec<MadeInfo>,
    ) -> Solved<Option<solved::Boards>> {
        if self.part_profile(part)?.is_some() {
            return Ok(None);
        }
        let stock = self.part_stock(part)?;
        let Some(max_width) = self.stock_length(stock, StockField::MaxWidth)? else {
            return Ok(None);
        };
        let form = self
            .model
            .defs
            .stock(stock)
            .map(|s| s.form)
            .ok_or(Reported)?;
        let source = self.body_source(part)?;
        let image = source != part;
        let body = self.part_body(source).cloned();
        let grain = body.as_ref().map_or(Grain::Length, |b| b.grain);
        let axis = match grain {
            Grain::Length => PartAxis::Width,
            Grain::Width => PartAxis::Length,
        };
        let total = geom.cut_size(axis)?;
        if total <= max_width {
            return Ok(None);
        }
        let (file, span) = self.boards_place(source);
        match form {
            StockForm::Solid => {}
            StockForm::Dowel => return Ok(None),
            StockForm::Plywood | StockForm::Mdf => {
                if image {
                    return Err(Reported);
                }
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
                        "{name}: its {} across the grain works out as {}, which is more than its stock's max_width of {}",
                        axis.word(),
                        stave_model::value::format_length(total),
                        stave_model::value::format_length(max_width)
                    ),
                    file,
                    self.size_span(source, axis).unwrap_or(span),
                    format!(
                        "a sheet isn't glued up from boards: choose a sheet wider than '{stock_name}', or make the part from several parts"
                    ),
                ));
            }
        }
        let ratio = total.checked_ratio(max_width).ok_or(Reported)?;
        let whole = ratio.floor();
        let count = if ratio.is_integer() {
            whole
        } else {
            whole.checked_add(1).ok_or(Reported)?
        };
        let count = u32::try_from(count).map_err(|_| Reported)?;
        let methods = self.board_methods(part, source, file, span)?;
        let layout = self.lay_out_boards(part, geom, axis, total, count, geoms)?;

        // Pass A: requires and what each joint adds, per application.
        let mut requires = Vec::new();
        let mut applications = Vec::new();
        let mut extra: BTreeMap<PartId, [ExactLength; 2]> = BTreeMap::new();
        for (method_index, method) in methods.iter().enumerate() {
            let plans: Vec<(Option<usize>, Vec<PartId>)> = if method.across {
                vec![(None, layout.ids.clone())]
            } else {
                layout
                    .ids
                    .windows(2)
                    .enumerate()
                    .map(|(seam, pair)| (Some(seam), pair.to_vec()))
                    .collect()
            };
            for (seam, parts) in plans {
                let pair = self.board_pass_a(
                    part,
                    method,
                    seam,
                    parts,
                    axis,
                    (file, span),
                    !image,
                    &mut extra,
                    &mut requires,
                )?;
                applications.push(Application {
                    method: method_index,
                    seam,
                    pair,
                });
            }
        }
        // What the joints add widens the boards.
        for (board, [start, end]) in &extra {
            let Some(mut board_geom) = geoms.get(*board).cloned() else {
                return Err(Reported);
            };
            let slot = local_slot(axis);
            if let (Some(s), Some(e)) =
                (board_geom.start.get_mut(slot), board_geom.end.get_mut(slot))
            {
                *s = *start;
                *e = *end;
            }
            let direction = board_geom.orientation.direction(axis);
            let world = direction.axis();
            let (low_add, high_add) = if direction.is_positive() {
                (*start, *end)
            } else {
                (*end, *start)
            };
            let low = board_geom
                .cut_box
                .min(world)
                .checked_sub(low_add)
                .ok_or(Reported)?;
            let high = board_geom
                .cut_box
                .max(world)
                .checked_add(high_add)
                .ok_or(Reported)?;
            board_geom.cut_box = board_geom.cut_box.with(world, low, high);
            geoms.set(*board, board_geom);
        }

        // Pass B: made parts and cuts.
        let mut board_cuts: BTreeMap<PartId, Vec<Cut>> = BTreeMap::new();
        for application in &mut applications {
            let Some(method) = methods.get(application.method) else {
                return Err(Reported);
            };
            self.board_pass_b(
                part,
                method,
                application,
                geoms,
                cuts,
                memberships,
                made,
                &mut board_cuts,
            )?;
        }

        // The boards as cut.
        // Where a frame starts along `axis`, signed so that differences
        // are distances along the part's own axis.
        let origin = |geom: &PartGeom| -> Solved<ExactLength> {
            let direction = geom.orientation.direction(axis);
            let world = direction.axis();
            if direction.is_positive() {
                Ok(geom.cut_box.min(world))
            } else {
                geom.cut_box.max(world).checked_neg().ok_or(Reported)
            }
        };
        let part_origin = origin(geom)?;
        let mut written = Vec::new();
        for (number, board) in layout.ids.iter().enumerate() {
            let board_geom = geoms.get(*board).cloned().ok_or(Reported)?;
            let index = u32::try_from(number)
                .ok()
                .and_then(|n| n.checked_add(1))
                .and_then(BoardIndex::new)
                .ok_or(Reported)?;
            let offset = origin(&board_geom)?
                .checked_sub(part_origin)
                .ok_or(Reported)?;
            written.push(solved::Board {
                index,
                width: round_length(board_geom.cut_size(axis)?)?,
                offset: round_length(offset)?,
                cuts: board_cuts.remove(board).unwrap_or_default(),
            });
        }
        let boards = NonEmpty::new(written).ok_or(Reported)?;
        let methods = NonEmpty::new(
            methods
                .into_iter()
                .map(|m| BoardMethod {
                    kind: m.kind,
                    name: m.name,
                    glue: m.glue,
                })
                .collect(),
        )
        .ok_or(Reported)?;
        Ok(Some(solved::Boards {
            axis,
            boards,
            seams: layout.seams,
            methods,
            requires,
        }))
    }

    /// The part whose body a part uses: itself, or the source of a mirror
    /// image.
    pub(crate) fn body_source(&mut self, part: PartId) -> Solved<PartId> {
        let mut current = part;
        while let Some((source, _, _)) = self.mirror_source(current)? {
            current = source;
        }
        Ok(current)
    }

    /// Where a part's `boards:` is written, or the part when it isn't.
    fn boards_place(&self, part: PartId) -> (FileId, Span) {
        let (Some(body), Some(env)) = (self.part_body(part), self.part_env(part)) else {
            return (self.project_file, self.project_span);
        };
        match &body.boards {
            Some(expr) => (env.file, expr.span),
            None => (env.file, body.span),
        }
    }

    /// The span of a part's size field along `axis`, if written.
    pub(crate) fn size_span(&self, part: PartId, axis: PartAxis) -> Option<Span> {
        let body = self.part_body(part)?;
        let size = match axis {
            PartAxis::Length => Some(&body.length),
            PartAxis::Width => body.width.as_ref(),
            PartAxis::Thickness => None,
        }?;
        Some(match size {
            stave_model::defs::Size::Expr(expr) => expr.span,
            stave_model::defs::Size::Fit(span) => *span,
        })
    }

    /// An expression of a module's header (a project's or a library's
    /// `boards:`), evaluated in that module.
    fn eval_in_module(
        &mut self,
        module: stave_modules::ModuleId,
        file: FileId,
        expr: &stave_model::defs::RExpr,
    ) -> Solved<Value> {
        let instance = self.model.module_instance(module).ok_or(Reported)?;
        let env = Env {
            ctx: Ctx {
                instance,
                element: None,
            },
            file,
            kind: None,
            array: None,
        };
        self.eval(expr, &env)
    }

    /// The joint kinds a part's boards are joined by: its own `boards:`,
    /// the project's, or the first prelude library's (grammar, "Boards").
    fn board_methods(
        &mut self,
        part: PartId,
        source: PartId,
        file: FileId,
        span: Span,
    ) -> Solved<Vec<Method>> {
        let own = self
            .part_body(source)
            .and_then(|b| b.boards.clone())
            .zip(self.part_env(source));
        let value = match own {
            Some((expr, env)) => Some(self.eval(&expr, &env)?),
            None => match self.model.defs.project(self.model.root).cloned() {
                Some(project) if project.boards.is_some() => match &project.boards {
                    Some(expr) => Some(self.eval_in_module(self.model.root, project.file, expr)?),
                    None => None,
                },
                _ => {
                    // The prelude's say, from the first library in it that
                    // has `boards:`.
                    let found = self.model.defs.prelude.iter().find_map(|module| {
                        let library = self
                            .model
                            .defs
                            .modules
                            .get(module.index())?
                            .library
                            .as_ref()?;
                        Some((*module, library.file, library.boards.clone()?))
                    });
                    match found {
                        Some((module, file, expr)) => {
                            Some(self.eval_in_module(module, file, &expr)?)
                        }
                        None => None,
                    }
                }
            },
        };
        let kinds: Vec<KindValue> = match value {
            Some(Value::Boards(joining)) => joining.by.iter().cloned().collect(),
            Some(other) => {
                return Err(self.error(
                    Code::TypeMismatch,
                    format!(
                        "'boards' needs joined(by: ...), found {}",
                        other.description()
                    ),
                    file,
                    span,
                    "write boards: joined(by: kind_name)",
                ));
            }
            None => {
                let name = self.part_name(part);
                return Err(self.error(
                    Code::UnknownName,
                    format!("{name} is made of boards, and nothing says how they are joined"),
                    file,
                    span,
                    "say how its boards are joined with boards: joined(by: ...)",
                ));
            }
        };
        let mut methods = Vec::new();
        for value in kinds {
            if self.model.defs.is_poisoned(DefId::Kind(value.kind)) {
                return Err(self.skip());
            }
            let Some(kind) = self.model.defs.kind(value.kind).cloned() else {
                return Err(Reported);
            };
            if kind.base != BaseKind::Joint {
                return Err(self.error(
                    Code::TypeMismatch,
                    format!("'{}' is not a joint kind", value.name),
                    file,
                    span,
                    "boards are joined by joint kinds whose two roles are parts: joined(by: kind_name)",
                ));
            }
            let roles: Vec<RoleType> = kind.roles.iter().map(|r| r.ty).collect();
            let across = match roles.as_slice() {
                [RoleType::Parts] => true,
                [first, second] if *first != RoleType::Region && *second != RoleType::Region => {
                    false
                }
                _ => {
                    return Err(self.error(
                        Code::TypeMismatch,
                        format!(
                            "'{}' can't join boards: it has {} roles",
                            value.name,
                            roles.len()
                        ),
                        file,
                        span,
                        "a joint between boards has two roles (one board on each side of a seam), or one Parts role (across all of them)",
                    ));
                }
            };
            let mut glue = true;
            let mut settings = Vec::new();
            for (name, setting) in value.settings {
                match name.as_str() {
                    "glue" => glue = setting == Value::Bool(true),
                    "methods" => {}
                    _ => settings.push((name, setting)),
                }
            }
            methods.push(Method {
                kind: value.kind,
                name: value.name,
                settings,
                glue,
                across,
            });
        }
        Ok(methods)
    }

    /// Splits a part into `count` equal boards along `axis`, numbered from
    /// the reference edge, and stands each in for a part.
    fn lay_out_boards(
        &mut self,
        part: PartId,
        geom: &PartGeom,
        axis: PartAxis,
        total: ExactLength,
        count: u32,
        geoms: &mut GeomTable,
    ) -> Solved<Layout> {
        let share = total
            .checked_div(Rational::from(i64::from(count)))
            .ok_or(Reported)?;
        let direction = geom.orientation.direction(axis);
        let world = direction.axis();
        // Board 1 is at the reference edge, which is at the start or the
        // end of the part's local axis.
        let from_start = match axis {
            PartAxis::Width => geom.reference_edge.end == End::Start,
            PartAxis::Length | PartAxis::Thickness => true,
        };
        let local_low = |k: u32| -> Solved<ExactLength> {
            let k = Rational::from(i64::from(k));
            let at = share.checked_mul(k).ok_or(Reported)?;
            if from_start {
                Ok(at)
            } else {
                total
                    .checked_sub(at)
                    .and_then(|x| x.checked_sub(share))
                    .ok_or(Reported)
            }
        };
        let to_world =
            |low: ExactLength, high: ExactLength| -> Solved<(ExactLength, ExactLength)> {
                if direction.is_positive() {
                    let base = geom.cut_box.min(world);
                    Ok((
                        base.checked_add(low).ok_or(Reported)?,
                        base.checked_add(high).ok_or(Reported)?,
                    ))
                } else {
                    let base = geom.cut_box.max(world);
                    Ok((
                        base.checked_sub(high).ok_or(Reported)?,
                        base.checked_sub(low).ok_or(Reported)?,
                    ))
                }
            };
        let mut drawn = [ExactLength::ZERO; 3];
        for each in PartAxis::ALL {
            if let Some(slot) = drawn.get_mut(local_slot(each)) {
                *slot = if each == axis {
                    share
                } else {
                    geom.cut_size(each)?
                };
            }
        }
        let first = self
            .boards
            .keys()
            .next_back()
            .map_or(BOARD_ID_BASE, |last| last.get().saturating_add(1));
        let mut ids = Vec::new();
        let mut seams = Vec::new();
        for k in 0..count {
            let low = local_low(k)?;
            let high = low.checked_add(share).ok_or(Reported)?;
            let (world_low, world_high) = to_world(low, high)?;
            let bounds: ExactBox = geom.cut_box.with(world, world_low, world_high);
            let id =
                PartId::from_usize(usize::try_from(first.saturating_add(k)).map_err(|_| Reported)?);
            let index = k.checked_add(1).and_then(BoardIndex::new).ok_or(Reported)?;
            let mut min = [ExactLength::ZERO; 3];
            for each in Axis::ALL {
                if let Some(slot) = min.get_mut(world_slot(each)) {
                    *slot = bounds.min(each);
                }
            }
            self.boards.insert(
                id,
                BoardPart {
                    part,
                    division: Division::Board,
                    index,
                    orientation: geom.orientation,
                    drawn,
                    min,
                },
            );
            geoms.set(
                id,
                PartGeom {
                    orientation: geom.orientation,
                    drawn,
                    start: [ExactLength::ZERO; 3],
                    end: [ExactLength::ZERO; 3],
                    drawn_box: bounds,
                    cut_box: bounds,
                    reference_face: geom.reference_face,
                    reference_edge: geom.reference_edge,
                },
            );
            if let Some(next) = k.checked_add(2).and_then(BoardIndex::new)
                && k.checked_add(1).is_some_and(|n| n < count)
            {
                let at = if from_start { high } else { low };
                seams.push(Seam {
                    between: (index, next),
                    at: round_length(at)?,
                });
            }
            ids.push(id);
        }
        Ok(Layout { ids, seams })
    }

    /// Pass A of one board joint: its requires, and what it adds to each
    /// board.
    #[allow(clippy::too_many_arguments)]
    fn board_pass_a(
        &mut self,
        part: PartId,
        method: &Method,
        seam: Option<usize>,
        parts: Vec<PartId>,
        axis: PartAxis,
        (file, span): (FileId, Span),
        report: bool,
        extra: &mut BTreeMap<PartId, [ExactLength; 2]>,
        requires: &mut Vec<RequireResult>,
    ) -> Solved<PairWork> {
        let Some(kind) = self.model.defs.kind(method.kind).cloned() else {
            return Err(Reported);
        };
        let roles: Vec<Value> = if method.across {
            vec![Value::Parts(parts.clone())]
        } else {
            parts.iter().map(|p| Value::Part(*p)).collect()
        };
        let env = KindEnv::new(
            method.kind,
            roles,
            method.settings.clone(),
            kind.params.len(),
        );
        let Some(ctx) = self.module_ctx(DefId::Kind(method.kind)) else {
            return Err(Reported);
        };
        let kind_env = Env {
            ctx,
            file: kind.decl.file,
            kind: Some(&env),
            array: None,
        };
        let part_name = self.part_name(part);
        let prefix = match seam {
            Some(seam) => format!("{part_name}.seam[{}]", seam.saturating_add(1)),
            None => format!("{part_name}.boards"),
        };
        for require in &kind.requires {
            // The wall round a cut between boards isn't measured yet
            // (grammar, "Walls"): such a rule is left out here.
            if crate::walls::wall_comparison(&require.condition).is_some() {
                continue;
            }
            let holds = self.eval_bool(&require.condition, &kind_env)?;
            let message = self.format_text(&require.message, &kind_env)?;
            if !holds && report {
                let names: Vec<String> = parts.iter().map(|p| self.part_name(*p)).collect();
                self.report(
                    Diagnostic::error(
                        Code::Require,
                        format!("'{}' fails: {message}", require.decl.name),
                        Label::new(file, span),
                    )
                    .with_label(Label::with_message(
                        require.decl.file,
                        require.decl.span,
                        "the kind's rule",
                    ))
                    .with_note(format!(
                        "for {} in {part_name}'s boards ({})",
                        names.join(" and "),
                        method.name
                    ))
                    .with_help("change the joint's settings or the part's size so the rule holds"),
                );
            }
            requires.push(RequireResult {
                name: format!("{prefix}.{}", require.decl.name),
                holds,
                message,
            });
        }
        let mut extensions = Vec::new();
        if let Some(joint_def) = &kind.joint {
            for extend in &joint_def.adds {
                let by = self.eval_length(&extend.by, &kind_env)?;
                let members: Vec<PartId> = if method.across {
                    parts.clone()
                } else {
                    parts.get(extend.role).copied().into_iter().collect()
                };
                for board in members {
                    for other in parts.iter().copied().filter(|o| *o != board) {
                        let (a, b) = (self.drawn_box(board)?, self.drawn_box(other)?);
                        let Some(world) = a.touching_axis(&b) else {
                            continue;
                        };
                        let orientation = self.part_orientation(board)?;
                        if orientation.part_axis(world) != axis {
                            continue;
                        }
                        let toward_positive = a.max(world) == b.min(world);
                        let along = orientation.direction(axis).is_positive();
                        let end = if along == toward_positive {
                            End::End
                        } else {
                            End::Start
                        };
                        let entry = extra.entry(board).or_insert([ExactLength::ZERO; 2]);
                        let slot = match end {
                            End::Start => &mut entry[0],
                            End::End => &mut entry[1],
                        };
                        *slot = slot.checked_add(by).ok_or(Reported)?;
                        extensions.push(solved::Extension {
                            part,
                            axis,
                            end,
                            by: round_length(by)?,
                        });
                    }
                }
            }
            if !joint_def.uses.is_empty() && report && seam.is_none_or(|s| s == 0) {
                self.report(
                    Diagnostic::warning(
                        Code::NotSupported,
                        format!(
                            "listing the fasteners of '{}' between boards is not supported yet",
                            method.name
                        ),
                        Label::new(file, span),
                    )
                    .with_note(format!(
                        "{part_name} is made of boards; the hardware list leaves out what their joints use"
                    ))
                    .with_help("add the fasteners to your shopping list by hand"),
                );
            }
            let has_members = kind
                .bodies
                .iter()
                .any(|body| matches!(body.with, stave_model::defs::Carrier::Contact(_)));
            if has_members && report && seam.is_none_or(|s| s == 0) {
                self.report(
                    Diagnostic::warning(
                        Code::NotSupported,
                        format!(
                            "placing the members of '{}' between boards is not supported yet",
                            method.name
                        ),
                        Label::new(file, span),
                    )
                    .with_note(format!(
                        "{part_name} is made of boards; the pieces their joints place are not drawn, drilled or listed"
                    ))
                    .with_help("lay out and drill the holes for them between the boards by hand"),
                );
            }
        }
        let contact = match parts.as_slice() {
            [p, q] if !method.across => {
                match self.drawn_box(*p)?.touching_axis(&self.drawn_box(*q)?) {
                    Some(axis) => Contact::Butt { axis },
                    None => Contact::Cross,
                }
            }
            _ => Contact::Connection,
        };
        Ok(PairWork {
            parts,
            across: method.across,
            contact,
            env,
            extensions,
        })
    }

    /// Pass B of one board joint: the parts it makes and its cuts.
    #[allow(clippy::too_many_arguments)]
    fn board_pass_b(
        &mut self,
        part: PartId,
        method: &Method,
        application: &mut Application,
        geoms: &mut GeomTable,
        cuts: &mut Vec<Vec<Cut>>,
        memberships: &mut Vec<Vec<JointId>>,
        made: &mut Vec<MadeInfo>,
        board_cuts: &mut BTreeMap<PartId, Vec<Cut>>,
    ) -> Solved<()> {
        let Some(kind) = self.model.defs.kind(method.kind).cloned() else {
            return Err(Reported);
        };
        let Some(ctx) = self.module_ctx(DefId::Kind(method.kind)) else {
            return Err(Reported);
        };
        let joint_def = kind.joint.clone().unwrap_or_default();
        let part_name = self.part_name(part);
        let assembly = self.model.parts.get(part.index()).and_then(|p| p.assembly);
        let origin = PartOrigin::Boards {
            part,
            seam: application.seam,
        };
        let pair = &mut application.pair;
        let mut made_ids = Vec::new();
        for made_def in &joint_def.makes {
            let id = PartId::from_usize(self.model.parts.len() + made.len());
            let name = match application.seam {
                Some(seam) => format!("{part_name}.{}[{}]", made_def.name, seam.saturating_add(1)),
                None => format!("{part_name}.{}", made_def.name),
            };
            let env = Env {
                ctx,
                file: kind.decl.file,
                kind: Some(&pair.env),
                array: None,
            };
            let placed = self.place_made_part(made_def, pair, geoms, &env, &name)?;
            let note = match &made_def.body.note {
                Some(text) => Some(self.format_text(text, &env)?),
                None => None,
            };
            geoms.set(id, placed.geom);
            if cuts.len() <= id.index() {
                cuts.resize(id.index() + 1, Vec::new());
                memberships.resize(id.index() + 1, Vec::new());
            }
            made.push(MadeInfo {
                origin,
                assembly,
                name,
                stock: placed.stock,
                note,
            });
            made_ids.push(id);
        }
        pair.env.made = made_ids;
        let env = Env {
            ctx,
            file: kind.decl.file,
            kind: Some(&pair.env),
            array: None,
        };
        let source = CutSource::Boards {
            part,
            seam: application.seam,
        };
        let owner = |id: PartId| -> PartId { if is_board(id) { part } else { id } };
        for cut_def in &joint_def.cuts {
            for (target, face, place, cut_kind) in self.pair_cuts(cut_def, pair, geoms, &env)? {
                let place = match place {
                    CutPlace::End { toward } => CutPlace::End {
                        toward: owner(toward),
                    },
                    CutPlace::Side {
                        from,
                        along_from,
                        across_axis,
                        across_from,
                    } => CutPlace::Side {
                        from: owner(from),
                        along_from,
                        across_axis,
                        across_from,
                    },
                    CutPlace::Crossing { other, footprint } => CutPlace::Crossing {
                        other: owner(other),
                        footprint,
                    },
                    member @ CutPlace::Member { .. } => member,
                };
                let cut = Cut {
                    source,
                    name: cut_def.name.clone(),
                    face,
                    place,
                    kind: cut_kind,
                };
                if is_board(target) {
                    if !made_on_boards(&cut.kind) {
                        self.not_on_boards(&cut.kind, &part_name, cut_def.span, kind.decl.file);
                    }
                    board_cuts.entry(target).or_default().push(cut);
                } else if let Some(list) = cuts.get_mut(target.index()) {
                    list.push(cut);
                }
            }
        }
        Ok(())
    }

    /// Warns that the geometry can't make a cut on a board yet.
    fn not_on_boards(&mut self, kind: &CutKind, part_name: &str, span: Span, file: FileId) {
        let noun = match kind.operation() {
            CutOperation::Notch => "a notch",
            CutOperation::Slot => "a slot",
            CutOperation::Pocket => "a mortise",
            CutOperation::Tenon => "a tenon",
            CutOperation::Spigot => "a round tenon",
            CutOperation::Round => "a round section",
            CutOperation::Drill => "a hole",
            CutOperation::DovetailSocket => "a dovetail socket",
            CutOperation::DovetailTail => "a dovetail tail",
            CutOperation::Outline => "an outline",
        };
        self.report(
            Diagnostic::warning(
                Code::NotSupported,
                format!("cutting {noun} in a board of a wide part is not supported yet"),
                Label::new(file, span),
            )
            .with_note(format!(
                "{part_name} is made of boards; this cut is listed on its board but not drawn or checked"
            ))
            .with_help("check this joint's fit by hand on a test piece before cutting the real parts"),
        );
    }
}

/// `true` for the cuts the geometry makes on a board: the box-shaped ones
/// and the angled ones (grammar, "Boards": the collision checks see a
/// part's wood as its boards make it).
pub(crate) fn made_on_boards(kind: &CutKind) -> bool {
    match kind {
        CutKind::Notch { .. }
        | CutKind::Pocket { .. }
        | CutKind::Tenon { .. }
        | CutKind::Slot { .. }
        | CutKind::DovetailSocket { .. }
        | CutKind::DovetailTail { .. }
        | CutKind::Outline { .. } => true,
        CutKind::Spigot { .. } | CutKind::Round { .. } | CutKind::Drill { .. } => false,
    }
}
