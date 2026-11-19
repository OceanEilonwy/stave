//! Joints: binding kind calls, finding pairs, what joints add to their
//! parts, the parts they make and the cuts they describe.

use stave_lang::ast::{BaseKind, Span};
use stave_lang::placement::{Axis, Face, PartAxis};
use stave_lang::units::{ExactLength, Length, Rational};
use stave_lang::{Code, Diagnostic, FileId, Label};
use stave_model::defs::{
    CutDef, DefId, JointMember, KindCallDef, MadePartDef, MadeTermKind, OperationDef, RExprKind,
    SelectorDef, Size, StockField, Symbol, Word,
};
use stave_model::ids::{JointId, KindId, PartId, RecipeId, StockId};
use stave_model::solved::{
    AssemblyStep, Contact, CutKind, CutPlace, Depth, End, LocalBox, LocalFace, LocalPoint, Mark,
    Orientation, Slope, StepAmount, StepKind,
};
use stave_model::value::{ExactBox, KindValue, Value};
use stave_schema::CutOperation;
use stave_schema::kinds::RoleType;
use stave_schema::operations::Selector;
use stave_schema::placement::AssemblyStepKind;
use stave_schema::words::{Grain, StockForm};

use crate::eval::{Env, KindEnv, midpoint};
use crate::{Failure, Fault, Frame, Memo, Solved, Solver};

/// A kind call with its kind, roles and settings worked out.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Binding {
    pub kind: KindId,
    /// The kind's name as written where it was named.
    pub name: String,
    pub roles: Vec<Value>,
    pub settings: Vec<(String, Value)>,
    pub glue: bool,
    pub methods: Vec<RecipeId>,
    pub face: Option<Face>,
}

/// Who a cut belongs to and where to report a mistake in its sizes: a
/// joint's name and its declaration.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CutAt<'a> {
    pub owner: &'a str,
    pub file: FileId,
    pub span: Span,
}

/// What a joint adds to one end of a part.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Extend {
    pub axis: PartAxis,
    pub end: End,
    pub by: ExactLength,
}

/// A part's geometry once every joint has added to it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PartGeom {
    pub orientation: Orientation,
    /// Drawn size per local axis (length, width, thickness).
    pub drawn: [ExactLength; 3],
    /// What is added at the start of each local axis.
    pub start: [ExactLength; 3],
    /// What is added at the end of each local axis.
    pub end: [ExactLength; 3],
    pub drawn_box: ExactBox,
    pub cut_box: ExactBox,
    pub reference_face: LocalFace,
    pub reference_edge: LocalFace,
}

/// The axis of a part its grain runs along: its length, or its width for
/// `grain: width` (grammar, "part").
pub(crate) fn grain_axis(grain: Grain) -> PartAxis {
    match grain {
        Grain::Length => PartAxis::Length,
        Grain::Width => PartAxis::Width,
    }
}

pub(crate) fn local_slot(axis: PartAxis) -> usize {
    match axis {
        PartAxis::Length => 0,
        PartAxis::Width => 1,
        PartAxis::Thickness => 2,
    }
}

impl PartGeom {
    /// Cut size along a local axis.
    pub fn cut_size(&self, axis: PartAxis) -> Solved<ExactLength> {
        let slot = local_slot(axis);
        let (d, s, e) = (
            self.drawn.get(slot),
            self.start.get(slot),
            self.end.get(slot),
        );
        match (d, s, e) {
            (Some(d), Some(s), Some(e)) => d
                .checked_add(*s)
                .and_then(|x| x.checked_add(*e))
                .ok_or(Failure::Fault(Fault::Overflow)),
            _ => Err(Fault::Internal("a part's size").into()),
        }
    }

    /// World coordinate of local 0 along the world axis of `axis`.
    fn origin(&self, axis: PartAxis) -> ExactLength {
        let direction = self.orientation.direction(axis);
        if direction.is_positive() {
            self.cut_box.min(direction.axis())
        } else {
            self.cut_box.max(direction.axis())
        }
    }

    /// A world coordinate along `axis`'s world axis, as a local one.
    pub fn local(&self, axis: PartAxis, world: ExactLength) -> Solved<ExactLength> {
        let origin = self.origin(axis);
        let value = if self.orientation.direction(axis).is_positive() {
            world.checked_sub(origin)
        } else {
            origin.checked_sub(world)
        };
        value.ok_or(Failure::Fault(Fault::Overflow))
    }

    /// A world box in this part's frame.
    pub fn local_box(&self, world: &ExactBox) -> Solved<[(ExactLength, ExactLength); 3]> {
        let mut out = [(ExactLength::ZERO, ExactLength::ZERO); 3];
        for axis in PartAxis::ALL {
            let w = self.orientation.direction(axis).axis();
            let a = self.local(axis, world.min(w))?;
            let b = self.local(axis, world.max(w))?;
            if let Some(slot) = out.get_mut(local_slot(axis)) {
                *slot = if a <= b { (a, b) } else { (b, a) };
            }
        }
        Ok(out)
    }

    /// The local face on the `positive` side of a world axis.
    pub fn face_toward(&self, world: Axis, positive: bool) -> LocalFace {
        let axis = self.orientation.part_axis(world);
        let along = self.orientation.direction(axis).is_positive();
        LocalFace {
            axis,
            end: if along == positive {
                End::End
            } else {
                End::Start
            },
        }
    }

    /// The world plane of a local face.
    pub fn face_plane(&self, face: LocalFace) -> (Axis, ExactLength, bool) {
        let direction = self.orientation.direction(face.axis);
        let positive = direction.is_positive() == (face.end == End::End);
        let world = direction.axis();
        let position = if positive {
            self.cut_box.max(world)
        } else {
            self.cut_box.min(world)
        };
        (world, position, positive)
    }
}

/// A joint's pairs while it is worked out.
pub(crate) struct JointWork {
    pub binding: Binding,
    pub pairs: Vec<PairWork>,
    pub unmatched: Vec<PartId>,
    pub requires: Vec<Vec<stave_model::solved::RequireResult>>,
    /// The cuts its members make, in the world, once its pairs are
    /// finished: a mirrored assembly cuts them again, reflected, in the
    /// parts outside it.
    pub member_cuts: Vec<crate::members::WorldMemberCut>,
}

/// One pair of a joint.
pub(crate) struct PairWork {
    /// One part per role; for a kind with one `Parts` role, every part of
    /// that role.
    pub parts: Vec<PartId>,
    /// `true` for a kind with one `Parts` role: one connection across all
    /// of `parts`.
    pub across: bool,
    pub contact: Contact,
    pub env: KindEnv,
    pub extensions: Vec<stave_model::solved::Extension>,
    /// Whether the joint makes its cuts, parts and members for this pair.
    pub making: Making,
}

/// Whether a joint makes anything for one of its pairs (grammar, "Checks
/// and `require`"): a pair whose parts don't meet the way one of the
/// kind's rules asks (`edge_to_edge(piece, into)`) is not a place for the
/// joint at all, so nothing is cut, made or added there, and that rule's
/// error is the only one the pair gives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Making {
    /// The joint makes its cuts, parts and members.
    Made,
    /// The parts don't meet as the kind needs: nothing is made.
    Withheld,
}

fn flip(end: End) -> End {
    match end {
        End::Start => End::End,
        End::End => End::Start,
    }
}

