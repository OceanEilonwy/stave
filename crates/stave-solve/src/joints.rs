//! Joints: binding kind calls, finding pairs, what joints add to their
//! parts, the parts they make and the cuts they describe.

use stave_lang::ast::{BaseKind, Span};
use stave_lang::placement::{Axis, Face, PartAxis};
use stave_lang::units::{ExactLength, Rational};
use stave_lang::{Code, Diagnostic, FileId, Label};
use stave_model::defs::{
    CutDef, DefId, JointMember, KindCallDef, MadePartDef, MadeTermKind, OperationDef, SelectorDef,
    Size, StockField, Word,
};
use stave_model::ids::{JointId, KindId, PartId, RecipeId, StockId};
use stave_model::solved::{
    AssemblyStep, Contact, CutKind, CutPlace, Depth, End, LocalBox, LocalFace, LocalPoint, Mark,
    Orientation, StepAmount, StepKind,
};
use stave_model::value::{ExactBox, KindValue, Value};
use stave_schema::CutOperation;
use stave_schema::kinds::RoleType;
use stave_schema::operations::Selector;
use stave_schema::placement::AssemblyStepKind;

use crate::eval::{Env, KindEnv, midpoint};
use crate::{Frame, Memo, Reported, Solved, Solver};

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
                .ok_or(Reported),
            _ => Err(Reported),
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
        value.ok_or(Reported)
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
}