impl Solver<'_> {
    /// The kind, roles and settings of a joint or hardware call.
    pub(crate) fn bind_call(
        &mut self,
        call: &KindCallDef,
        env: &Env<'_>,
        what: &str,
        owner: DefId,
    ) -> Solved<Binding> {
        let kind_value = match self.eval(&call.kind, env)? {
            Value::Kind(kind) => kind,
            other => {
                return Err(self
                    .error(
                        Code::TypeMismatch,
                        format!("{what} needs a kind, found {}", other.description()),
                        env.file,
                        call.kind.span,
                        "name a joint or hardware kind",
                    )
                    .into());
            }
        };
        let KindValue {
            kind,
            name: kind_name,
            settings: given,
        } = kind_value;
        if self.model.defs.is_poisoned(DefId::Kind(kind)) {
            return Err(self.skip().into());
        }
        let Some(kind_def) = self.model.defs.kind(kind).cloned() else {
            return Err(Fault::Internal("a kind's declaration").into());
        };
        // A kind chosen through a param that doesn't take what this call
        // hands it is one error, in the words of the choice.
        self.check_kind_signature(call, env, kind, &kind_name, owner)?;
        let mut roles: Vec<Option<Value>> = vec![None; kind_def.roles.len()];
        let mut settings = given;
        let mut glue = true;
        let mut methods = Vec::new();
        let mut face = None;
        if let Some(positional) = &call.positional {
            let value = self.eval(positional, env)?;
            match roles.first_mut() {
                Some(slot) => *slot = Some(value),
                None => {
                    return Err(self
                        .error(
                            Code::TypeMismatch,
                            format!("the kind '{}' has no roles", kind_name),
                            env.file,
                            positional.span,
                            "a kind value takes its settings by name",
                        )
                        .into());
                }
            }
        }
        for setting in &call.named {
            let value = self.eval(&setting.value, env)?;
            if let Some(index) = kind_def.role(&setting.name) {
                if let Some(slot) = roles.get_mut(index) {
                    *slot = Some(value);
                }
                continue;
            }
            match (setting.name.as_str(), kind_def.base) {
                ("glue", BaseKind::Joint) => {
                    glue = value == Value::Bool(true);
                    continue;
                }
                ("methods", BaseKind::Joint) => {
                    if let Value::List(items) = &value {
                        methods.extend(items.iter().filter_map(|v| match v {
                            Value::Recipe(r) => Some(*r),
                            _ => None,
                        }));
                    } else if let Value::Recipe(r) = value {
                        methods.push(r);
                    }
                    continue;
                }
                ("face", base) if base.is_mounted_hardware() => {
                    if let Value::Word(Word::Face(f)) = value {
                        face = Some(f);
                    }
                    continue;
                }
                _ => {}
            }
            if kind_def.param(&setting.name).is_none() {
                return Err(self
                    .error(
                        Code::UnknownField,
                        format!("the kind '{}' has no field '{}'", kind_name, setting.name),
                        env.file,
                        setting.name_span,
                        stave_model::suggest::did_you_mean(
                            &setting.name,
                            kind_def
                                .params
                                .iter()
                                .map(|p| p.decl.name.as_str())
                                .chain(kind_def.roles.iter().map(|r| r.name.as_str())),
                        )
                        .unwrap_or_else(|| "name one of the kind's roles or params".to_owned()),
                    )
                    .into());
            }
            if settings.iter().any(|(n, _)| *n == setting.name) {
                return Err(self
                    .error(
                        Code::DuplicateName,
                        format!("'{}' is declared twice", setting.name),
                        env.file,
                        setting.name_span,
                        "the kind value already gives this setting",
                    )
                    .into());
            }
            settings.push((setting.name.clone(), value));
        }
        // A setting must be within its param's `within`.
        if let Some(ctx) = self.module_ctx(DefId::Kind(kind)) {
            for (name, value) in &settings {
                if let Some(param) = kind_def.params.iter().find(|p| p.decl.name == *name) {
                    let written = call.named.iter().find(|s| s.name == *name);
                    let span = written.map_or(call.span, |s| s.value.span);
                    // A setting written in this call is set by a patch
                    // on the declaration; one given by a kind value
                    // elsewhere is not.
                    let target = written
                        .and_then(|_| self.model.defs.patch_path(owner, self.model.root))
                        .map(|path| format!("{path}.{name}"));
                    let written = crate::fixes::Written {
                        file: env.file,
                        span,
                        path: target,
                    };
                    self.check_domain(param, value, written, name, ctx)?;
                }
            }
        }
        let mut bound = Vec::new();
        for (index, role) in roles.into_iter().enumerate() {
            let Some(role_def) = kind_def.roles.get(index) else {
                continue;
            };
            let Some(value) = role else {
                return Err(self
                    .error(
                        Code::MissingField,
                        format!("{what} is missing '{}'", role_def.name),
                        env.file,
                        call.span,
                        format!("name the part for '{}' in the call", role_def.name),
                    )
                    .into());
            };
            let fits = match role_def.ty {
                RoleType::Part => matches!(value, Value::Part(_)),
                RoleType::Parts => value.parts().is_some(),
                RoleType::Region => matches!(value, Value::Region(_)),
            };
            if !fits {
                let wanted = match role_def.ty {
                    RoleType::Part => "a part",
                    RoleType::Parts => "parts",
                    RoleType::Region => "a region",
                };
                return Err(self
                    .error(
                        Code::TypeMismatch,
                        format!(
                            "'{}' needs {wanted}, found {}",
                            role_def.name,
                            value.description()
                        ),
                        env.file,
                        call.span,
                        "check the role's argument",
                    )
                    .into());
            }
            bound.push(value);
        }
        Ok(Binding {
            name: kind_name,
            kind,
            roles: bound,
            settings,
            glue,
            methods,
            face,
        })
    }

    /// The binding of a joint, worked out once.
    pub(crate) fn joint_binding(&mut self, joint: JointId) -> Solved<Binding> {
        match self.joints.get(joint.index()) {
            Some(Memo::Done(binding)) => return Ok(binding.clone()),
            Some(Memo::Failed(accounted)) => return Err((*accounted).into()),
            Some(Memo::Busy) => return Err(self.cycle(Frame::Joint(joint)).into()),
            _ => {}
        }
        let Some(inst) = self.model.joints.get(joint.index()).cloned() else {
            return Err(Fault::Internal("a joint").into());
        };
        let Some(def) = self.model.defs.joint(inst.def).cloned() else {
            return Err(Fault::Internal("a joint's declaration").into());
        };
        if self.poisoned(DefId::Joint(inst.def)) {
            return Err(self.skip().into());
        }
        if let Some(slot) = self.joints.get_mut(joint.index()) {
            *slot = Memo::Busy;
        }
        self.enter(Frame::Joint(joint), Some((def.decl.file, def.call.span)));
        let array = def.array.as_ref().map(|_| {
            (
                DefId::Joint(inst.def),
                self.array_len(DefId::Joint(inst.def), inst.ctx),
            )
        });
        let env = Env {
            ctx: inst.ctx,
            file: def.decl.file,
            kind: None,
            array,
        };
        let what = format!("joint '{}'", inst.name);
        let result = self
            .bind_call(&def.call, &env, &what, DefId::Joint(inst.def))
            .and_then(|binding| {
                let base = self.model.defs.kind(binding.kind).map(|k| k.base);
                if base == Some(BaseKind::Joint) {
                    Ok(binding)
                } else {
                    Err(self
                        .error(
                            Code::TypeMismatch,
                            format!("'{}' needs a joint kind", inst.name),
                            def.decl.file,
                            def.call.kind.span,
                            "use a kind declared with kind name : joint",
                        )
                        .into())
                }
            });
        self.leave();
        let (memo, result) = self.remember(result, def.decl.file, def.call.span);
        if let Some(slot) = self.joints.get_mut(joint.index()) {
            *slot = memo;
        }
        result
    }

    pub(crate) fn joint_place(&self, joint: JointId) -> (FileId, Span) {
        let def = self
            .model
            .joints
            .get(joint.index())
            .and_then(|j| self.model.defs.joint(j.def));
        match def {
            Some(def) => (def.decl.file, def.call.span),
            None => (self.project_file, self.project_span),
        }
    }

    /// One mistake, one error: when another pair of the same joint already
    /// broke this rule (the same message at the same call), that error
    /// gets a note `the same for <pair>` and `true` comes back.
    fn noted_with_earlier_pair(
        &mut self,
        file: FileId,
        span: Span,
        message: &str,
        pair: &str,
    ) -> bool {
        let earlier = self.diagnostics.iter_mut().find(|diagnostic| {
            diagnostic.code == Code::Require
                && diagnostic.message == message
                && diagnostic
                    .primary()
                    .is_some_and(|label| label.file == file && label.span == span)
        });
        let Some(earlier) = earlier else {
            return false;
        };
        let note = format!("the same for {pair}");
        if !earlier.notes.contains(&note) {
            earlier.notes.push(note);
        }
        true
    }

    /// Pass A of a joint: pairs, requires and what it adds to its parts.
    pub(crate) fn work_joint(&mut self, joint: JointId) -> Solved<JointWork> {
        let binding = self.joint_binding(joint)?;
        let Some(kind) = self.model.defs.kind(binding.kind).cloned() else {
            return Err(Fault::Internal("a kind's declaration").into());
        };
        let (file, span) = self.joint_place(joint);
        let name = self
            .model
            .joints
            .get(joint.index())
            .map(|j| j.name.clone())
            .unwrap_or_default();
        let has_parts = kind.roles.iter().any(|r| r.ty == RoleType::Parts);
        // Two roles with a `Parts` role: pairs. One `Parts` role: one
        // connection across all of its parts (grammar, "Roles and pairs").
        let pairwise = has_parts && kind.roles.len() == 2;
        let across = has_parts && kind.roles.len() == 1;
        let mut plans: Vec<(Vec<PartId>, Contact)> = Vec::new();
        let mut unmatched = Vec::new();
        let role_parts: Vec<Vec<PartId>> = binding
            .roles
            .iter()
            .map(|v| v.parts().unwrap_or_default())
            .collect();
        if pairwise {
            let (Some(first), Some(second)) = (role_parts.first(), role_parts.get(1)) else {
                return Err(Fault::Earlier.into());
            };
            let mut met: Vec<PartId> = Vec::new();
            for p in first {
                for q in second {
                    if p == q {
                        continue;
                    }
                    let (a, b) = (self.drawn_box(*p)?, self.drawn_box(*q)?);
                    if a.meets(&b) {
                        let contact = match a.touching_axis(&b) {
                            Some(axis) => Contact::Butt { axis },
                            None => Contact::Cross,
                        };
                        plans.push((vec![*p, *q], contact));
                        met.push(*p);
                        met.push(*q);
                    }
                }
            }
            for part in first.iter().chain(second) {
                if !met.contains(part) && !unmatched.contains(part) {
                    unmatched.push(*part);
                }
            }
        } else if across {
            let parts = role_parts.first().cloned().unwrap_or_default();
            for (index, part) in parts.iter().enumerate() {
                let mut meets = false;
                for (other_index, other) in parts.iter().enumerate() {
                    if index != other_index
                        && self.drawn_box(*part)?.meets(&self.drawn_box(*other)?)
                    {
                        meets = true;
                        break;
                    }
                }
                if !meets && !unmatched.contains(part) {
                    unmatched.push(*part);
                }
            }
            plans.push((parts, Contact::Connection));
        } else {
            let parts: Vec<PartId> = role_parts
                .iter()
                .filter_map(|p| p.first().copied())
                .collect();
            let contact = match parts.as_slice() {
                [p, q] => {
                    let (a, b) = (self.drawn_box(*p)?, self.drawn_box(*q)?);
                    if a.meets(&b) {
                        match a.touching_axis(&b) {
                            Some(axis) => Contact::Butt { axis },
                            None => Contact::Cross,
                        }
                    } else {
                        Contact::Connection
                    }
                }
                _ => Contact::Connection,
            };
            plans.push((parts, contact));
        }
        let mut pairs = Vec::new();
        let mut requires = Vec::new();
        for (parts, contact) in plans {
            let roles: Vec<Value> = if across {
                vec![Value::Parts(parts.clone())]
            } else {
                parts.iter().map(|p| Value::Part(*p)).collect()
            };
            let env = KindEnv::new(
                binding.kind,
                roles,
                binding.settings.clone(),
                kind.params.len(),
            );
            let Some(ctx) = self.module_ctx(DefId::Kind(binding.kind)) else {
                return Err(Fault::Internal("the file an item is declared in").into());
            };
            let kind_env = Env {
                ctx,
                file: kind.decl.file,
                kind: Some(&env),
                array: None,
            };
            // Requires, at the current values, for this pair.
            let mut results = Vec::new();
            let source = stave_model::solved::CutSource::Joint {
                joint,
                pair: pairs.len(),
            };
            // A rule that asks how the parts meet comes first; when one
            // fails, the joint is not for this pair and nothing else of
            // it is checked or made here.
            let mut order: Vec<(usize, &stave_model::defs::RequireDef)> =
                kind.requires.iter().enumerate().collect();
            order.sort_by_key(|(_, require)| {
                !crate::meeting::asks_how_parts_meet(&require.condition)
            });
            let mut making = Making::Made;
            for (index, require) in order {
                if self.defer_wall_rule(source, binding.kind, index, require, &kind_env)? {
                    continue;
                }
                let holds = self.eval_bool(&require.condition, &kind_env)?;
                let message = self.format_text(&require.message, &kind_env)?;
                let meeting_rule = crate::meeting::asks_how_parts_meet(&require.condition);
                if !holds {
                    let names: Vec<String> = parts.iter().map(|p| self.part_name(*p)).collect();
                    let pair = format!("{} in {name}", names.join(" and "));
                    let message = format!("'{}' fails: {message}", require.decl.name);
                    // One mistake, one error: another pair of this joint
                    // that breaks the same rule is a note on the first.
                    if !self.noted_with_earlier_pair(file, span, &message, &pair) {
                        let mut diagnostic =
                            Diagnostic::error(Code::Require, message, Label::new(file, span))
                                .with_label(Label::with_message(
                                    require.decl.file,
                                    require.decl.span,
                                    "the kind's rule",
                                ))
                                .with_note(format!("for {pair}"));
                        if let Some(note) = self.measures_note(&require.condition, &kind_env) {
                            diagnostic = diagnostic.with_note(note);
                        }
                        if let Some(note) = self.meeting_note(&require.condition, &kind_env) {
                            diagnostic = diagnostic.with_note(note);
                        }
                        let help = self.require_help(&require.condition, &kind_env, "joint");
                        self.report(diagnostic.with_help(help));
                    }
                }
                results.push(stave_model::solved::RequireResult {
                    name: format!("{name}.{}", require.decl.name),
                    holds,
                    message,
                });
                if !holds && meeting_rule {
                    making = Making::Withheld;
                    break;
                }
            }
            requires.push(results);
            // What the joint adds.
            let mut extensions = Vec::new();
            if let (Some(joint_def), Making::Made) = (&kind.joint, making) {
                for extend in &joint_def.adds {
                    let by = self.eval_length(&extend.by, &kind_env)?;
                    let members: Vec<PartId> = if across {
                        parts.clone()
                    } else {
                        parts.get(extend.role).copied().into_iter().collect()
                    };
                    for (part, other) in members.iter().flat_map(|part| {
                        parts
                            .iter()
                            .filter(move |other| *other != part)
                            .map(move |other| (*part, *other))
                    }) {
                        let (a, b) = (self.drawn_box(part)?, self.drawn_box(other)?);
                        if !a.meets(&b) {
                            continue;
                        }
                        let Some(axis) = a.touching_axis(&b) else {
                            let (pn, qn) = (self.part_name(part), self.part_name(other));
                            return Err(self.error(Code::TypeMismatch, format!("'extend' needs parts that butt, but {pn} and {qn} cross"), file, span, "a joint whose parts cross can't lengthen them; draw them meeting face to face").into());
                        };
                        let toward_positive = a.max(axis) == b.min(axis);
                        let orientation = self.part_orientation(part)?;
                        let local = orientation.part_axis(axis);
                        // `extend` lengthens an end or an edge, never a
                        // face: a part laid face to face on its
                        // counterpart would come out thicker than any
                        // board of its stock (grammar, "adds:").
                        if local == PartAxis::Thickness {
                            let (pn, qn) = (self.part_name(part), self.part_name(other));
                            return Err(self
                                .error(
                                    Code::TypeMismatch,
                                    format!(
                                        "'{name}' would make {pn} thicker: {pn} meets {qn} with its face, but extend lengthens an end or an edge"
                                    ),
                                    file,
                                    span,
                                    format!(
                                        "place {pn} so its end or edge meets {qn}, or use a kind without adds:"
                                    ),
                                )
                                .into());
                        }
                        let along = orientation.direction(local).is_positive();
                        let end = if along == toward_positive {
                            End::End
                        } else {
                            End::Start
                        };
                        if let Some(list) = self.extensions.get_mut(part.index()) {
                            list.push(Extend {
                                axis: local,
                                end,
                                by,
                            });
                        }
                        extensions.push(stave_model::solved::Extension {
                            part,
                            axis: local,
                            end,
                            by: round_length(by)?,
                        });
                    }
                }
            }
            pairs.push(PairWork {
                parts,
                across,
                contact,
                env,
                extensions,
                making,
            });
        }
        Ok(JointWork {
            binding,
            pairs,
            unmatched,
            requires,
            member_cuts: Vec::new(),
        })
    }

    /// Geometry of a declared part, once joints have added to it.
    pub(crate) fn part_geom(&mut self, part: PartId) -> Solved<PartGeom> {
        let orientation = self.part_orientation(part)?;
        let mut drawn = [ExactLength::ZERO; 3];
        for axis in PartAxis::ALL {
            if let Some(slot) = drawn.get_mut(local_slot(axis)) {
                *slot = self.drawn_size(part, axis)?;
            }
        }
        let drawn_box = self.drawn_box(part)?;
        let mut start = [ExactLength::ZERO; 3];
        let mut end = [ExactLength::ZERO; 3];
        let extends = match self.model.parts.get(part.index()).and_then(|p| p.mirror_of) {
            // A part of a mirrored assembly has its source's extensions:
            // its frame is the reflection, so local ends match.
            Some(source) => self
                .extensions
                .get(source.index())
                .cloned()
                .unwrap_or_default(),
            None => self
                .extensions
                .get(part.index())
                .cloned()
                .unwrap_or_default(),
        };
        for extend in extends {
            let slot = local_slot(extend.axis);
            let target = if extend.end == End::Start {
                start.get_mut(slot)
            } else {
                end.get_mut(slot)
            };
            if let Some(target) = target {
                *target = target.checked_add(extend.by).ok_or(Fault::Overflow)?;
            }
        }
        let mut cut_box = drawn_box;
        for axis in PartAxis::ALL {
            let slot = local_slot(axis);
            let direction = orientation.direction(axis);
            let world = direction.axis();
            let (s, e) = (
                start.get(slot).copied().unwrap_or(ExactLength::ZERO),
                end.get(slot).copied().unwrap_or(ExactLength::ZERO),
            );
            let (low_add, high_add) = if direction.is_positive() {
                (s, e)
            } else {
                (e, s)
            };
            let low = cut_box
                .min(world)
                .checked_sub(low_add)
                .ok_or(Fault::Overflow)?;
            let high = cut_box
                .max(world)
                .checked_add(high_add)
                .ok_or(Fault::Overflow)?;
            cut_box = cut_box.with(world, low, high);
        }
        let (reference_face, reference_edge) = self.reference_faces(part, &orientation)?;
        Ok(PartGeom {
            orientation,
            drawn,
            start,
            end,
            drawn_box,
            cut_box,
            reference_face,
            reference_edge,
        })
    }

    /// A part's cut length along its grain must not be more than its
    /// stock's `max_length` (grammar, "stock"): its length, or its width
    /// when its grain runs across it. Mirror images are left to their
    /// source, so one mistake is one error.
    pub(crate) fn check_max_length(&mut self, part: PartId, geom: &PartGeom) -> Solved<()> {
        if self.mirror_source(part)?.is_some()
            || self
                .model
                .parts
                .get(part.index())
                .is_some_and(|p| p.mirror_of.is_some())
        {
            return Ok(());
        }
        // A spliced part's pieces are measured once they are cut
        // (grammar, "Splices").
        if self.part_body(part).is_some_and(|b| b.lengths.is_some()) {
            return Ok(());
        }
        let stock = self.part_stock(part)?;
        let Some(max_length) =
            self.stock_length(stock, stave_model::defs::StockField::MaxLength)?
        else {
            return Ok(());
        };
        let grain = self
            .part_body(part)
            .map_or(Grain::Length, |body| body.grain);
        let axis = grain_axis(grain);
        let cut_length = geom.cut_size(axis)?;
        if cut_length <= max_length {
            return Ok(());
        }
        let across = geom.cut_size(match axis {
            PartAxis::Length => PartAxis::Width,
            PartAxis::Width | PartAxis::Thickness => PartAxis::Length,
        })?;
        if self.sheet_fits_turned(stock, cut_length, across)? {
            return Ok(());
        }
        let name = self.part_name(part);
        let stock_name = self.stock_name(stock);
        let max = stave_model::value::format_length(max_length);
        let (file, span) = match (self.size_span(part, axis), self.part_env(part)) {
            (Some(span), Some(env)) => (env.file, span),
            _ => self.part_place(part),
        };
        let (message, help) = match grain {
            Grain::Length => {
                let drawn = geom
                    .drawn
                    .get(local_slot(axis))
                    .copied()
                    .unwrap_or(cut_length);
                let help = match self.splice_suggestion(part, drawn, max_length) {
                    Some(splice) => format!(
                        "stock '{stock_name}' comes no longer than {max}: splice it end to end with {splice}, choosing the end joint and leaving each piece room for what its splices add, or buy longer stock"
                    ),
                    None => format!(
                        "stock '{stock_name}' comes no longer than {max}: splice it end to end with lengths:, or buy longer stock"
                    ),
                };
                (
                    format!(
                        "{name}: its cut length works out as {}, which is more than its stock's max_length of {max}",
                        stave_model::value::format_length(cut_length)
                    ),
                    help,
                )
            }
            Grain::Width => (
                format!(
                    "{name}: its grain runs across its width, and its width works out as {}, which is more than its stock's max_length of {max}",
                    stave_model::value::format_length(cut_length)
                ),
                format!(
                    "stock '{stock_name}' comes no longer than {max}, measured along the grain: make the part narrower, let its grain run along its length, or buy longer stock"
                ),
            ),
        };
        Err(self
            .error(Code::TypeMismatch, message, file, span, help)
            .into())
    }

    /// `true` when a part cut from a sheet (plywood, MDF) fits it turned a
    /// quarter turn: its size `along` its grain within the sheet's
    /// `max_width` and its size `across` within its `max_length`. A part
    /// may lie either way on a sheet (grammar, "stock").
    pub(crate) fn sheet_fits_turned(
        &mut self,
        stock: StockId,
        along: ExactLength,
        across: ExactLength,
    ) -> Solved<bool> {
        let sheet = match self.model.defs.stock(stock).map(|def| def.form) {
            Some(StockForm::Plywood | StockForm::Mdf) => true,
            Some(StockForm::Solid | StockForm::Dowel) | None => false,
        };
        if !sheet {
            return Ok(false);
        }
        let (Some(max_width), Some(max_length)) = (
            self.stock_length(stock, StockField::MaxWidth)?,
            self.stock_length(stock, StockField::MaxLength)?,
        ) else {
            return Ok(false);
        };
        Ok(along <= max_width && across <= max_length)
    }

    /// A part's cut thickness against its stock's: nothing may make a part
    /// thicker than the board it is cut from (grammar, "part"). The
    /// `thickness:` field is checked where it is written; this catches what
    /// joints add. A turned part is checked against its blank at `shape:`.
    pub(crate) fn check_cut_thickness(&mut self, part: PartId, geom: &PartGeom) -> Solved<()> {
        if self.mirror_source(part)?.is_some()
            || self
                .model
                .parts
                .get(part.index())
                .is_some_and(|p| p.mirror_of.is_some())
            || self.part_profile(part)?.is_some()
        {
            return Ok(());
        }
        let stock = self.part_stock(part)?;
        let Some(stock_thickness) =
            self.stock_length(stock, stave_model::defs::StockField::Thickness)?
        else {
            return Ok(());
        };
        let cut_thickness = geom.cut_size(PartAxis::Thickness)?;
        if cut_thickness <= stock_thickness {
            return Ok(());
        }
        let name = self.part_name(part);
        let stock_name = self
            .model
            .defs
            .stock(stock)
            .map(|s| s.decl.name.clone())
            .unwrap_or_default();
        let (file, span) = self.part_place(part);
        let reason = format!(
            "its cut thickness works out as {}, which is more than its stock's {}",
            stave_model::value::format_length(cut_thickness),
            stave_model::value::format_length(stock_thickness)
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
                    "a part can be planed thinner than its stock '{stock_name}', never thicker; choose thicker stock, or let the joint add to an end or an edge"
                ),
            )
            .into())
    }

    /// A part a joint makes is cut from its stock like any other part
    /// (grammar, "Joint definitions"): each size more than 0mm, never
    /// thicker than the stock, never wider than stock sold at a fixed
    /// width, and no longer along its grain than the stock's `max_length`.
    /// Reported at `at`, the joint's call, once for every made part with
    /// the same fault there.
    fn check_made_part_stock(
        &mut self,
        stock: StockId,
        name: &str,
        [length, width, thickness]: [ExactLength; 3],
        grain: Grain,
        at: (FileId, Span),
    ) -> Solved<()> {
        let (file, span) = at;
        let stock_name = self.stock_name(stock);
        let size = stave_model::value::format_length;
        let mut fault: Option<(Code, String, String)> = None;
        for (word, value) in [
            ("length", length),
            ("width", width),
            ("thickness", thickness),
        ] {
            if fault.is_none() && value <= ExactLength::ZERO {
                fault = Some((
                    Code::OutOfRange,
                    format!("its {word} works out as {}", size(value)),
                    format!("give the joint settings that make the part's {word} more than 0mm"),
                ));
            }
        }
        if fault.is_none()
            && let Some(stock_thickness) = self.stock_length(stock, StockField::Thickness)?
            && thickness > stock_thickness
        {
            fault = Some((
                Code::TypeMismatch,
                format!(
                    "its thickness works out as {}, but {stock_name} is only {} thick",
                    size(thickness),
                    size(stock_thickness)
                ),
                format!(
                    "choose thicker stock for the part with the joint's settings, or make the part at most {} thick",
                    size(stock_thickness)
                ),
            ));
        }
        let fixed_width = self
            .model
            .defs
            .stock(stock)
            .is_some_and(|def| def.width.is_some());
        if fault.is_none()
            && fixed_width
            && let Some(stock_width) = self.stock_length(stock, StockField::Width)?
            && width > stock_width
        {
            fault = Some((
                Code::TypeMismatch,
                format!(
                    "its width is {}, but {stock_name} is sold {} wide",
                    size(width),
                    size(stock_width)
                ),
                format!(
                    "choose wider stock for the part with the joint's settings, or make the part at most {} wide",
                    size(stock_width)
                ),
            ));
        }
        if fault.is_none()
            && let Some(max_length) = self.stock_length(stock, StockField::MaxLength)?
        {
            let along = match grain {
                Grain::Length => length,
                Grain::Width => width,
            };
            if along > max_length {
                fault = Some((
                    Code::TypeMismatch,
                    format!(
                        "its length along the grain works out as {}, which is more than its stock's max_length of {}",
                        size(along),
                        size(max_length)
                    ),
                    format!(
                        "stock '{stock_name}' comes no longer than {}: choose longer stock for the part with the joint's settings, or make the part shorter",
                        size(max_length)
                    ),
                ));
            }
        }
        let Some((code, reason, help)) = fault else {
            return Ok(());
        };
        if self.noted_with_earlier_part(code, file, span, &reason, name) {
            return Err(self.skip().into());
        }
        Err(self
            .error(code, format!("{name}: {reason}"), file, span, help)
            .into())
    }

    /// The reference face and edge (grammar, "Faces to cut").
    fn reference_faces(
        &mut self,
        part: PartId,
        orientation: &Orientation,
    ) -> Solved<(LocalFace, LocalFace)> {
        if let Some((source, _, _)) = self.mirror_source(part)? {
            let source_orientation = self.part_orientation(source)?;
            return self.reference_faces(source, &source_orientation);
        }
        let preferred = |axis: Axis| match axis {
            Axis::Up => stave_lang::placement::Direction::Up,
            Axis::Across => stave_lang::placement::Direction::Right,
            Axis::Deep => stave_lang::placement::Direction::Front,
        };
        let body = self.part_body(part).cloned();
        let file = self
            .part_env(part)
            .map(|e| e.file)
            .unwrap_or(self.project_file);
        let pick = |this: &mut Self,
                    axis: PartAxis,
                    chosen: Option<(Face, Span)>,
                    field: &str|
         -> Solved<LocalFace> {
            let direction = orientation.direction(axis);
            match chosen {
                Some((face, span)) => {
                    if face.axis() != direction.axis() {
                        return Err(this
                            .error(
                                Code::TypeMismatch,
                                format!(
                                    "'{field}' needs a face across the part's {}, found '{}'",
                                    axis.word(),
                                    face.word()
                                ),
                                file,
                                span,
                                format!("name one of the two faces across its {}", axis.word()),
                            )
                            .into());
                    }
                    Ok(LocalFace {
                        axis,
                        end: if face.direction() == direction {
                            End::End
                        } else {
                            End::Start
                        },
                    })
                }
                None => Ok(LocalFace {
                    axis,
                    end: if preferred(direction.axis()) == direction {
                        End::End
                    } else {
                        End::Start
                    },
                }),
            }
        };
        let face = pick(
            self,
            PartAxis::Thickness,
            body.as_ref().and_then(|b| b.face_side),
            "face_side",
        )?;
        let edge = pick(
            self,
            PartAxis::Width,
            body.as_ref().and_then(|b| b.face_edge),
            "face_edge",
        )?;
        Ok((face, edge))
    }

    // -----------------------------------------------------------------------
    // Pass B: made parts and cuts
    // -----------------------------------------------------------------------

    /// Resolves who a selector or step names in a pair.
    fn member_part(pair: &PairWork, member: JointMember) -> Option<PartId> {
        match member {
            JointMember::Role(index) => pair.parts.get(index).copied(),
            JointMember::Made(index) => pair.env.made.get(index).copied(),
        }
    }

    /// The geometry a made part's place reads for a member: its part's,
    /// or, for a kind's only `Parts` role, the box around all of its parts
    /// in the first part's axes (grammar, "Roles and pairs").
    fn member_geom(
        pair: &PairWork,
        member: JointMember,
        geoms: &GeomTable,
    ) -> Solved<(PartId, PartGeom)> {
        let part = Self::member_part(pair, member).ok_or(Fault::Earlier)?;
        let mut geom = geoms.get(part).cloned().ok_or(Fault::Earlier)?;
        if !(pair.across && member == JointMember::Role(0)) {
            return Ok((part, geom));
        }
        for other in pair.parts.iter().skip(1) {
            let other = geoms.get(*other).ok_or(Fault::Earlier)?;
            geom.drawn_box = hull(&geom.drawn_box, &other.drawn_box);
            geom.cut_box = hull(&geom.cut_box, &other.cut_box);
        }
        for axis in PartAxis::ALL {
            let direction = geom.orientation.direction(axis);
            let world = direction.axis();
            let low = geom
                .drawn_box
                .min(world)
                .checked_sub(geom.cut_box.min(world))
                .ok_or(Fault::Overflow)?;
            let high = geom
                .cut_box
                .max(world)
                .checked_sub(geom.drawn_box.max(world))
                .ok_or(Fault::Overflow)?;
            let (start, end) = if direction.is_positive() {
                (low, high)
            } else {
                (high, low)
            };
            let slot = local_slot(axis);
            let size = geom.drawn_box.size(world).ok_or(Fault::Overflow)?;
            if let (Some(d), Some(s), Some(e)) = (
                geom.drawn.get_mut(slot),
                geom.start.get_mut(slot),
                geom.end.get_mut(slot),
            ) {
                *d = size;
                *s = start;
                *e = end;
            }
        }
        Ok((part, geom))
    }

    /// The face a selector names and where on it.
    /// `target` overrides the part the selector names (one part of a
    /// kind's only `Parts` role).
    pub(crate) fn selector_place(
        &mut self,
        selector: &SelectorDef,
        pair: &PairWork,
        geoms: &GeomTable,
        file: FileId,
        target: Option<PartId>,
    ) -> Solved<(PartId, LocalFace, CutPlace)> {
        let (Some(p), Some(q)) = (
            target.or_else(|| Self::member_part(pair, selector.part)),
            Self::member_part(pair, selector.other),
        ) else {
            return Err(Fault::Earlier.into());
        };
        let (Some(gp), Some(gq)) = (geoms.get(p), geoms.get(q)) else {
            return Err(Fault::Earlier.into());
        };
        let _ = file;
        selector_on(selector, p, gp, q, gq)
    }

    fn depth(&mut self, value: Value, env: &Env<'_>, span: Span) -> Solved<Depth> {
        match value {
            Value::Length(length) => Ok(Depth::Length(round_length(length)?)),
            Value::Word(Word::Through) => Ok(Depth::Through),
            other => Err(self
                .error(
                    Code::TypeMismatch,
                    format!(
                        "a depth needs a length or 'through', found {}",
                        other.description()
                    ),
                    env.file,
                    span,
                    "write a length or through",
                )
                .into()),
        }
    }

    /// Evaluates a cut operation's arguments.
    pub(crate) fn cut_kind(&mut self, operation: &OperationDef, env: &Env<'_>) -> Solved<CutKind> {
        let spec = stave_schema::operations::cut_operation(operation.operation);
        let mut values: Vec<(String, Value, Span)> = Vec::new();
        for arg in &operation.args {
            values.push((
                arg.name.clone(),
                self.eval(&arg.value, env)?,
                arg.value.span,
            ));
        }
        let get = |name: &str| {
            values
                .iter()
                .find(|(n, _, _)| n == name)
                .map(|(_, v, s)| (v.clone(), *s))
        };
        let length = |this: &mut Self, name: &str| -> Solved<stave_lang::units::Length> {
            match get(name) {
                Some((Value::Length(l), _)) => round_length(l),
                Some((other, span)) => Err(this
                    .error(
                        Code::UnitMismatch,
                        format!("'{name}' needs a length, found {}", other.description()),
                        env.file,
                        span,
                        "write a length",
                    )
                    .into()),
                None => match spec.argument(name).map(|a| a.default) {
                    Some(stave_schema::operations::ArgumentDefault::Length(nm)) => {
                        Ok(stave_lang::units::Length::from_nm(nm))
                    }
                    _ => Err(Fault::Earlier.into()),
                },
            }
        };
        let angle = |this: &mut Self, name: &str| -> Solved<stave_lang::units::Angle> {
            match get(name) {
                Some((Value::Angle(a), _)) => {
                    a.to_nearest_angle().ok_or(Failure::Fault(Fault::Overflow))
                }
                Some((other, span)) => Err(this
                    .error(
                        Code::UnitMismatch,
                        format!("'{name}' needs an angle, found {}", other.description()),
                        env.file,
                        span,
                        "write an angle",
                    )
                    .into()),
                None => Ok(stave_lang::units::Angle::ZERO),
            }
        };
        let depth = |this: &mut Self, name: &str| -> Solved<Depth> {
            match get(name) {
                Some((value, span)) => this.depth(value, env, span),
                None => Err(Fault::Earlier.into()),
            }
        };
        let flag = |name: &str| matches!(get(name), Some((Value::Bool(true), _)));
        Ok(match operation.operation {
            CutOperation::Notch => CutKind::Notch {
                depth: depth(self, "depth")?,
            },
            CutOperation::Slot => CutKind::Slot {
                width: length(self, "width")?,
                depth: depth(self, "depth")?,
                angle: angle(self, "angle")?,
                from_centre: flag("from_centre"),
            },
            CutOperation::Pocket => CutKind::Pocket {
                width: length(self, "width")?,
                thickness: length(self, "thickness")?,
                depth: depth(self, "depth")?,
            },
            CutOperation::Tenon => CutKind::Tenon {
                length: length(self, "length")?,
                width: length(self, "width")?,
                thickness: length(self, "thickness")?,
            },
            CutOperation::Spigot => CutKind::Spigot {
                length: length(self, "length")?,
                diameter: length(self, "diameter")?,
            },
            CutOperation::Round => CutKind::Round {
                diameter: length(self, "diameter")?,
                flats: match get("flats") {
                    Some(_) => Some(length(self, "flats")?),
                    None => None,
                },
                flats_angle: angle(self, "flats_angle")?,
            },
            CutOperation::Drill => CutKind::Drill {
                diameter: length(self, "diameter")?,
                depth: depth(self, "depth")?,
            },
            CutOperation::DovetailSocket => CutKind::DovetailSocket {
                neck: length(self, "neck")?,
                depth: length(self, "depth")?,
                slope: self.dovetail_slope(operation, &values, env)?,
            },
            CutOperation::DovetailTail => CutKind::DovetailTail {
                neck: length(self, "neck")?,
                depth: length(self, "depth")?,
                slope: self.dovetail_slope(operation, &values, env)?,
            },
            CutOperation::Outline => {
                let mut marks = Vec::new();
                for (along, across) in &operation.marks {
                    let along = round_length(self.eval_length(along, env)?)?;
                    let across = round_length(self.eval_length(across, env)?)?;
                    marks.push(Mark { along, across });
                }
                CutKind::Outline {
                    marks,
                    depth: depth(self, "depth")?,
                    from_depth: length(self, "from_depth")?,
                }
            }
        })
    }

    /// A dovetail's `angle:` or `slope:`: exactly one is given. A slope is
    /// a positive ratio, `1 in 8`.
    fn dovetail_slope(
        &mut self,
        operation: &OperationDef,
        values: &[(String, Value, Span)],
        env: &Env<'_>,
    ) -> Solved<Slope> {
        let get = |name: &str| {
            values
                .iter()
                .find(|(n, _, _)| n == name)
                .map(|(_, v, s)| (v.clone(), *s))
        };
        match (get("angle"), get("slope")) {
            (Some((Value::Angle(angle), _)), None) => Ok(Slope::Angle(
                angle.to_nearest_angle().ok_or(Fault::Overflow)?,
            )),
            (Some((other, span)), None) => Err(self
                .error(
                    Code::UnitMismatch,
                    format!("'angle' needs an angle, found {}", other.description()),
                    env.file,
                    span,
                    "write an angle",
                )
                .into()),
            (None, Some((value, span))) => match value.number() {
                Some(ratio) if ratio > Rational::ZERO => Ok(Slope::Ratio(ratio)),
                Some(_) => Err(self
                    .error(
                        Code::TypeMismatch,
                        "a dovetail's slope must be more than 0",
                        env.file,
                        span,
                        "write how far across for how far along: slope: 1 in 8",
                    )
                    .into()),
                None => Err(self
                    .error(
                        Code::TypeMismatch,
                        format!(
                            "'slope' needs a slope such as 1 in 8, found {}",
                            value.description()
                        ),
                        env.file,
                        span,
                        "write how far across for how far along: slope: 1 in 8",
                    )
                    .into()),
            },
            (Some(_), Some((_, span))) => Err(self
                .error(
                    Code::TypeMismatch,
                    "a dovetail takes 'angle' or 'slope', not both",
                    env.file,
                    span,
                    "remove 'angle' or 'slope'",
                )
                .into()),
            (None, None) => Err(self
                .error(
                    Code::MissingField,
                    format!(
                        "{} is missing 'angle' or 'slope'",
                        operation.operation.word()
                    ),
                    env.file,
                    operation.span,
                    "add angle: 10deg, or slope: 1 in 8",
                )
                .into()),
        }
    }

    /// Every size of a cut is more than zero (grammar, "Cut operations"):
    /// a 0mm tenon cuts the whole extension away and joins nothing, a 0mm
    /// long one is a butt joint by another name. `error[cut-size]` at the
    /// joint, naming the kind's setting the size comes from when it is
    /// one.
    pub(crate) fn check_cut_size(
        &mut self,
        at: CutAt<'_>,
        name: Option<&stave_model::defs::CutName>,
        operation: &OperationDef,
        kind: &CutKind,
        part: PartId,
    ) -> Solved<()> {
        let length = |depth: &Depth| match depth {
            Depth::Length(length) => Some(*length),
            Depth::Through => None,
        };
        let sizes: Vec<(&str, &str, Option<Length>)> = match kind {
            CutKind::Notch { depth } => vec![("depth", "deep", length(depth))],
            CutKind::Slot { width, depth, .. } => vec![
                ("width", "wide", Some(*width)),
                ("depth", "deep", length(depth)),
            ],
            CutKind::Pocket {
                width,
                thickness,
                depth,
            } => vec![
                ("width", "wide", Some(*width)),
                ("thickness", "thick", Some(*thickness)),
                ("depth", "deep", length(depth)),
            ],
            CutKind::Tenon {
                length,
                width,
                thickness,
            } => vec![
                ("length", "long", Some(*length)),
                ("width", "wide", Some(*width)),
                ("thickness", "thick", Some(*thickness)),
            ],
            CutKind::Spigot { length, diameter } => vec![
                ("length", "long", Some(*length)),
                ("diameter", "across", Some(*diameter)),
            ],
            CutKind::Round {
                diameter, flats, ..
            } => vec![
                ("diameter", "across", Some(*diameter)),
                ("flats", "across its flats", *flats),
            ],
            CutKind::Drill { diameter, depth } => vec![
                ("diameter", "across", Some(*diameter)),
                ("depth", "deep", length(depth)),
            ],
            CutKind::DovetailSocket { neck, depth, .. }
            | CutKind::DovetailTail { neck, depth, .. } => vec![
                ("neck", "wide at its neck", Some(*neck)),
                ("depth", "deep", Some(*depth)),
            ],
            CutKind::Outline { depth, .. } => vec![("depth", "deep", length(depth))],
        };
        let Some((argument, word, size)) = sizes.into_iter().find_map(|(argument, word, size)| {
            size.filter(|size| *size <= Length::ZERO)
                .map(|size| (argument, word, size))
        }) else {
            return Ok(());
        };
        let cut = name.map_or_else(|| operation.operation.word().to_owned(), |n| n.words());
        let part_name = self.part_name(part);
        let size = stave_model::value::format_length(size.into());
        // The setting the size comes from, when the operation reads one of
        // the kind's params straight.
        let setting = operation
            .args
            .iter()
            .find(|arg| arg.name == argument)
            .and_then(|arg| match &arg.value.kind {
                RExprKind::Symbol(Symbol::KindParam(kind, index)) => self
                    .model
                    .defs
                    .kind(*kind)
                    .and_then(|k| k.params.get(*index))
                    .map(|param| param.decl.name.clone()),
                _ => None,
            });
        let help = match setting {
            Some(setting) => format!("give {} a {setting}: of more than 0mm", at.owner),
            None => format!(
                "check the settings of {}: its {cut}'s {argument} must come out more than 0mm",
                at.owner
            ),
        };
        Err(self
            .error(
                Code::CutSize,
                format!("{}: its {cut} on {part_name} is {size} {word}", at.owner),
                at.file,
                at.span,
                help,
            )
            .into())
    }

    /// The cuts of one `cut selector { }` entry for one pair.
    pub(crate) fn pair_cuts(
        &mut self,
        cut: &CutDef,
        pair: &PairWork,
        geoms: &GeomTable,
        env: &Env<'_>,
        at: CutAt<'_>,
    ) -> Solved<Vec<(PartId, LocalFace, CutPlace, CutKind)>> {
        // A cut on a kind's only `Parts` role is made on each of its parts
        // that meets the other part of the selector.
        let targets: Vec<Option<PartId>> = if pair.across
            && cut.selector.part == JointMember::Role(0)
        {
            let other = Self::member_part(pair, cut.selector.other);
            pair.parts
                .iter()
                .filter(|part| {
                    let (Some(a), Some(b)) = (geoms.get(**part), other.and_then(|o| geoms.get(o)))
                    else {
                        return false;
                    };
                    Some(**part) != other && a.cut_box.meets(&b.cut_box)
                })
                .map(|part| Some(*part))
                .collect()
        } else {
            vec![None]
        };
        let mut out = Vec::new();
        for target in targets {
            let (part, face, place) =
                self.selector_place(&cut.selector, pair, geoms, env.file, target)?;
            for operation in &cut.operations {
                let kind = self.cut_kind(operation, env)?;
                self.check_cut_size(at, cut.name.as_ref(), operation, &kind, part)?;
                self.check_tongue(operation, &place, face, env)?;
                out.push((part, face, place.clone(), kind));
            }
        }
        Ok(out)
    }

    /// A tenon on `crossing` is a tongue along an edge, for parts that
    /// butt edge to edge: the face must be across the part's width
    /// (grammar, "Cut operations").
    fn check_tongue(
        &mut self,
        operation: &OperationDef,
        place: &CutPlace,
        face: LocalFace,
        env: &Env<'_>,
    ) -> Solved<()> {
        let CutPlace::Crossing { other, .. } = place else {
            return Ok(());
        };
        if operation.operation != CutOperation::Tenon || face.axis == PartAxis::Width {
            return Ok(());
        }
        Err(self.error(
            Code::TypeMismatch,
            format!(
                "a tenon on crossing is a tongue along an edge, but {} meets this part on its {}",
                self.part_name(*other),
                match face.axis {
                    PartAxis::Length => "end",
                    PartAxis::Width | PartAxis::Thickness => "face",
                }
            ),
            env.file,
            operation.span,
            "use crossing only for parts that butt edge to edge, or cut the tenon on end(...)",
        ).into())
    }

    /// The assembly steps of a pair.
    pub(crate) fn pair_steps(
        &mut self,
        kind: KindId,
        pair: &PairWork,
        env: &Env<'_>,
    ) -> Solved<Vec<AssemblyStep>> {
        let Some(joint) = self.model.defs.kind(kind).and_then(|k| k.joint.clone()) else {
            return Ok(Vec::new());
        };
        let mut steps = Vec::new();
        for step in &joint.insert {
            let Some(part) = Self::member_part(pair, step.part) else {
                continue;
            };
            let by = match &step.by {
                Some(expr) => Some(match self.eval(expr, env)? {
                    Value::Length(l) => StepAmount::Length(round_length(l)?),
                    Value::Angle(a) => {
                        StepAmount::Angle(a.to_nearest_angle().ok_or(Fault::Overflow)?)
                    }
                    _ => return Err(Fault::Earlier.into()),
                }),
                None => None,
            };
            let angle = match &step.angle {
                Some(expr) => Some(
                    self.eval_angle(expr, env)?
                        .to_nearest_angle()
                        .ok_or(Fault::Overflow)?,
                ),
                None => None,
            };
            let kind = match step.kind {
                AssemblyStepKind::Push => StepKind::Push,
                AssemblyStepKind::Rotate => StepKind::Rotate,
                AssemblyStepKind::Drive => StepKind::Drive,
            };
            steps.push(AssemblyStep {
                kind,
                part,
                axis: step.axis,
                by,
                angle,
            });
        }
        Ok(steps)
    }

    /// Places a part a joint makes, for one pair.
    pub(crate) fn place_made_part(
        &mut self,
        made: &MadePartDef,
        pair: &PairWork,
        geoms: &GeomTable,
        env: &Env<'_>,
        name: &str,
        at: (FileId, Span),
    ) -> Solved<MadeGeom> {
        let body = &made.body;
        let stock = match self.eval(&body.stock, env)? {
            Value::Stock(stock) => stock,
            other => {
                return Err(self
                    .error(
                        Code::TypeMismatch,
                        format!("this needs a stock, found {}", other.description()),
                        env.file,
                        body.stock.span,
                        "name a stock",
                    )
                    .into());
            }
        };
        let size_of = |this: &mut Self,
                       size: &Option<Size>,
                       field: StockField,
                       word: &str|
         -> Solved<ExactLength> {
            match size {
                Some(Size::Expr(expr)) => this.eval_length(expr, env),
                Some(Size::Fit(span)) => Err(this
                    .error(
                        Code::FitUndecided,
                        format!("{name}: nothing decides its {word}"),
                        env.file,
                        *span,
                        "a made part's sizes must be written",
                    )
                    .into()),
                None => this.stock_length(stock, field)?.ok_or_else(|| {
                    this.error(
                        Code::MissingField,
                        format!("made part '{name}' is missing '{word}'"),
                        env.file,
                        body.span,
                        format!("add {word}: ..."),
                    )
                    .into()
                }),
            }
        };
        let length = size_of(
            self,
            &Some(body.length.clone()),
            StockField::Thickness,
            "length",
        )?;
        let width = size_of(self, &body.width, StockField::Width, "width")?;
        let thickness = match &body.thickness {
            Some(expr) => self.eval_length(expr, env)?,
            None => self
                .stock_length(stock, StockField::Thickness)?
                .ok_or(Fault::Earlier)?,
        };
        let drawn = [length, width, thickness];
        self.check_made_part_stock(stock, name, drawn, body.grain, at)?;
        // Orientation.
        let mut orientation = None;
        let mut of_part = None;
        for term in &made.place {
            if let MadeTermKind::Orient { length, width, of } = &term.kind {
                let Some(part) = pair.parts.get(*of).copied() else {
                    return Err(Fault::Internal("a part").into());
                };
                let Some(geom) = geoms.get(part) else {
                    return Err(Fault::Earlier.into());
                };
                let (l, w) = (
                    geom.orientation.direction(*length),
                    geom.orientation.direction(*width),
                );
                orientation = Some(Orientation::from_length_and_width(l, w).map_err(|_| {
                    self.error(
                        Code::TypeMismatch,
                        "'orient' needs a length and width along different axes".to_owned(),
                        env.file,
                        term.span,
                        "use two different axes of the part",
                    )
                })?);
                of_part = Some(part);
            }
        }
        let Some(orientation) = orientation else {
            return Err(Fault::Earlier.into());
        };
        let size_along = |axis: Axis| {
            drawn
                .get(local_slot(orientation.part_axis(axis)))
                .copied()
                .unwrap_or(ExactLength::ZERO)
        };
        let mut lows: [Option<ExactLength>; 3] = [None, None, None];
        let axis_index = |axis: Axis| match axis {
            Axis::Across => 0,
            Axis::Up => 1,
            Axis::Deep => 2,
        };
        let mut set = |this: &mut Self, axis: Axis, low: ExactLength, span: Span| -> Solved<()> {
            let slot = lows
                .get_mut(axis_index(axis))
                .ok_or(Fault::Internal("an axis of a made part"))?;
            if slot.is_some() {
                return Err(this
                    .error(
                        Code::AxisSetTwice,
                        format!("{name}: position along '{}' is set twice", axis.word()),
                        env.file,
                        span,
                        format!("keep one term for '{}'", axis.word()),
                    )
                    .into());
            }
            *slot = Some(low);
            Ok(())
        };
        for term in &made.place {
            let selector = match &term.kind {
                MadeTermKind::Orient { .. } => continue,
                MadeTermKind::Center { selector, .. }
                | MadeTermKind::Against { selector, .. }
                | MadeTermKind::Flush { selector, .. } => selector,
            };
            let (p, gp) = Self::member_geom(pair, selector.part, geoms)?;
            let (q, gq) = Self::member_geom(pair, selector.other, geoms)?;
            let (_, face, _) = selector_on(selector, p, &gp, q, &gq)?;
            let (face_axis, _, positive) = gp.face_plane(face);
            let drawn_face = if positive {
                gp.drawn_box.max(face_axis)
            } else {
                gp.drawn_box.min(face_axis)
            };
            // The plane: for `end` and `side`, that face of the first part;
            // for `crossing`, the contact plane of the drawn boxes for parts
            // that butt, the middle of their overlap for parts that cross.
            let plane = match selector.selector {
                Selector::End | Selector::Side => drawn_face,
                Selector::Crossing => match gp.drawn_box.touching_axis(&gq.drawn_box) {
                    Some(axis) if axis == face_axis => drawn_face,
                    _ => {
                        let low = gp.drawn_box.min(face_axis).max(gq.drawn_box.min(face_axis));
                        let high = gp.drawn_box.max(face_axis).min(gq.drawn_box.max(face_axis));
                        midpoint(low, high).ok_or(Fault::Overflow)?
                    }
                },
            };
            let centre_of = |axis: Axis| {
                midpoint(
                    gq.cut_box.min(axis).max(gp.cut_box.min(axis)),
                    gq.cut_box.max(axis).min(gp.cut_box.max(axis)),
                )
                .ok_or(Fault::Overflow)
            };
            let half = |axis: Axis| {
                size_along(axis)
                    .checked_div(Rational::from(2i64))
                    .ok_or(Fault::Overflow)
            };
            match &term.kind {
                MadeTermKind::Center { axis: None, .. } => {
                    for axis in Axis::ALL {
                        let centre = if axis == face_axis {
                            plane
                        } else {
                            centre_of(axis)?
                        };
                        set(
                            self,
                            axis,
                            centre.checked_sub(half(axis)?).ok_or(Fault::Overflow)?,
                            term.span,
                        )?;
                    }
                }
                MadeTermKind::Center {
                    axis: Some(part_axis),
                    ..
                } => {
                    let Some(of) = of_part.and_then(|p| geoms.get(p)) else {
                        return Err(Fault::Earlier.into());
                    };
                    let axis = of.orientation.direction(*part_axis).axis();
                    let centre = if axis == face_axis {
                        plane
                    } else {
                        centre_of(axis)?
                    };
                    set(
                        self,
                        axis,
                        centre.checked_sub(half(axis)?).ok_or(Fault::Overflow)?,
                        term.span,
                    )?;
                }
                MadeTermKind::Against { gap, .. } => {
                    let gap = match gap {
                        Some(g) => self.eval_length(g, env)?,
                        None => ExactLength::ZERO,
                    };
                    let low = if positive {
                        plane.checked_add(gap)
                    } else {
                        plane
                            .checked_sub(gap)
                            .and_then(|x| x.checked_sub(size_along(face_axis)))
                    };
                    set(self, face_axis, low.ok_or(Fault::Overflow)?, term.span)?;
                }
                MadeTermKind::Flush { inset, .. } => {
                    let inset = match inset {
                        Some(i) => self.eval_length(i, env)?,
                        None => ExactLength::ZERO,
                    };
                    let low = if positive {
                        plane
                            .checked_sub(inset)
                            .and_then(|x| x.checked_sub(size_along(face_axis)))
                    } else {
                        plane.checked_add(inset)
                    };
                    set(self, face_axis, low.ok_or(Fault::Overflow)?, term.span)?;
                }
                MadeTermKind::Orient { .. } => {}
            }
        }
        let mut extents = Vec::new();
        for axis in [Axis::Across, Axis::Up, Axis::Deep] {
            let Some(Some(low)) = lows.get(axis_index(axis)).copied() else {
                return Err(self.axis_not_set(name, axis, env.file, made.span).into());
            };
            extents.push((
                low,
                low.checked_add(size_along(axis)).ok_or(Fault::Overflow)?,
            ));
        }
        let [across, up, deep] = extents.as_slice() else {
            return Err(Fault::Internal("a made part's box").into());
        };
        let drawn_box = ExactBox::new(*across, *up, *deep);
        let preferred = |axis: Axis| match axis {
            Axis::Up => stave_lang::placement::Direction::Up,
            Axis::Across => stave_lang::placement::Direction::Right,
            Axis::Deep => stave_lang::placement::Direction::Front,
        };
        let reference = |axis: PartAxis| {
            let d = orientation.direction(axis);
            LocalFace {
                axis,
                end: if preferred(d.axis()) == d {
                    End::End
                } else {
                    End::Start
                },
            }
        };
        Ok(MadeGeom {
            stock,
            geom: PartGeom {
                orientation,
                drawn,
                start: [ExactLength::ZERO; 3],
                end: [ExactLength::ZERO; 3],
                drawn_box,
                cut_box: drawn_box,
                reference_face: reference(PartAxis::Thickness),
                reference_edge: reference(PartAxis::Width),
            },
        })
    }
}

/// A part a joint made, placed.
pub(crate) struct MadeGeom {
    pub stock: StockId,
    pub geom: PartGeom,
}

/// Geometry of every part (declared and made), by id.
#[derive(Default)]
pub(crate) struct GeomTable {
    pub parts: Vec<Option<PartGeom>>,
    /// The boards of a part made of boards, while they are worked out.
    pub boards: std::collections::BTreeMap<PartId, PartGeom>,
}

impl GeomTable {
    pub fn get(&self, part: PartId) -> Option<&PartGeom> {
        if crate::boards::is_board(part) {
            return self.boards.get(&part);
        }
        self.parts.get(part.index()).and_then(Option::as_ref)
    }

    pub fn set(&mut self, part: PartId, geom: PartGeom) {
        if crate::boards::is_board(part) {
            self.boards.insert(part, geom);
            return;
        }
        if self.parts.len() <= part.index() {
            self.parts.resize(part.index() + 1, None);
        }
        if let Some(slot) = self.parts.get_mut(part.index()) {
            *slot = Some(geom);
        }
    }
}

/// Rounds an exact length to whole nanometres for storing.
pub(crate) fn round_length(length: ExactLength) -> Solved<stave_lang::units::Length> {
    length
        .to_nearest_length()
        .ok_or(Failure::Fault(Fault::Overflow))
}