/// One pair of a joint.
pub(crate) struct PairWork {
    pub parts: Vec<PartId>,
    pub contact: Contact,
    pub env: KindEnv,
    pub extensions: Vec<stave_model::solved::Extension>,
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
    ) -> Solved<Binding> {
        let kind_value = match self.eval(&call.kind, env)? {
            Value::Kind(kind) => kind,
            other => {
                return Err(self.error(
                    Code::TypeMismatch,
                    format!("{what} needs a kind, found {}", other.description()),
                    env.file,
                    call.kind.span,
                    "name a joint or hardware kind",
                ));
            }
        };
        let KindValue {
            kind,
            name: kind_name,
            settings: given,
        } = kind_value;
        if self.model.defs.is_poisoned(DefId::Kind(kind)) {
            return Err(self.skip());
        }
        let Some(kind_def) = self.model.defs.kind(kind).cloned() else {
            return Err(Reported);
        };
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
                    return Err(self.error(
                        Code::TypeMismatch,
                        format!("the kind '{}' has no roles", kind_name),
                        env.file,
                        positional.span,
                        "a kind value takes its settings by name",
                    ));
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
                return Err(self.error(
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
                ));
            }
            if settings.iter().any(|(n, _)| *n == setting.name) {
                return Err(self.error(
                    Code::DuplicateName,
                    format!("'{}' is declared twice", setting.name),
                    env.file,
                    setting.name_span,
                    "the kind value already gives this setting",
                ));
            }
            settings.push((setting.name.clone(), value));
        }
        // A setting must be within its param's `within`.
        if let Some(ctx) = self.module_ctx(DefId::Kind(kind)) {
            for (name, value) in &settings {
                if let Some(param) = kind_def.params.iter().find(|p| p.decl.name == *name) {
                    let span = call
                        .named
                        .iter()
                        .find(|s| s.name == *name)
                        .map_or(call.span, |s| s.value.span);
                    self.check_domain(param, value, env.file, span, name, ctx)?;
                }
            }
        }
        let mut bound = Vec::new();
        for (index, role) in roles.into_iter().enumerate() {
            let Some(role_def) = kind_def.roles.get(index) else {
                continue;
            };
            let Some(value) = role else {
                return Err(self.error(
                    Code::MissingField,
                    format!("{what} is missing '{}'", role_def.name),
                    env.file,
                    call.span,
                    format!("name the part for '{}' in the call", role_def.name),
                ));
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
                return Err(self.error(
                    Code::TypeMismatch,
                    format!(
                        "'{}' needs {wanted}, found {}",
                        role_def.name,
                        value.description()
                    ),
                    env.file,
                    call.span,
                    "check the role's argument",
                ));
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
            Some(Memo::Failed) => return Err(Reported),
            Some(Memo::Busy) => return Err(self.cycle(Frame::Joint(joint))),
            _ => {}
        }
        let Some(inst) = self.model.joints.get(joint.index()).cloned() else {
            return Err(Reported);
        };
        let Some(def) = self.model.defs.joint(inst.def).cloned() else {
            return Err(Reported);
        };
        if self.poisoned(DefId::Joint(inst.def)) {
            return Err(self.skip());
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
        let result = self.bind_call(&def.call, &env, &what).and_then(|binding| {
            let base = self.model.defs.kind(binding.kind).map(|k| k.base);
            if base == Some(BaseKind::Joint) {
                Ok(binding)
            } else {
                Err(self.error(
                    Code::TypeMismatch,
                    format!("'{}' needs a joint kind", inst.name),
                    def.decl.file,
                    def.call.kind.span,
                    "use a kind declared with kind name : joint",
                ))
            }
        });
        self.leave();
        if let Some(slot) = self.joints.get_mut(joint.index()) {
            *slot = match &result {
                Ok(binding) => Memo::Done(binding.clone()),
                Err(_) => Memo::Failed,
            };
        }
        result
    }

    fn joint_place(&self, joint: JointId) -> (FileId, Span) {
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

    /// Pass A of a joint: pairs, requires and what it adds to its parts.
    pub(crate) fn work_joint(&mut self, joint: JointId) -> Solved<JointWork> {
        let binding = self.joint_binding(joint)?;
        let Some(kind) = self.model.defs.kind(binding.kind).cloned() else {
            return Err(Reported);
        };
        let (file, span) = self.joint_place(joint);
        let name = self
            .model
            .joints
            .get(joint.index())
            .map(|j| j.name.clone())
            .unwrap_or_default();
        let pairwise = kind.roles.iter().any(|r| r.ty == RoleType::Parts);
        let mut plans: Vec<(Vec<PartId>, Contact)> = Vec::new();
        let mut unmatched = Vec::new();
        let role_parts: Vec<Vec<PartId>> = binding
            .roles
            .iter()
            .map(|v| v.parts().unwrap_or_default())
            .collect();
        if pairwise {
            let (Some(first), Some(second)) = (role_parts.first(), role_parts.get(1)) else {
                return Err(Reported);
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
            let roles: Vec<Value> = parts.iter().map(|p| Value::Part(*p)).collect();
            let env = KindEnv::new(
                binding.kind,
                roles,
                binding.settings.clone(),
                kind.params.len(),
            );
            let Some(ctx) = self.module_ctx(DefId::Kind(binding.kind)) else {
                return Err(Reported);
            };
            let kind_env = Env {
                ctx,
                file: kind.decl.file,
                kind: Some(&env),
                array: None,
            };
            // Requires, at the current values, for this pair.
            let mut results = Vec::new();
            for require in &kind.requires {
                let holds = self.eval_bool(&require.condition, &kind_env)?;
                let message = self.format_text(&require.message, &kind_env)?;
                if !holds {
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
                        .with_note(format!("for {} in {name}", names.join(" and ")))
                        .with_help(
                            "change the joint's settings or the parts' sizes so the rule holds",
                        ),
                    );
                }
                results.push(stave_model::solved::RequireResult {
                    name: format!("{name}.{}", require.decl.name),
                    holds,
                    message,
                });
            }
            requires.push(results);
            // What the joint adds.
            let mut extensions = Vec::new();
            if let Some(joint_def) = &kind.joint {
                for extend in &joint_def.adds {
                    let by = self.eval_length(&extend.by, &kind_env)?;
                    let Some(part) = parts.get(extend.role).copied() else {
                        continue;
                    };
                    let others: Vec<PartId> =
                        parts.iter().copied().filter(|p| *p != part).collect();
                    for other in others {
                        let (a, b) = (self.drawn_box(part)?, self.drawn_box(other)?);
                        if !a.meets(&b) {
                            continue;
                        }
                        let Some(axis) = a.touching_axis(&b) else {
                            let (pn, qn) = (self.part_name(part), self.part_name(other));
                            return Err(self.error(Code::TypeMismatch, format!("'extend' needs parts that butt, but {pn} and {qn} cross"), file, span, "a joint whose parts cross can't lengthen them; draw them meeting face to face"));
                        };
                        let toward_positive = a.max(axis) == b.min(axis);
                        let orientation = self.part_orientation(part)?;
                        let local = orientation.part_axis(axis);
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
                contact,
                env,
                extensions,
            });
        }
        Ok(JointWork {
            binding,
            pairs,
            unmatched,
            requires,
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
                *target = target.checked_add(extend.by).ok_or(Reported)?;
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
            let low = cut_box.min(world).checked_sub(low_add).ok_or(Reported)?;
            let high = cut_box.max(world).checked_add(high_add).ok_or(Reported)?;
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

    /// A part's cut length must not be more than its stock's `max_length`
    /// (grammar, "stock"). Mirror images are left to their source, so one
    /// mistake is one error.
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
        let stock = self.part_stock(part)?;
        let Some(max_length) =
            self.stock_length(stock, stave_model::defs::StockField::MaxLength)?
        else {
            return Ok(());
        };
        let slot = local_slot(PartAxis::Length);
        let cut_length = [&geom.drawn, &geom.start, &geom.end]
            .iter()
            .filter_map(|sizes| sizes.get(slot).copied())
            .try_fold(ExactLength::ZERO, |sum, size| sum.checked_add(size))
            .ok_or(Reported)?;
        if cut_length <= max_length {
            return Ok(());
        }
        let name = self.part_name(part);
        let stock_name = self
            .model
            .defs
            .stock(stock)
            .map(|s| s.decl.name.clone())
            .unwrap_or_default();
        let (file, span) = match (self.part_body(part).cloned(), self.part_env(part)) {
            (Some(body), Some(env)) => match &body.length {
                stave_model::defs::Size::Expr(expr) => (env.file, expr.span),
                stave_model::defs::Size::Fit(span) => (env.file, *span),
            },
            _ => (self.project_file, self.project_span),
        };
        Err(self.error(
            Code::TypeMismatch,
            format!(
                "{name}: its cut length works out as {}, which is more than its stock's max_length of {}",
                stave_model::value::format_length(cut_length),
                stave_model::value::format_length(max_length)
            ),
            file,
            span,
            format!(
                "stock '{stock_name}' comes no longer than {}; make the part shorter or choose longer stock",
                stave_model::value::format_length(max_length)
            ),
        ))
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
                        return Err(this.error(
                            Code::TypeMismatch,
                            format!(
                                "'{field}' needs a face across the part's {}, found '{}'",
                                axis.word(),
                                face.word()
                            ),
                            file,
                            span,
                            format!("name one of the two faces across its {}", axis.word()),
                        ));
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

    /// The face a selector names and where on it.
    pub(crate) fn selector_place(
        &mut self,
        selector: &SelectorDef,
        pair: &PairWork,
        geoms: &GeomTable,
        file: FileId,
    ) -> Solved<(PartId, LocalFace, CutPlace)> {
        let (Some(p), Some(q)) = (
            Self::member_part(pair, selector.part),
            Self::member_part(pair, selector.other),
        ) else {
            return Err(Reported);
        };
        let (Some(gp), Some(gq)) = (geoms.get(p), geoms.get(q)) else {
            return Err(Reported);
        };
        let (gp, gq) = (gp.clone(), gq.clone());
        let length_world = gp.orientation.direction(PartAxis::Length).axis();
        let q_centre = midpoint(
            gq.drawn_box.min(length_world),
            gq.drawn_box.max(length_world),
        )
        .ok_or(Reported)?;
        let p_centre = midpoint(
            gp.drawn_box.min(length_world),
            gp.drawn_box.max(length_world),
        )
        .ok_or(Reported)?;
        let near_end = gp.face_toward(length_world, q_centre >= p_centre);
        let _ = file;
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
                    PartAxis::Thickness | PartAxis::Length => {
                        (gp.reference_face, gp.reference_edge)
                    }
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
                        along_from: near_end.end,
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

    fn depth(&mut self, value: Value, env: &Env<'_>, span: Span) -> Solved<Depth> {
        match value {
            Value::Length(length) => Ok(Depth::Length(round_length(length)?)),
            Value::Word(Word::Through) => Ok(Depth::Through),
            other => Err(self.error(
                Code::TypeMismatch,
                format!(
                    "a depth needs a length or 'through', found {}",
                    other.description()
                ),
                env.file,
                span,
                "write a length or through",
            )),
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
                Some((other, span)) => Err(this.error(
                    Code::UnitMismatch,
                    format!("'{name}' needs a length, found {}", other.description()),
                    env.file,
                    span,
                    "write a length",
                )),
                None => match spec.argument(name).map(|a| a.default) {
                    Some(stave_schema::operations::ArgumentDefault::Length(nm)) => {
                        Ok(stave_lang::units::Length::from_nm(nm))
                    }
                    _ => Err(Reported),
                },
            }
        };
        let angle = |this: &mut Self, name: &str| -> Solved<stave_lang::units::Angle> {
            match get(name) {
                Some((Value::Angle(a), _)) => a.to_nearest_angle().ok_or(Reported),
                Some((other, span)) => Err(this.error(
                    Code::UnitMismatch,
                    format!("'{name}' needs an angle, found {}", other.description()),
                    env.file,
                    span,
                    "write an angle",
                )),
                None => Ok(stave_lang::units::Angle::ZERO),
            }
        };
        let depth = |this: &mut Self, name: &str| -> Solved<Depth> {
            match get(name) {
                Some((value, span)) => this.depth(value, env, span),
                None => Err(Reported),
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
                count: match get("count") {
                    Some((value, span)) => match value.number() {
                        Some(n) if n.is_integer() && !n.is_negative() => {
                            u32::try_from(n.numerator()).map_err(|_| Reported)?
                        }
                        _ => {
                            return Err(self.error(
                                Code::TypeMismatch,
                                "'count' needs a whole number",
                                env.file,
                                span,
                                "write a count such as 4",
                            ));
                        }
                    },
                    None => 1,
                },
            },
            CutOperation::DovetailSocket => CutKind::DovetailSocket {
                neck: length(self, "neck")?,
                depth: length(self, "depth")?,
                angle: angle(self, "angle")?,
            },
            CutOperation::DovetailTail => CutKind::DovetailTail {
                neck: length(self, "neck")?,
                depth: length(self, "depth")?,
                angle: angle(self, "angle")?,
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

    /// The cuts of one `cut selector { }` entry for one pair.
    pub(crate) fn pair_cuts(
        &mut self,
        cut: &CutDef,
        pair: &PairWork,
        geoms: &GeomTable,
        env: &Env<'_>,
    ) -> Solved<Vec<(PartId, LocalFace, CutPlace, CutKind)>> {
        let (part, face, place) = self.selector_place(&cut.selector, pair, geoms, env.file)?;
        let mut out = Vec::new();
        for operation in &cut.operations {
            let kind = self.cut_kind(operation, env)?;
            out.push((part, face, place.clone(), kind));
        }
        Ok(out)
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
                    Value::Angle(a) => StepAmount::Angle(a.to_nearest_angle().ok_or(Reported)?),
                    _ => return Err(Reported),
                }),
                None => None,
            };
            let angle = match &step.angle {
                Some(expr) => Some(
                    self.eval_angle(expr, env)?
                        .to_nearest_angle()
                        .ok_or(Reported)?,
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
    ) -> Solved<MadeGeom> {
        let body = &made.body;
        let stock = match self.eval(&body.stock, env)? {
            Value::Stock(stock) => stock,
            other => {
                return Err(self.error(
                    Code::TypeMismatch,
                    format!("this needs a stock, found {}", other.description()),
                    env.file,
                    body.stock.span,
                    "name a stock",
                ));
            }
        };
        let size_of = |this: &mut Self,
                       size: &Option<Size>,
                       field: StockField,
                       word: &str|
         -> Solved<ExactLength> {
            match size {
                Some(Size::Expr(expr)) => this.eval_length(expr, env),
                Some(Size::Fit(span)) => Err(this.error(
                    Code::FitUndecided,
                    format!("{name}: nothing decides its {word}"),
                    env.file,
                    *span,
                    "a made part's sizes must be written",
                )),
                None => this.stock_length(stock, field)?.ok_or_else(|| {
                    this.error(
                        Code::MissingField,
                        format!("made part '{name}' is missing '{word}'"),
                        env.file,
                        body.span,
                        format!("add {word}: ..."),
                    )
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
                .ok_or(Reported)?,
        };
        let drawn = [length, width, thickness];
        // Orientation.
        let mut orientation = None;
        let mut of_part = None;
        for term in &made.place {
            if let MadeTermKind::Orient { length, width, of } = &term.kind {
                let Some(part) = pair.parts.get(*of).copied() else {
                    return Err(Reported);
                };
                let Some(geom) = geoms.get(part) else {
                    return Err(Reported);
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
            return Err(Reported);
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
            let slot = lows.get_mut(axis_index(axis)).ok_or(Reported)?;
            if slot.is_some() {
                return Err(this.error(
                    Code::AxisSetTwice,
                    format!("{name}: position along '{}' is set twice", axis.word()),
                    env.file,
                    span,
                    format!("keep one term for '{}'", axis.word()),
                ));
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
            let (p, q) = match (
                Self::member_part(pair, selector.part),
                Self::member_part(pair, selector.other),
            ) {
                (Some(p), Some(q)) => (p, q),
                _ => return Err(Reported),
            };
            let (Some(gp), Some(gq)) = (geoms.get(p).cloned(), geoms.get(q).cloned()) else {
                return Err(Reported);
            };
            let (_, face, _) = self.selector_place(selector, pair, geoms, env.file)?;
            let (face_axis, _, positive) = gp.face_plane(face);
            // The plane: the contact plane of the drawn boxes for parts that
            // butt; the middle of their overlap for parts that cross.
            let plane = match gp.drawn_box.touching_axis(&gq.drawn_box) {
                Some(axis) if axis == face_axis => {
                    if positive {
                        gp.drawn_box.max(axis)
                    } else {
                        gp.drawn_box.min(axis)
                    }
                }
                _ => {
                    let low = gp.drawn_box.min(face_axis).max(gq.drawn_box.min(face_axis));
                    let high = gp.drawn_box.max(face_axis).min(gq.drawn_box.max(face_axis));
                    midpoint(low, high).ok_or(Reported)?
                }
            };
            let centre_of = |axis: Axis| {
                midpoint(
                    gq.cut_box.min(axis).max(gp.cut_box.min(axis)),
                    gq.cut_box.max(axis).min(gp.cut_box.max(axis)),
                )
                .ok_or(Reported)
            };
            let half = |axis: Axis| {
                size_along(axis)
                    .checked_div(Rational::from(2i64))
                    .ok_or(Reported)
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
                            centre.checked_sub(half(axis)?).ok_or(Reported)?,
                            term.span,
                        )?;
                    }
                }
                MadeTermKind::Center {
                    axis: Some(part_axis),
                    ..
                } => {
                    let Some(of) = of_part.and_then(|p| geoms.get(p)) else {
                        return Err(Reported);
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
                        centre.checked_sub(half(axis)?).ok_or(Reported)?,
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
                    set(self, face_axis, low.ok_or(Reported)?, term.span)?;
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
                    set(self, face_axis, low.ok_or(Reported)?, term.span)?;
                }
                MadeTermKind::Orient { .. } => {}
            }
        }
        let mut extents = Vec::new();
        for axis in [Axis::Across, Axis::Up, Axis::Deep] {
            let Some(Some(low)) = lows.get(axis_index(axis)).copied() else {
                return Err(self.axis_not_set(name, axis, env.file, made.span));
            };
            extents.push((low, low.checked_add(size_along(axis)).ok_or(Reported)?));
        }
        let [across, up, deep] = extents.as_slice() else {
            return Err(Reported);
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
}

impl GeomTable {
    pub fn get(&self, part: PartId) -> Option<&PartGeom> {
        self.parts.get(part.index()).and_then(Option::as_ref)
    }

    pub fn set(&mut self, part: PartId, geom: PartGeom) {
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
    length.to_nearest_length().ok_or(Reported)
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