fn local_box(local: &[(ExactLength, ExactLength); 3]) -> Solved<LocalBox> {
    let point = |pick: fn(&(ExactLength, ExactLength)) -> ExactLength| -> Solved<LocalPoint> {
        let [l, w, t] = local;
        Ok(LocalPoint {
            length: round_length(pick(l))?,
            width: round_length(pick(w))?,
            thickness: round_length(pick(t))?,
        })
    };
    Ok(LocalBox {
        min: point(|p| p.0)?,
        max: point(|p| p.1)?,
    })
}

/// The face a selector names and where on it, given the geometry of its
/// two parts (`gp` may stand for all the parts of a kind's only `Parts`
/// role together). A selector that names a part against itself (`end(p)`,
/// `side(p, normal: ..)`) measures from the start of `p`'s length, and
/// `end(p)` is the end at the end of it.
fn selector_on(
    selector: &SelectorDef,
    p: PartId,
    gp: &PartGeom,
    q: PartId,
    gq: &PartGeom,
) -> Solved<(PartId, LocalFace, CutPlace)> {
    let alone = selector.part == selector.other;
    let near_end = if alone {
        LocalFace {
            axis: PartAxis::Length,
            end: End::End,
        }
    } else {
        let length_world = gp.orientation.direction(PartAxis::Length).axis();
        let q_centre = midpoint(
            gq.drawn_box.min(length_world),
            gq.drawn_box.max(length_world),
        )
        .ok_or(Fault::Overflow)?;
        let p_centre = midpoint(
            gp.drawn_box.min(length_world),
            gp.drawn_box.max(length_world),
        )
        .ok_or(Fault::Overflow)?;
        gp.face_toward(length_world, q_centre >= p_centre)
    };
    let along_from = if alone { End::Start } else { near_end.end };
    match selector.selector {
        Selector::End => {
            let face = if selector.opposite {
                LocalFace {
                    axis: PartAxis::Length,
                    end: flip(near_end.end),
                }
            } else {
                near_end
            };
            Ok((p, face, CutPlace::End { toward: q }))
        }
        Selector::Side => {
            let normal = selector.normal.unwrap_or(PartAxis::Thickness);
            let (face_ref, across_ref) = match normal {
                PartAxis::Width => (gp.reference_edge, gp.reference_face),
                PartAxis::Thickness | PartAxis::Length => (gp.reference_face, gp.reference_edge),
            };
            let face = if selector.opposite {
                LocalFace {
                    axis: face_ref.axis,
                    end: flip(face_ref.end),
                }
            } else {
                face_ref
            };
            let across_from = if selector.flip {
                flip(across_ref.end)
            } else {
                across_ref.end
            };
            Ok((
                p,
                face,
                CutPlace::Side {
                    from: q,
                    along_from,
                    across_axis: across_ref.axis,
                    across_from,
                },
            ))
        }
        Selector::Crossing => {
            let face = match selector.normal {
                Some(PartAxis::Thickness) => gp.reference_face,
                Some(PartAxis::Width) => gp.reference_edge,
                Some(PartAxis::Length) => near_end,
                None => match gp.drawn_box.touching_axis(&gq.drawn_box) {
                    Some(axis) => {
                        let toward_positive = gp.drawn_box.max(axis) == gq.drawn_box.min(axis);
                        gp.face_toward(axis, toward_positive)
                    }
                    None => {
                        let q_length = gq.orientation.direction(PartAxis::Length).axis();
                        let p_thickness = gp.orientation.direction(PartAxis::Thickness).axis();
                        if q_length != p_thickness {
                            gp.reference_face
                        } else {
                            gp.reference_edge
                        }
                    }
                },
            };
            let face = if selector.opposite {
                LocalFace {
                    axis: face.axis,
                    end: flip(face.end),
                }
            } else {
                face
            };
            let local = gp.local_box(&gq.cut_box.intersection(&gp.cut_box))?;
            let footprint = local_box(&local)?;
            Ok((
                p,
                face,
                CutPlace::Crossing {
                    other: q,
                    footprint,
                },
            ))
        }
    }
}

/// The smallest box holding both boxes.
fn hull(a: &ExactBox, b: &ExactBox) -> ExactBox {
    let mut out = *a;
    for axis in Axis::ALL {
        out = out.with(
            axis,
            a.min(axis).min(b.min(axis)),
            a.max(axis).max(b.max(axis)),
        );
    }
    out
}
