//! Evaluating resolved expressions to exact values, on demand.

use std::borrow::Cow;
use std::cell::RefCell;

use stave_lang::ast::Span;
use stave_lang::placement::{Axis, Face};
use stave_lang::units::{ExactLength, Rational};
use stave_lang::{Code, Diagnostic, FileId, Label};
use stave_model::defs::{
    ArrayDef, BaseEntry, BuiltinCall, DefId, Index, Member, OpeningFaces, RExpr, RExprKind,
    SplitBy, StockField, Symbol, Text, TextPart, Word,
};
use stave_model::ids::{InstanceId, KindId, PartId, StockId};
use stave_model::instance::{Ctx, ItemSlot, ParamSource, Slot};
use stave_model::value::{
    self, AnchorOwner, AnchorValue, Envelope, ExactBox, KindValue, ModelRef, Mount, RegionOrigin,
    RegionValue, Spacing, Value,
};
use stave_schema::functions::Builtin;
use stave_schema::kinds::{BaseField, JointSetting};
use stave_schema::words::StockForm;

use crate::fixes::Written;
use crate::{Fault, Frame, Memo, Reported, Solved, Solver};

/// The param values of one application of a kind, worked out on demand
/// and remembered. A [`crate::joints::Binding`] carries one so that every
/// environment built from it shares what was worked out. A copy starts
/// empty, because a copy is made to change the settings (the help search
/// tries other values), and the values remembered would be wrong for it.
#[derive(Default)]
pub(crate) struct ParamCache(RefCell<Vec<Option<Value>>>);

impl ParamCache {
    /// The remembered value of the kind's param `index`.
    fn get(&self, index: usize) -> Option<Value> {
        self.0.borrow().get(index).cloned().flatten()
    }

    /// Remembers the value of the kind's param `index`.
    fn set(&self, index: usize, value: Value) {
        let mut values = self.0.borrow_mut();
        if values.len() <= index {
            values.resize(index.saturating_add(1), None);
        }
        if let Some(slot) = values.get_mut(index) {
            *slot = Some(value);
        }
    }
}

impl Clone for ParamCache {
    fn clone(&self) -> Self {
        ParamCache::default()
    }
}

impl PartialEq for ParamCache {
    /// What is remembered follows from the settings: it never makes two
    /// bindings differ.
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl std::fmt::Debug for ParamCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ParamCache")
    }
}

/// A kind environment's param values: its own, or a binding's, shared.
enum Params<'e> {
    Own(ParamCache),
    Shared(&'e ParamCache),
}

/// The roles and settings of one application of a kind. It borrows them
/// where it can: from the binding of a joint or a piece of hardware, or
/// from a kind value.
pub(crate) struct KindEnv<'e> {
    pub kind: KindId,
    /// Per role: a part or a region; empty for a kind value.
    pub roles: Cow<'e, [Value]>,
    /// Settings from the kind value and the call.
    pub settings: Cow<'e, [(String, Value)]>,
    /// Param values, worked out on demand.
    params: Params<'e>,
    /// Parts the kind made for this pair.
    pub made: Cow<'e, [PartId]>,
    /// For a piece of hardware once it is placed: its mount face, as an
    /// anchor (`mount` in its kind's rules).
    pub mount: Option<AnchorValue>,
}

impl<'e> KindEnv<'e> {
    /// An environment with its own (empty) param values.
    pub fn new(
        kind: KindId,
        roles: impl Into<Cow<'e, [Value]>>,
        settings: impl Into<Cow<'e, [(String, Value)]>>,
    ) -> Self {
        KindEnv {
            kind,
            roles: roles.into(),
            settings: settings.into(),
            params: Params::Own(ParamCache::default()),
            made: Cow::Borrowed(&[]),
            mount: None,
        }
    }

    /// The environment of a binding: its roles and settings, borrowed, and
    /// the param values it remembers, shared with every other environment
    /// of the same binding.
    pub fn of_binding(binding: &'e crate::joints::Binding) -> Self {
        KindEnv {
            kind: binding.kind,
            roles: Cow::Borrowed(&binding.roles),
            settings: Cow::Borrowed(&binding.settings),
            params: Params::Shared(&binding.params),
            made: Cow::Borrowed(&[]),
            mount: None,
        }
    }

    fn params(&self) -> &ParamCache {
        match &self.params {
            Params::Own(params) => params,
            Params::Shared(params) => params,
        }
    }
}

/// Where an expression is evaluated.
#[derive(Clone, Copy)]
pub(crate) struct Env<'e> {
    pub ctx: Ctx,
    pub file: FileId,
    pub kind: Option<&'e KindEnv<'e>>,
    /// The array being declared and its element count, for `[same]` and
    /// `split(region, axis)`.
    pub array: Option<(DefId, usize)>,
}

/// An array's elements, from an indexed name.
enum ArrayItems<'m> {
    Parts(&'m [PartId]),
    Regions(&'m [stave_model::ids::RegionId]),
    Assemblies(&'m [stave_model::ids::AssemblyId]),
    Joints(&'m [stave_model::ids::JointId]),
    Hardware(&'m [stave_model::ids::HardwareId]),
}

impl ArrayItems<'_> {
    fn len(&self) -> usize {
        match self {
            ArrayItems::Parts(v) => v.len(),
            ArrayItems::Regions(v) => v.len(),
            ArrayItems::Assemblies(v) => v.len(),
            ArrayItems::Joints(v) => v.len(),
            ArrayItems::Hardware(v) => v.len(),
        }
    }

    fn get(&self, index: usize) -> Option<Value> {
        Some(match self {
            ArrayItems::Parts(v) => Value::Part(*v.get(index)?),
            ArrayItems::Regions(v) => Value::Region(*v.get(index)?),
            ArrayItems::Assemblies(v) => Value::Assembly(*v.get(index)?),
            ArrayItems::Joints(v) => Value::Joint(*v.get(index)?),
            ArrayItems::Hardware(v) => Value::Hardware(*v.get(index)?),
        })
    }

    fn all(self) -> Value {
        match self {
            ArrayItems::Parts(v) => Value::Parts(v.to_vec()),
            ArrayItems::Regions(v) => Value::List(v.iter().copied().map(Value::Region).collect()),
            ArrayItems::Assemblies(v) => Value::Assemblies(v.to_vec()),
            ArrayItems::Joints(v) => Value::List(v.iter().copied().map(Value::Joint).collect()),
            ArrayItems::Hardware(v) => {
                Value::List(v.iter().copied().map(Value::Hardware).collect())
            }
        }
    }
}

fn pick<T: Copy>(all: &[T], element: Option<usize>) -> Option<T> {
    match element {
        Some(index) => all.get(index).copied(),
        None if all.len() == 1 => all.first().copied(),
        None => None,
    }
}

fn slot_value(slot: &ItemSlot, element: Option<usize>) -> Option<Value> {
    Some(match slot {
        ItemSlot::Part(Slot::One(p)) => Value::Part(*p),
        ItemSlot::Region(Slot::One(r)) => Value::Region(*r),
        ItemSlot::Assembly(Slot::One(a)) => Value::Assembly(*a),
        ItemSlot::Joint(Slot::One(j)) => Value::Joint(*j),
        ItemSlot::Hardware(Slot::One(h)) => Value::Hardware(*h),
        ItemSlot::Part(Slot::Many(all)) => Value::Part(pick(all, element)?),
        ItemSlot::Region(Slot::Many(all)) => Value::Region(pick(all, element)?),
        ItemSlot::Assembly(Slot::Many(all)) => Value::Assembly(pick(all, element)?),
        ItemSlot::Joint(Slot::Many(all)) => Value::Joint(pick(all, element)?),
        ItemSlot::Hardware(Slot::Many(all)) => Value::Hardware(pick(all, element)?),
        ItemSlot::Param(_) | ItemSlot::Let(_) | ItemSlot::Require(_) => return None,
    })
}

impl<'m> Solver<'m> {
    pub(crate) fn module_ctx(&self, def: DefId) -> Option<Ctx> {
        let scope = self.model.defs.decl(def)?.scope;
        let instance = self.model.scope_instance(
            scope,
            Ctx {
                instance: InstanceId::from_usize(0),
                element: None,
            },
        )?;
        Some(Ctx {
            instance,
            element: None,
        })
    }

    fn eval_error(
        &mut self,
        env: &Env<'_>,
        span: Span,
        message: impl Into<String>,
        help: impl Into<String>,
    ) -> Reported {
        self.error(Code::TypeMismatch, message, env.file, span, help)
    }

    /// Evaluates an expression.
    pub(crate) fn eval(&mut self, expr: &RExpr, env: &Env<'_>) -> Solved<Value> {
        let span = expr.span;
        match &expr.kind {
            RExprKind::Length(length) => Ok(Value::Length(*length)),
            RExprKind::Angle(angle) => Ok(Value::Angle(*angle)),
            RExprKind::Number(number) => Ok(if expr.ty == stave_model::types::Ty::Count {
                Value::Count(*number)
            } else {
                Value::Ratio(*number)
            }),
            RExprKind::Bool(b) => Ok(Value::Bool(*b)),
            RExprKind::Text(text) => Ok(Value::Text(self.format_text(text, env)?)),
            RExprKind::Fit => Err(self
                .eval_error(
                    env,
                    span,
                    "'fit' is only a part's length or width",
                    "write a length",
                )
                .into()),
            RExprKind::Word(word) => Ok(Value::Word(*word)),
            RExprKind::Symbol(symbol) => self.symbol(*symbol, env, span),
            RExprKind::SelfElement(def) => {
                let slot = self.model.item(*def, env.ctx);
                Ok(slot
                    .and_then(|slot| slot_value(slot, env.ctx.element))
                    .ok_or(Fault::Earlier)?)
            }
            RExprKind::Member { base, member } => {
                let base_value = self.eval(base, env)?;
                self.member(&base_value, member, env, span)
            }
            RExprKind::Index { base, index } => self.index(base, index, env, span),
            // A rule that reads a wall is measured by the fit check once
            // every cut is made (`walls`); nothing else may read one.
            RExprKind::Wall(_) => Err(self
                .eval_error(
                    env,
                    span,
                    "wall(...) can't be worked out here".to_owned(),
                    "wall(...) is read only as one side of a kind's require",
                )
                .into()),
            RExprKind::Builtin(call) => self.builtin(call, env, span),
            RExprKind::KindValue { kind, settings } => {
                let base = self.eval(kind, env)?;
                let Value::Kind(mut value) = base else {
                    return Err(self
                        .eval_error(
                            env,
                            kind.span,
                            format!("'{}' is not a kind", base.description()),
                            "name a kind",
                        )
                        .into());
                };
                for setting in settings {
                    let joint_setting = JointSetting::from_word(&setting.name);
                    let given = match joint_setting {
                        Some(JointSetting::Glue) => value.glue.is_some(),
                        Some(JointSetting::Methods) => !value.methods.is_empty(),
                        None => value.setting(&setting.name).is_some(),
                    };
                    if given {
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
                    let v = self.eval(&setting.value, env)?;
                    match joint_setting {
                        Some(JointSetting::Glue) => value.glue = Some(v == Value::Bool(true)),
                        Some(JointSetting::Methods) => value.methods = recipes(&v),
                        None => value.settings.push((setting.name.clone(), v)),
                    }
                }
                Ok(Value::Kind(value))
            }
            RExprKind::Unary { op, operand } => {
                let operand = self.eval(operand, env)?;
                value::unary(*op, &operand).map_err(|e| self.arithmetic(e, env, span).into())
            }
            RExprKind::Binary { op, lhs, rhs } => {
                let lhs = self.eval(lhs, env)?;
                let rhs = self.eval(rhs, env)?;
                value::binary(*op, &lhs, &rhs).map_err(|e| self.arithmetic(e, env, span).into())
            }
            RExprKind::List(items) => {
                let mut values = Vec::new();
                for item in items {
                    match self.eval(item, env)? {
                        Value::List(inner) => values.extend(inner),
                        Value::Parts(parts) => values.extend(parts.into_iter().map(Value::Part)),
                        other => values.push(other),
                    }
                }
                if !values.is_empty() && values.iter().all(|v| matches!(v, Value::Part(_))) {
                    let parts = values.iter().filter_map(|v| match v {
                        Value::Part(p) => Some(*p),
                        _ => None,
                    });
                    return Ok(Value::Parts(parts.collect()));
                }
                Ok(Value::List(values))
            }
            // Resolving reported this expression.
            RExprKind::Error => Err(Fault::Earlier.into()),
        }
    }

    fn arithmetic(&mut self, error: value::ValueError, env: &Env<'_>, span: Span) -> Reported {
        let code = match error {
            value::ValueError::Mismatch { .. } => Code::UnitMismatch,
            value::ValueError::Overflow | value::ValueError::DivisionByZero => Code::TypeMismatch,
        };
        self.error(
            code,
            error.to_string(),
            env.file,
            span,
            "check the values this expression is worked out from",
        )
    }

    pub(crate) fn eval_length(&mut self, expr: &RExpr, env: &Env<'_>) -> Solved<ExactLength> {
        match self.eval(expr, env)? {
            Value::Length(length) => Ok(length),
            other => Err(self
                .error(
                    Code::UnitMismatch,
                    format!("this needs a length, found {}", other.description()),
                    env.file,
                    expr.span,
                    "write a length with a unit, such as 19mm",
                )
                .into()),
        }
    }

    pub(crate) fn eval_angle(
        &mut self,
        expr: &RExpr,
        env: &Env<'_>,
    ) -> Solved<stave_lang::units::ExactAngle> {
        match self.eval(expr, env)? {
            Value::Angle(angle) => Ok(angle),
            other => Err(self
                .error(
                    Code::UnitMismatch,
                    format!("this needs an angle, found {}", other.description()),
                    env.file,
                    expr.span,
                    "write an angle, such as 45deg",
                )
                .into()),
        }
    }

    pub(crate) fn eval_bool(&mut self, expr: &RExpr, env: &Env<'_>) -> Solved<bool> {
        match self.eval(expr, env)? {
            Value::Bool(b) => Ok(b),
            other => Err(self
                .eval_error(
                    env,
                    expr.span,
                    format!("this needs true or false, found {}", other.description()),
                    "write a comparison",
                )
                .into()),
        }
    }

    pub(crate) fn eval_count(&mut self, expr: &RExpr, env: &Env<'_>) -> Solved<u32> {
        let value = self.eval(expr, env)?;
        match value.number() {
            Some(n) if n.is_integer() && !n.is_negative() => {
                u32::try_from(n.numerator()).map_err(|_| {
                    self.eval_error(
                        env,
                        expr.span,
                        "this count is too large",
                        "use a smaller count",
                    )
                    .into()
                })
            }
            _ => Err(self
                .eval_error(
                    env,
                    expr.span,
                    format!("this needs a whole number, found {value}"),
                    "write a count such as 4",
                )
                .into()),
        }
    }

    pub(crate) fn eval_part(&mut self, expr: &RExpr, env: &Env<'_>) -> Solved<PartId> {
        match self.eval(expr, env)? {
            Value::Part(part) => Ok(part),
            other => Err(self
                .eval_error(
                    env,
                    expr.span,
                    format!("this needs a part, found {}", other.description()),
                    "name a part",
                )
                .into()),
        }
    }

    pub(crate) fn eval_anchor(&mut self, expr: &RExpr, env: &Env<'_>) -> Solved<AnchorValue> {
        match self.eval(expr, env)? {
            Value::Anchor(anchor) => Ok(anchor),
            other => Err(self
                .eval_error(
                    env,
                    expr.span,
                    format!(
                        "this needs a face or mid-plane, found {}",
                        other.description()
                    ),
                    "name a face, such as base.top",
                )
                .into()),
        }
    }

    // -----------------------------------------------------------------------
    // Names
    // -----------------------------------------------------------------------

    fn symbol(&mut self, symbol: Symbol, env: &Env<'_>, span: Span) -> Solved<Value> {
        match symbol {
            Symbol::Def(def) => self.def_value(def, env, span),
            Symbol::Role(_, index) => env
                .kind
                .and_then(|k| k.roles.get(index.index()).cloned())
                .ok_or_else(|| self.eval_error(env, span, "a role has no part here", "roles are only bound where the kind is used in a joint or hardware declaration").into()),
            Symbol::KindParam(_, index) => match env.kind {
                Some(kind_env) => self.kind_param(kind_env, index, env.file, span),
                None => Err(self.eval_error(env, span, "a kind's param has no value here", "use the kind in a joint or hardware declaration").into()),
            },
            Symbol::MadePart(_, index) => env
                .kind
                .and_then(|k| k.made.get(index.index()).copied())
                .map(Value::Part)
                .ok_or_else(|| self.eval_error(env, span, "the made part doesn't exist yet here", "refer to made parts only in cuts and assembly steps").into()),
            Symbol::Mount(_) => env
                .kind
                .and_then(|k| k.mount)
                .map(Value::Anchor)
                .ok_or_else(|| self.eval_error(env, span, "the mount face is known only where the hardware is placed", "use mount in the kind's require entries").into()),
        }
    }

    fn def_value(&mut self, def: DefId, env: &Env<'_>, span: Span) -> Solved<Value> {
        match def {
            DefId::Stock(id) => return Ok(Value::Stock(id)),
            DefId::Profile(id) => return Ok(Value::Profile(id)),
            DefId::Recipe(id) => return Ok(Value::Recipe(id)),
            DefId::Kind(id) => {
                let is_tool = self
                    .model
                    .defs
                    .kind(id)
                    .is_some_and(|k| k.base() == stave_lang::ast::BaseKind::Tool);
                return Ok(if is_tool {
                    Value::Tool(id)
                } else {
                    Value::Kind(KindValue::new(
                        id,
                        self.model.defs.kind_display(id, Some(env.file)),
                    ))
                });
            }
            DefId::Component(_) | DefId::Scene(_) => {
                return Err(self
                    .eval_error(
                        env,
                        span,
                        "this is not a value",
                        "name a part, region or value",
                    )
                    .into());
            }
            _ => {}
        }
        let Some(slot) = self.model.item(def, env.ctx) else {
            return Err(self
                .eval_error(
                    env,
                    span,
                    "this item doesn't exist here",
                    "items of a project can't be used from a library",
                )
                .into());
        };
        match slot {
            ItemSlot::Param(param) => self.param_value(*param),
            ItemSlot::Let(binding) => self.let_value(*binding),
            ItemSlot::Require(require) => {
                let Some(inst) = self.model.requires.get(require.index()) else {
                    return Err(Fault::Internal("a require").into());
                };
                let Some(def) = self.model.defs.require(inst.def) else {
                    return Err(Fault::Internal("a require's declaration").into());
                };
                let env = Env {
                    ctx: inst.ctx,
                    file: def.decl.file,
                    kind: None,
                    array: None,
                };
                Ok(Value::Bool(self.eval_bool(&def.condition, &env)?))
            }
            other => slot_value(other, None).ok_or_else(|| {
                self.eval_error(
                    env,
                    span,
                    "this is an array",
                    "index it: name[label], or name[*] for all of them",
                )
                .into()
            }),
        }
    }

    pub(crate) fn param_value(&mut self, param: stave_model::ids::ParamId) -> Solved<Value> {
        match self.params.get(param.index()) {
            Some(Memo::Done(value)) => return Ok(value.clone()),
            Some(Memo::Failed(accounted)) => return Err((*accounted).into()),
            Some(Memo::Busy) => return Err(self.cycle(Frame::Param(param)).into()),
            Some(Memo::Todo) => {}
            None => return Err(Fault::Internal("a param").into()),
        }
        let Some(inst) = self.model.params.get(param.index()) else {
            return Err(Fault::Internal("a param").into());
        };
        let Some(def) = self.model.defs.param(inst.def) else {
            return Err(Fault::Internal("a param's declaration").into());
        };
        if self.poisoned(DefId::Param(inst.def)) {
            return Err(self.skip().into());
        }
        if let Some(slot) = self.params.get_mut(param.index()) {
            *slot = Memo::Busy;
        }
        self.enter(
            Frame::Param(param),
            Some((def.decl.file, def.decl.name_span)),
        );
        let result = match inst.source {
            ParamSource::SameAs(other) => self.param_value(other),
            ParamSource::Default => match &def.default {
                Some(default) => {
                    let env = Env {
                        ctx: Ctx {
                            instance: inst.instance,
                            element: None,
                        },
                        file: def.decl.file,
                        kind: None,
                        array: None,
                    };
                    let value = self.eval(default, &env);
                    // No `?` here: a value outside the domain must still
                    // reach `leave` and the memo below, or the slot stays
                    // busy and the next read reports a false cycle.
                    match (value, def.decl.scope) {
                        (Ok(value), stave_model::defs::Scope::Module(_)) => {
                            let domain_ctx = env.ctx;
                            self.check_domain(
                                def,
                                &value,
                                Written {
                                    file: def.decl.file,
                                    span: default.span,
                                    path: self
                                        .model
                                        .defs
                                        .patch_path(DefId::Param(inst.def), self.model.root),
                                },
                                &inst.name,
                                domain_ctx,
                            )
                            .map(|()| value)
                        }
                        (value, _) => value,
                    }
                }
                None => Err(Fault::Earlier.into()),
            },
            ParamSource::Argument {
                assembly,
                index,
                ctx,
            } => {
                let arg = self
                    .model
                    .defs
                    .assembly(assembly)
                    .and_then(|a| match &a.kind {
                        stave_model::defs::AssemblyKind::Instance { args, .. } => {
                            args.get(index).map(|(_, e)| e)
                        }
                        _ => None,
                    });
                let file = self
                    .model
                    .defs
                    .assembly(assembly)
                    .map(|a| a.decl.file)
                    .unwrap_or(def.decl.file);
                let array = self.model.defs.array(DefId::Assembly(assembly)).map(|_| {
                    let len = self.array_len(DefId::Assembly(assembly), ctx);
                    (DefId::Assembly(assembly), len)
                });
                match arg {
                    Some(arg) => {
                        let env = Env {
                            ctx,
                            file,
                            kind: None,
                            array,
                        };
                        let value = self.eval(arg, &env);
                        // As for a default: no `?`, so the memo is settled.
                        value.and_then(|value| {
                            let domain_ctx = Ctx {
                                instance: inst.instance,
                                element: None,
                            };
                            let target = self
                                .model
                                .defs
                                .patch_path(DefId::Assembly(assembly), self.model.root)
                                .map(|path| format!("{path}.{}", def.decl.name));
                            self.check_domain(
                                def,
                                &value,
                                Written {
                                    file,
                                    span: arg.span,
                                    path: target,
                                },
                                &inst.name,
                                domain_ctx,
                            )
                            .map(|()| value)
                        })
                    }
                    None => Err(Fault::Earlier.into()),
                }
            }
        };
        self.leave();
        let (memo, result) = self.remember(result, def.decl.file, def.decl.name_span);
        if let Some(slot) = self.params.get_mut(param.index()) {
            *slot = memo;
        }
        result
    }

    /// `out-of-range` for a value outside its param's `within`. The domain
    /// is evaluated in `domain_ctx`, where the param is declared.
    pub(crate) fn check_domain(
        &mut self,
        def: &stave_model::defs::ParamDef,
        value: &Value,
        written: Written,
        name: &str,
        domain_ctx: Ctx,
    ) -> Solved<()> {
        let Some(domain) = &def.domain else {
            return Ok(());
        };
        let module_env = Env {
            ctx: domain_ctx,
            file: def.decl.file,
            kind: None,
            array: None,
        };
        // Whether the value is inside, the domain as messages write it, and
        // the allowed value nearest to it.
        let (inside, text, nearest) = match domain {
            stave_model::defs::Domain::Range { low, high, .. } => {
                let low = self.eval(low, &module_env)?;
                let high = self.eval(high, &module_env)?;
                let above = value::binary(stave_lang::ast::BinaryOp::Ge, value, &low).ok()
                    == Some(Value::Bool(true));
                let below = value::binary(stave_lang::ast::BinaryOp::Le, value, &high).ok()
                    == Some(Value::Bool(true));
                let text = format!("{low}..{high}");
                (above && below, text, if above { high } else { low })
            }
            stave_model::defs::Domain::OneOf { values, .. } => {
                let mut list = Vec::new();
                for v in values {
                    list.push(self.eval(v, &module_env)?);
                }
                let inside = list.contains(value);
                let nearest = crate::fixes::nearest(value, &list).cloned();
                let text = list.iter().map(Value::to_string).collect::<Vec<_>>();
                let text = format!("[{}]", text.join(", "));
                match nearest {
                    Some(nearest) => (inside, text, nearest),
                    None => (inside, text, value.clone()),
                }
            }
        };
        if inside {
            return Ok(());
        }
        let short = name.rsplit('.').next().unwrap_or(name);
        let mut diagnostic = Diagnostic::error(
            Code::OutOfRange,
            format!("'{short}' must be within {text}, found {value}"),
            Label::new(written.file, written.span),
        )
        .with_help(format!("give a value within {text}"));
        // The fix sets the value to the nearest allowed one, where a patch
        // can reach the value (grammar, "Fixes").
        if &nearest != value
            && let Some(fix) = written
                .path
                .and_then(|path| crate::fixes::set_to(&path, &nearest))
        {
            diagnostic = diagnostic.with_fix(fix);
        }
        let reported = self.report(diagnostic);
        Err(reported.into())
    }

    pub(crate) fn let_value(&mut self, binding: stave_model::ids::LetId) -> Solved<Value> {
        match self.lets.get(binding.index()) {
            Some(Memo::Done(value)) => return Ok(value.clone()),
            Some(Memo::Failed(accounted)) => return Err((*accounted).into()),
            Some(Memo::Busy) => return Err(self.cycle(Frame::Let(binding)).into()),
            Some(Memo::Todo) => {}
            None => return Err(Fault::Internal("a let").into()),
        }
        let Some(inst) = self.model.lets.get(binding.index()) else {
            return Err(Fault::Internal("a let").into());
        };
        let Some(def) = self.model.defs.let_def(inst.def) else {
            return Err(Fault::Internal("a let's declaration").into());
        };
        if self.poisoned(DefId::Let(inst.def)) {
            return Err(self.skip().into());
        }
        if let Some(slot) = self.lets.get_mut(binding.index()) {
            *slot = Memo::Busy;
        }
        self.enter(
            Frame::Let(binding),
            Some((def.decl.file, def.decl.name_span)),
        );
        let env = Env {
            ctx: inst.ctx,
            file: def.decl.file,
            kind: None,
            array: None,
        };
        let result = self.eval(&def.value, &env);
        self.leave();
        let (memo, result) = self.remember(result, def.decl.file, def.decl.name_span);
        if let Some(slot) = self.lets.get_mut(binding.index()) {
            *slot = memo;
        }
        result
    }

    /// A param of a kind application: the setting, or its default for
    /// this pair.
    pub(crate) fn kind_param(
        &mut self,
        kind_env: &KindEnv<'_>,
        index: usize,
        file: FileId,
        span: Span,
    ) -> Solved<Value> {
        if self.poisoned(DefId::Kind(kind_env.kind)) {
            return Err(self.skip().into());
        }
        if let Some(value) = kind_env.params().get(index) {
            return Ok(value);
        }
        let Some(kind) = self.model.defs.kind(kind_env.kind) else {
            return Err(Fault::Internal("a kind's declaration").into());
        };
        let Some(param) = kind.params.get(index) else {
            return Err(Fault::Internal("a kind's setting").into());
        };
        let value = match kind_env
            .settings
            .iter()
            .find(|(n, _)| *n == param.decl.name)
        {
            Some((_, value)) => value.clone(),
            None => match &param.default {
                Some(default) => {
                    let Some(ctx) = self.module_ctx(DefId::Kind(kind_env.kind)) else {
                        return Err(Fault::Internal("the file an item is declared in").into());
                    };
                    let env = Env {
                        ctx,
                        file: kind.decl.file,
                        kind: Some(kind_env),
                        array: None,
                    };
                    self.eval(default, &env)?
                }
                None => {
                    return Err(self
                        .error(
                            Code::MissingField,
                            format!(
                                "kind '{}' is missing '{}'",
                                self.model.defs.kind_display(kind_env.kind, Some(file)),
                                param.decl.name
                            ),
                            file,
                            span,
                            format!("give it a value: {}: ...", param.decl.name),
                        )
                        .into());
                }
            },
        };
        kind_env.params().set(index, value.clone());
        Ok(value)
    }

    /// One of the entries a kind value's base fixes (a fastener's
    /// `diameter`, a tool's `title`), or `None` when its kind gives none.
    pub(crate) fn base_field(
        &mut self,
        kind_value: &KindValue,
        field: BaseField,
        file: FileId,
        span: Span,
    ) -> Solved<Option<Value>> {
        let kind_env = KindEnv::new(kind_value.kind, Vec::new(), &kind_value.settings);
        self.base_field_in(&kind_env, field, file, span)
    }

    /// One of the entries a kind's base fixes, worked out with the roles
    /// and settings of `kind_env`, or `None` when the kind gives none.
    pub(crate) fn base_field_in(
        &mut self,
        kind_env: &KindEnv<'_>,
        field: BaseField,
        file: FileId,
        span: Span,
    ) -> Solved<Option<Value>> {
        if self.poisoned(DefId::Kind(kind_env.kind)) {
            return Err(self.skip().into());
        }
        let Some(kind) = self.model.defs.kind(kind_env.kind) else {
            return Err(Fault::Internal("a kind's declaration").into());
        };
        match kind.base_entry(field) {
            None => Ok(None),
            Some(BaseEntry::Param(index)) => self.kind_param(kind_env, index, file, span).map(Some),
            Some(BaseEntry::Field(index)) => {
                let Some(written) = kind.fields().get(index) else {
                    return Err(Fault::Internal("a kind's field").into());
                };
                self.kind_field(kind_env, &written.value).map(Some)
            }
        }
    }

    /// A field of the kind of `kind_env`, worked out in the kind's file.
    fn kind_field(&mut self, kind_env: &KindEnv<'_>, value: &RExpr) -> Solved<Value> {
        let Some(kind) = self.model.defs.kind(kind_env.kind) else {
            return Err(Fault::Internal("a kind's declaration").into());
        };
        let Some(ctx) = self.module_ctx(DefId::Kind(kind_env.kind)) else {
            return Err(Fault::Internal("the file an item is declared in").into());
        };
        let env = Env {
            ctx,
            file: kind.decl.file,
            kind: Some(kind_env),
            array: None,
        };
        self.eval(value, &env)
    }

    /// A param or field of a kind value (`slide.clearance`).
    pub(crate) fn kind_entry(
        &mut self,
        kind_value: &KindValue,
        name: &str,
        file: FileId,
        span: Span,
    ) -> Solved<Value> {
        if self.poisoned(DefId::Kind(kind_value.kind)) {
            return Err(self.skip().into());
        }
        if let Some(value) = kind_value.setting(name) {
            return Ok(value.clone());
        }
        let Some(kind) = self.model.defs.kind(kind_value.kind) else {
            return Err(Fault::Internal("a kind's declaration").into());
        };
        let kind_env = KindEnv::new(kind_value.kind, Vec::new(), &kind_value.settings);
        if let Some(index) = kind.param(name) {
            return self.kind_param(&kind_env, index, file, span);
        }
        if let Some(field) = kind.field(name) {
            return self.kind_field(&kind_env, &field.value);
        }
        let kind_name = self.model.defs.kind_display(kind_value.kind, Some(file));
        Err(self
            .error(
                Code::UnknownField,
                format!("the kind '{kind_name}' has no field '{name}'"),
                file,
                span,
                stave_model::suggest::did_you_mean(
                    name,
                    kind.params
                        .iter()
                        .map(|p| p.decl.name.as_str())
                        .chain(kind.fields().iter().map(|f| f.name.as_str())),
                )
                .unwrap_or_else(|| "a kind value can be read for its params and fields".to_owned()),
            )
            .into())
    }

    // -----------------------------------------------------------------------
    // Members and indexes
    // -----------------------------------------------------------------------

    fn member(
        &mut self,
        base: &Value,
        member: &Member,
        env: &Env<'_>,
        span: Span,
    ) -> Solved<Value> {
        match (base, member) {
            (Value::Part(part), Member::Size(axis)) => {
                Ok(Value::Length(self.drawn_size(*part, *axis)?))
            }
            (Value::Parts(parts), Member::Size(axis)) => {
                self.parts_size(parts, *axis).map(Value::Length)
            }
            (Value::Part(part), Member::Diameter) => {
                self.part_diameter(*part, env, span).map(Value::Length)
            }
            (Value::Part(part), Member::PartStock) => self.part_stock(*part).map(Value::Stock),
            (Value::Part(part), Member::Face(face)) => {
                self.part_anchor(*part, *face).map(Value::Anchor)
            }
            (Value::Region(region), Member::Face(face)) => {
                let bounds = self.region_value(*region)?.bounds;
                Ok(Value::Anchor(AnchorValue {
                    axis: face.axis(),
                    position: bounds.face(*face),
                    face: Some(*face),
                    owner: AnchorOwner::Region(*region),
                }))
            }
            (Value::Region(region), Member::RegionSize(axis)) => {
                let bounds = self.region_value(*region)?.bounds;
                Ok(Value::Length(bounds.size(*axis).ok_or(Fault::Overflow)?))
            }
            (Value::Stock(stock), Member::StockField(field)) => {
                self.stock_field(*stock, *field, span)
            }
            (Value::Profile(profile), Member::ProfileLength) => {
                self.profile_length(*profile).map(Value::Length)
            }
            (Value::Assembly(assembly), Member::AssemblyItem(def)) => {
                let Some(inner) = self.model.assemblies.get(assembly.index()).map(|a| a.inner)
                else {
                    return Err(Fault::Internal("a component").into());
                };
                let ctx = Ctx {
                    instance: inner,
                    element: None,
                };
                let inner_env = Env { ctx, ..*env };
                self.def_value(*def, &inner_env, span)
            }
            (Value::Patch(patch), Member::PatchSize(size)) => Ok(Value::Length(patch.size(*size))),
            (Value::Kind(kind_value), Member::KindEntry(name)) => {
                self.kind_entry(kind_value, name, env.file, span)
            }
            (value, Member::KindEntry(name)) => {
                // A member of a value whose type was only known now.
                let typed = match (value, name.as_str()) {
                    (Value::Part(_), "length") => {
                        Some(Member::Size(stave_lang::placement::PartAxis::Length))
                    }
                    (Value::Part(_), "width") => {
                        Some(Member::Size(stave_lang::placement::PartAxis::Width))
                    }
                    (Value::Part(_), "thickness") => {
                        Some(Member::Size(stave_lang::placement::PartAxis::Thickness))
                    }
                    (Value::Parts(_), "length") => {
                        Some(Member::Size(stave_lang::placement::PartAxis::Length))
                    }
                    (Value::Parts(_), "width") => {
                        Some(Member::Size(stave_lang::placement::PartAxis::Width))
                    }
                    (Value::Parts(_), "thickness") => {
                        Some(Member::Size(stave_lang::placement::PartAxis::Thickness))
                    }
                    (Value::Part(_), "diameter") => Some(Member::Diameter),
                    (Value::Part(_), "stock") => Some(Member::PartStock),
                    (Value::Part(_) | Value::Region(_), word)
                        if Face::from_word(word).is_some() =>
                    {
                        Face::from_word(word).map(Member::Face)
                    }
                    (Value::Region(_), word) => Axis::from_word(word).map(Member::RegionSize),
                    (Value::Stock(_), word) => StockField::from_name(word).map(Member::StockField),
                    (Value::Profile(_), "length") => Some(Member::ProfileLength),
                    (Value::Patch(_), word) => {
                        stave_schema::functions::PatchSize::from_word(word).map(Member::PatchSize)
                    }
                    _ => None,
                };
                match typed {
                    Some(member) => self.member(value, &member, env, span),
                    None => Err(self
                        .error(
                            Code::UnknownField,
                            format!("{} has no field '{name}'", value.description()),
                            env.file,
                            span,
                            "check the name of the field",
                        )
                        .into()),
                }
            }
            (value, _) => Err(self
                .eval_error(
                    env,
                    span,
                    format!("{} has no such field", value.description()),
                    "check what this refers to",
                )
                .into()),
        }
    }

    /// The size of the box around several parts along an axis of the
    /// first: a kind's only `Parts` role read as one (grammar, "Roles and
    /// pairs").
    fn parts_size(
        &mut self,
        parts: &[PartId],
        axis: stave_lang::placement::PartAxis,
    ) -> Solved<ExactLength> {
        let Some(first) = parts.first().copied() else {
            return Err(Fault::Earlier.into());
        };
        let world = self.part_orientation(first)?.direction(axis).axis();
        let mut low: Option<ExactLength> = None;
        let mut high: Option<ExactLength> = None;
        for part in parts {
            let bounds = self.drawn_box(*part)?;
            low = Some(low.map_or(bounds.min(world), |l| l.min(bounds.min(world))));
            high = Some(high.map_or(bounds.max(world), |h| h.max(bounds.max(world))));
        }
        match (low, high) {
            (Some(low), Some(high)) => Ok(high.checked_sub(low).ok_or(Fault::Overflow)?),
            _ => Err(Fault::Internal("the size of several parts").into()),
        }
    }

    fn array_items(&mut self, base: &RExpr, env: &Env<'_>) -> Solved<(ArrayItems<'m>, DefId)> {
        let (def, ctx) = match &base.kind {
            RExprKind::Symbol(Symbol::Def(def)) => (*def, env.ctx),
            RExprKind::Member {
                base: inner,
                member: Member::AssemblyItem(def),
            } => {
                let Value::Assembly(assembly) = self.eval(inner, env)? else {
                    return Err(Fault::Earlier.into());
                };
                let Some(instance) = self.model.assemblies.get(assembly.index()).map(|a| a.inner)
                else {
                    return Err(Fault::Internal("a component").into());
                };
                (
                    *def,
                    Ctx {
                        instance,
                        element: None,
                    },
                )
            }
            _ => {
                return Err(self
                    .eval_error(
                        env,
                        base.span,
                        "this can't be indexed",
                        "index an array declared with [labels] or [count]",
                    )
                    .into());
            }
        };
        let model = self.model;
        let Some(slot) = model.item(def, ctx) else {
            return Err(Fault::Internal("an item").into());
        };
        let items = match slot {
            ItemSlot::Part(slot) => ArrayItems::Parts(slot.as_slice()),
            ItemSlot::Region(slot) => ArrayItems::Regions(slot.as_slice()),
            ItemSlot::Assembly(slot) => ArrayItems::Assemblies(slot.as_slice()),
            ItemSlot::Joint(slot) => ArrayItems::Joints(slot.as_slice()),
            ItemSlot::Hardware(slot) => ArrayItems::Hardware(slot.as_slice()),
            ItemSlot::Param(_) | ItemSlot::Let(_) | ItemSlot::Require(_) => {
                return Err(Fault::Earlier.into());
            }
        };
        Ok((items, def))
    }

    pub(crate) fn array_len(&self, def: DefId, ctx: Ctx) -> usize {
        match self.model.item(def, ctx) {
            Some(ItemSlot::Part(slot)) => slot.as_slice().len(),
            Some(ItemSlot::Region(slot)) => slot.as_slice().len(),
            Some(ItemSlot::Assembly(slot)) => slot.as_slice().len(),
            Some(ItemSlot::Joint(slot)) => slot.as_slice().len(),
            Some(ItemSlot::Hardware(slot)) => slot.as_slice().len(),
            _ => match self.model.defs.array(def) {
                Some(ArrayDef::Labels(labels)) => labels.len(),
                _ => 0,
            },
        }
    }

    fn labels(&self, def: DefId) -> Option<&[(String, Span)]> {
        match self.model.defs.array(def)? {
            ArrayDef::Labels(labels) => Some(labels),
            ArrayDef::Count(_) => None,
        }
    }

    fn index(&mut self, base: &RExpr, index: &Index, env: &Env<'_>, span: Span) -> Solved<Value> {
        let (items, def) = self.array_items(base, env)?;
        match index {
            Index::All => Ok(items.all()),
            Index::Label(position) => Ok(items
                .get(*position)
                .ok_or(Fault::Internal("a labelled element of an array"))?),
            Index::Number(number) => {
                let n = self.eval_count(number, env)?;
                let position = usize::try_from(n).unwrap_or(usize::MAX);
                items.get(position).ok_or_else(|| {
                    self.error(
                        Code::UnknownName,
                        format!("unknown name '{n}'"),
                        env.file,
                        number.span,
                        format!("the array has {} elements, numbered from 0", items.len()),
                    )
                    .into()
                })
            }
            Index::Same => {
                let (Some((declared, count)), Some(element)) = (env.array, env.ctx.element) else {
                    return Err(self
                        .eval_error(
                            env,
                            span,
                            "'[same]' is only allowed inside the declaration of an array",
                            "name the element instead",
                        )
                        .into());
                };
                let same_shape = match (self.labels(declared), self.labels(def)) {
                    (Some(a), Some(b)) => a.iter().map(|(l, _)| l).eq(b.iter().map(|(l, _)| l)),
                    (None, None) => count == items.len(),
                    _ => false,
                };
                if !same_shape {
                    let name = self
                        .model
                        .defs
                        .decl(def)
                        .map(|d| d.name.clone())
                        .unwrap_or_default();
                    let own = self
                        .model
                        .defs
                        .decl(declared)
                        .map(|d| d.name.clone())
                        .unwrap_or_default();
                    return Err(self.eval_error(
                        env,
                        span,
                        format!(
                            "'{name}[same]' needs the same labels as '{own}', found different ones"
                        ),
                        "the two arrays need the same labels in the same order, or the same count",
                    ).into());
                }
                Ok(items
                    .get(element)
                    .ok_or(Fault::Internal("the same element of another array"))?)
            }
        }
    }

    // -----------------------------------------------------------------------
    // Stocks, profiles and parts
    // -----------------------------------------------------------------------

    pub(crate) fn stock_field(
        &mut self,
        stock: StockId,
        field: StockField,
        span: Span,
    ) -> Solved<Value> {
        if self.poisoned(DefId::Stock(stock)) {
            return Err(self.skip().into());
        }
        let Some(def) = self.model.defs.stock(stock) else {
            return Err(Fault::Internal("a stock's declaration").into());
        };
        // The expression, and the field it is written in (a dowel's
        // thickness and width are its diameter).
        let (expr, written) = match field {
            StockField::Species => return Ok(Value::Word(Word::Species(def.species))),
            StockField::Form => return Ok(Value::Word(Word::StockForm(def.form))),
            StockField::Thickness => match (&def.thickness, def.form) {
                (Some(t), _) => (t, "thickness"),
                (None, StockForm::Dowel) => match &def.diameter {
                    Some(d) => (d, "diameter"),
                    None => return Err(Fault::Earlier.into()),
                },
                (None, _) => return Err(Fault::Earlier.into()),
            },
            StockField::Diameter => (
                def.diameter.as_ref().ok_or_else(|| {
                    self.error(
                        Code::UnknownField,
                        format!("stock '{}' has no field 'diameter'", def.decl.name),
                        def.decl.file,
                        span,
                        "only dowel stock has a diameter",
                    )
                })?,
                "diameter",
            ),
            StockField::Width => match (&def.width, def.form) {
                (Some(w), _) => (w, "width"),
                (None, StockForm::Dowel) => {
                    (def.diameter.as_ref().ok_or(Fault::Earlier)?, "diameter")
                }
                (None, _) => {
                    return Err(self
                        .error(
                            Code::UnknownField,
                            format!("stock '{}' has no field 'width'", def.decl.name),
                            def.decl.file,
                            span,
                            "the stock has no fixed width",
                        )
                        .into());
                }
            },
            StockField::MaxWidth => (
                def.max_width.as_ref().ok_or_else(|| {
                    self.error(
                        Code::UnknownField,
                        format!("stock '{}' has no field 'max_width'", def.decl.name),
                        def.decl.file,
                        span,
                        "the stock sets no max_width",
                    )
                })?,
                "max_width",
            ),
            StockField::MaxLength => (
                def.max_length.as_ref().ok_or_else(|| {
                    self.error(
                        Code::UnknownField,
                        format!("stock '{}' has no field 'max_length'", def.decl.name),
                        def.decl.file,
                        span,
                        "the stock sets no max_length",
                    )
                })?,
                "max_length",
            ),
        };
        let file = def.decl.file;
        let Some(ctx) = self.module_ctx(DefId::Stock(stock)) else {
            return Err(Fault::Internal("the file an item is declared in").into());
        };
        let env = Env {
            ctx,
            file,
            kind: None,
            array: None,
        };
        let value = self.eval(expr, &env)?;
        if let Value::Length(size) = &value
            && *size <= ExactLength::ZERO
        {
            return Err(self.stock_size_not_positive(file, expr.span, written, *size));
        }
        Ok(value)
    }

    /// Every size of a stock is more than zero (grammar, "stock"): a stock
    /// field of 0mm or less is reported once, at the field, however often
    /// it is read.
    fn stock_size_not_positive(
        &mut self,
        file: FileId,
        span: Span,
        field: &str,
        size: ExactLength,
    ) -> crate::Failure {
        let message = format!(
            "'{field}' must be more than 0mm, found {}",
            Value::Length(size)
        );
        let reported = self.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == Code::OutOfRange
                && diagnostic.message == message
                && diagnostic
                    .primary()
                    .is_some_and(|label| label.file == file && label.span == span)
        });
        if reported {
            return self.skip().into();
        }
        self.error(
            Code::OutOfRange,
            message,
            file,
            span,
            format!("write the stock's {field} as you would measure it, more than 0mm"),
        )
        .into()
    }

    pub(crate) fn stock_length(
        &mut self,
        stock: StockId,
        field: StockField,
    ) -> Solved<Option<ExactLength>> {
        let Some(def) = self.model.defs.stock(stock) else {
            return Err(Fault::Internal("a stock's declaration").into());
        };
        let present = match field {
            StockField::Thickness => def.thickness.is_some() || def.form == StockForm::Dowel,
            StockField::Diameter => def.diameter.is_some(),
            StockField::Width => def.width.is_some() || def.form == StockForm::Dowel,
            StockField::MaxWidth => def.max_width.is_some(),
            StockField::MaxLength => def.max_length.is_some(),
            StockField::Species | StockField::Form => false,
        };
        if !present {
            return Ok(None);
        }
        match self.stock_field(stock, field, Span::default())? {
            Value::Length(length) => Ok(Some(length)),
            _ => Err(Fault::Earlier.into()),
        }
    }

    pub(crate) fn profile_length(
        &mut self,
        profile: stave_model::ids::ProfileId,
    ) -> Solved<ExactLength> {
        let stations = self.profile_stations(profile)?;
        Ok(stations.last().map(|(at, _)| *at).ok_or(Fault::Earlier)?)
    }

    /// The stations of a profile as (distance, diameter).
    pub(crate) fn profile_stations(
        &mut self,
        profile: stave_model::ids::ProfileId,
    ) -> Solved<Vec<(ExactLength, ExactLength)>> {
        if self.poisoned(DefId::Profile(profile)) {
            return Err(self.skip().into());
        }
        let Some(def) = self.model.defs.profile(profile) else {
            return Err(Fault::Internal("a profile's declaration").into());
        };
        let Some(ctx) = self.module_ctx(DefId::Profile(profile)) else {
            return Err(Fault::Internal("the file an item is declared in").into());
        };
        let env = Env {
            ctx,
            file: def.decl.file,
            kind: None,
            array: None,
        };
        let mut stations = Vec::new();
        let mut previous: Option<ExactLength> = None;
        for (at, value) in &def.stations {
            let at_value = self.eval_length(at, &env)?;
            let mut size = self.eval_length(value, &env)?;
            if def.measure == stave_schema::words::Measure::Radius {
                size = size
                    .checked_mul(Rational::from(2i64))
                    .ok_or(Fault::Overflow)?;
            }
            if previous.is_some_and(|p| at_value < p) {
                return Err(self
                    .error(
                        Code::TypeMismatch,
                        format!("profile '{}' has stations that go backwards", def.decl.name),
                        def.decl.file,
                        at.span,
                        "stations must not decrease",
                    )
                    .into());
            }
            previous = Some(at_value);
            stations.push((at_value, size));
        }
        Ok(stations)
    }

    // -----------------------------------------------------------------------
    // Built-in functions
    // -----------------------------------------------------------------------

    fn builtin(&mut self, call: &BuiltinCall, env: &Env<'_>, span: Span) -> Solved<Value> {
        let length = |this: &mut Self, expr: &Option<RExpr>| -> Solved<Option<ExactLength>> {
            match expr {
                Some(expr) => this.eval_length(expr, env).map(Some),
                None => Ok(None),
            }
        };
        match call {
            BuiltinCall::Between { a, b } => {
                let a = self.eval(a, env)?;
                let b = self.eval(b, env)?;
                self.between(&a, &b, env, span).map(Value::Length)
            }
            BuiltinCall::Extent { items, axis, of } => {
                let items = self.eval(items, env)?;
                let axis = match (self.eval(axis, env)?, of) {
                    (Value::Word(Word::Axis(axis)), None) => axis,
                    // An axis of another part, or of a contact patch: the
                    // world axis it runs along.
                    (Value::Word(Word::PartAxis(part_axis)), Some(of)) => {
                        match self.eval(of, env)? {
                            Value::Part(of) => {
                                self.part_orientation(of)?.direction(part_axis).axis()
                            }
                            Value::Patch(patch) => patch.axis(part_axis),
                            other => {
                                return Err(self
                                    .eval_error(
                                        env,
                                        of.span,
                                        format!(
                                            "'of' needs a part or a contact patch, found {}",
                                            other.description()
                                        ),
                                        "name one part, or write contact(a, b)",
                                    )
                                    .into());
                            }
                        }
                    }
                    _ => return Err(Fault::Earlier.into()),
                };
                let parts = items.parts().ok_or_else(|| {
                    self.eval_error(
                        env,
                        span,
                        "'extent' needs parts",
                        "list the parts: extent([a, b], up)",
                    )
                })?;
                let mut low: Option<ExactLength> = None;
                let mut high: Option<ExactLength> = None;
                for part in parts {
                    let bounds = self.drawn_box(part)?;
                    low = Some(low.map_or(bounds.min(axis), |l| l.min(bounds.min(axis))));
                    high = Some(high.map_or(bounds.max(axis), |h| h.max(bounds.max(axis))));
                }
                match (low, high) {
                    (Some(low), Some(high)) => {
                        Ok(Value::Length(high.checked_sub(low).ok_or(Fault::Overflow)?))
                    }
                    _ => Ok(Value::Length(ExactLength::ZERO)),
                }
            }
            BuiltinCall::Mid { item, axis } => {
                let item = self.eval(item, env)?;
                let Value::Word(Word::Axis(axis)) = self.eval(axis, env)? else {
                    return Err(Fault::Earlier.into());
                };
                let (bounds, owner) = match item {
                    Value::Part(part) => (self.drawn_box(part)?, AnchorOwner::Part(part)),
                    Value::Region(region) => (
                        self.region_value(region)?.bounds,
                        AnchorOwner::Region(region),
                    ),
                    other => {
                        return Err(self
                            .eval_error(
                                env,
                                span,
                                format!(
                                    "'mid' needs a part or a region, found {}",
                                    other.description()
                                ),
                                "name a part or region",
                            )
                            .into());
                    }
                };
                let position =
                    midpoint(bounds.min(axis), bounds.max(axis)).ok_or(Fault::Overflow)?;
                Ok(Value::Anchor(AnchorValue {
                    axis,
                    position,
                    face: None,
                    owner,
                }))
            }
            BuiltinCall::Opening(faces) => {
                let mut planes = Vec::new();
                for face in OpeningFaces::ORDER {
                    let expr = faces.face(face);
                    let name = face.word();
                    let anchor = self.eval_anchor(expr, env)?;
                    if anchor.axis != face.axis() {
                        return Err(self
                            .eval_error(
                                env,
                                expr.span,
                                format!(
                                    "'{name}' needs a plane across '{}', found one across '{}'",
                                    face.axis().word(),
                                    anchor.axis.word()
                                ),
                                format!(
                                    "name a {} face, or a mid-plane across {}",
                                    face.word(),
                                    face.axis().word()
                                ),
                            )
                            .into());
                    }
                    planes.push(anchor.position);
                }
                let [left, right, bottom, top, front, back] = planes.as_slice() else {
                    return Err(Fault::Internal("the six faces of an opening").into());
                };
                let bounds = ExactBox::new((*left, *right), (*bottom, *top), (*front, *back));
                Ok(Value::Regions(vec![RegionValue {
                    bounds,
                    origin: RegionOrigin::Plain,
                }]))
            }
            BuiltinCall::Contact { a, b } => {
                let a = self.one_part(Builtin::Contact, a, env)?;
                let b = self.one_part(Builtin::Contact, b, env)?;
                self.contact(a, b, env, span).map(Value::Patch)
            }
            BuiltinCall::EdgeToEdge { a, b } => self.eval_meeting(Builtin::EdgeToEdge, a, b, env),
            BuiltinCall::Split { region, axis, by } => self.split(region, axis, by, env, span),
            BuiltinCall::Cover {
                region,
                mount,
                thickness,
            } => self.cover(region, mount, thickness, env),
            BuiltinCall::Overlay {
                left,
                right,
                top,
                bottom,
                meeting,
            } => {
                let zero = ExactLength::ZERO;
                Ok(Value::Mount(Mount::Overlay {
                    left: length(self, left)?.unwrap_or(zero),
                    right: length(self, right)?.unwrap_or(zero),
                    top: length(self, top)?.unwrap_or(zero),
                    bottom: length(self, bottom)?.unwrap_or(zero),
                    meeting: length(self, meeting)?,
                }))
            }
            BuiltinCall::Inset { reveal } => Ok(Value::Mount(Mount::Inset {
                reveal: self.eval_length(reveal, env)?,
            })),
            BuiltinCall::Turned { profile } => match self.eval(profile, env)? {
                Value::Profile(profile) => Ok(Value::Turned(profile)),
                _ => Err(Fault::Earlier.into()),
            },
            BuiltinCall::Joined { by: expr } => {
                let kinds = match self.eval(expr, env)? {
                    Value::Kind(kind) => vec![kind],
                    Value::List(items) => {
                        let mut kinds = Vec::new();
                        for item in items {
                            match item {
                                Value::Kind(kind) => kinds.push(kind),
                                other => {
                                    return Err(self
                                        .error(
                                            Code::TypeMismatch,
                                            format!(
                                                "'by' needs joint kinds, found {}",
                                                other.description()
                                            ),
                                            env.file,
                                            expr.span,
                                            "list joint kinds: joined(by: [kind_name, other_kind])",
                                        )
                                        .into());
                                }
                            }
                        }
                        kinds
                    }
                    other => {
                        return Err(self
                            .error(
                                Code::TypeMismatch,
                                format!("'by' needs a joint kind, found {}", other.description()),
                                env.file,
                                expr.span,
                                "name a joint kind: joined(by: kind_name)",
                            )
                            .into());
                    }
                };
                match stave_model::nonempty::NonEmpty::new(kinds) {
                    Some(by) => Ok(Value::Boards(stave_model::value::Joining { by })),
                    None => Err(self
                        .error(
                            Code::TypeMismatch,
                            "'by' needs at least one joint kind",
                            env.file,
                            expr.span,
                            "name a joint kind: joined(by: kind_name)",
                        )
                        .into()),
                }
            }
            BuiltinCall::Spliced { by, at, from } => {
                let by = match self.eval(by, env)? {
                    Value::Kind(kind) => kind,
                    other => {
                        return Err(self.error(
                            Code::TypeMismatch,
                            format!("'by' needs a joint kind, found {}", other.description()),
                            env.file,
                            by.span,
                            "name the joint kind each splice is made with: spliced(by: kind_name, ...)",
                        ).into());
                    }
                };
                let positions = match self.eval(at, env)? {
                    Value::Length(length) => vec![length],
                    Value::List(items) => {
                        let mut lengths = Vec::new();
                        for item in items {
                            match item {
                                Value::Length(length) => lengths.push(length),
                                other => {
                                    return Err(self
                                        .error(
                                            Code::TypeMismatch,
                                            format!(
                                                "'at' needs lengths, found {}",
                                                other.description()
                                            ),
                                            env.file,
                                            at.span,
                                            "list where each splice is: at: [1200mm, 2400mm]",
                                        )
                                        .into());
                                }
                            }
                        }
                        lengths
                    }
                    other => {
                        return Err(self
                            .error(
                                Code::TypeMismatch,
                                format!("'at' needs a length, found {}", other.description()),
                                env.file,
                                at.span,
                                "say how far from the end the splice is: at: 1200mm",
                            )
                            .into());
                    }
                };
                let Some(positions) = stave_model::nonempty::NonEmpty::new(positions) else {
                    return Err(self
                        .error(
                            Code::TypeMismatch,
                            "'at' needs at least one length",
                            env.file,
                            at.span,
                            "say how far from the end the splice is: at: 1200mm",
                        )
                        .into());
                };
                let from = match self.eval(from, env)? {
                    Value::Word(Word::Face(face)) => face,
                    other => {
                        return Err(self
                            .error(
                                Code::TypeMismatch,
                                format!("'from' needs a face, found {}", other.description()),
                                env.file,
                                from.span,
                                "name the end the splices are measured from: from: left",
                            )
                            .into());
                    }
                };
                Ok(Value::Splices(stave_model::value::Splicing {
                    by,
                    at: positions,
                    from,
                }))
            }
            BuiltinCall::Box {
                length,
                width,
                thickness,
            } => Ok(Value::Envelope(Envelope::Box {
                length: self.eval_length(length, env)?,
                width: self.eval_length(width, env)?,
                thickness: self.eval_length(thickness, env)?,
            })),
            BuiltinCall::RegionBox { across, up, deep } => self.region_box(across, up, deep, env),
            BuiltinCall::Cylinder { diameter, length } => Ok(Value::Envelope(Envelope::Cylinder {
                diameter: self.eval_length(diameter, env)?,
                length: self.eval_length(length, env)?,
            })),
            BuiltinCall::Node { name } => self.eval(name, env),
            BuiltinCall::Model {
                source,
                align,
                sha256,
            } => {
                let source = match self.eval(source, env)? {
                    Value::Text(text) => text,
                    _ => return Err(Fault::Earlier.into()),
                };
                let align = match align.as_ref().map(|e| self.eval(e, env)).transpose()? {
                    Some(Value::Text(node)) => Some(node),
                    _ => None,
                };
                let sha256 = match sha256 {
                    Some(expr) => match self.eval(expr, env)? {
                        Value::Text(text) => match text.parse::<value::Sha256>() {
                            Ok(digest) => Some(digest),
                            Err(why) => {
                                return Err(self.error(
                                    Code::TypeMismatch,
                                    format!("'sha256' needs 64 hexadecimal digits: {why}"),
                                    env.file,
                                    expr.span,
                                    "copy the file's SHA-256 whole (`sha256sum model.glb` prints it), or leave sha256: out",
                                ).into());
                            }
                        },
                        _ => return Err(Fault::Earlier.into()),
                    },
                    None => None,
                };
                Ok(Value::Model(ModelRef {
                    source,
                    align,
                    sha256,
                }))
            }
            BuiltinCall::Ends { inset } => {
                Ok(Value::Spacing(Spacing::Ends(self.eval_length(inset, env)?)))
            }
            BuiltinCall::Pitch { distance } => Ok(Value::Spacing(Spacing::Pitch(
                self.eval_length(distance, env)?,
            ))),
            BuiltinCall::Gap { distance } => Ok(Value::Spacing(Spacing::Gap(
                self.eval_length(distance, env)?,
            ))),
        }
    }

    fn between(&mut self, a: &Value, b: &Value, env: &Env<'_>, span: Span) -> Solved<ExactLength> {
        match (a, b) {
            (Value::Part(a), Value::Part(b)) => {
                let (box_a, box_b) = (self.drawn_box(*a)?, self.drawn_box(*b)?);
                let mut apart = Vec::new();
                for axis in Axis::ALL {
                    if box_a.max(axis) <= box_b.min(axis) {
                        apart.push(
                            box_b
                                .min(axis)
                                .checked_sub(box_a.max(axis))
                                .ok_or(Fault::Overflow)?,
                        );
                    } else if box_b.max(axis) <= box_a.min(axis) {
                        apart.push(
                            box_a
                                .min(axis)
                                .checked_sub(box_b.max(axis))
                                .ok_or(Fault::Overflow)?,
                        );
                    }
                }
                match apart.as_slice() {
                    [distance] => Ok(*distance),
                    [] => {
                        let (name_a, name_b) = (self.part_name(*a), self.part_name(*b));
                        Err(self.eval_error(
                            env,
                            span,
                            format!(
                                "'between' needs parts apart along exactly one axis, but {name_a} and {name_b} touch or overlap"
                            ),
                            "check where the two parts are placed, or measure between two parallel faces: between(a.top, b.bottom)",
                        ).into())
                    }
                    several => {
                        let (name_a, name_b) = (self.part_name(*a), self.part_name(*b));
                        Err(self.eval_error(
                            env,
                            span,
                            format!(
                                "'between' needs parts apart along exactly one axis, but {name_a} and {name_b} are apart along {} axes",
                                several.len()
                            ),
                            "measure between two parallel faces instead: between(a.top, b.bottom)",
                        ).into())
                    }
                }
            }
            (Value::Anchor(anchor), Value::Region(region))
            | (Value::Region(region), Value::Anchor(anchor)) => {
                // From a face to the same side of the region.
                let Some(face) = anchor.face else {
                    return Err(self.eval_error(
                        env,
                        span,
                        "'between' needs a face to measure to a region, found a mid-plane",
                        "name a face, such as side.left: it is measured to the region's same side",
                    ).into());
                };
                let side = self.region_value(*region)?.bounds.face(face);
                let distance = anchor.position.checked_sub(side).ok_or(Fault::Overflow)?;
                Ok(if distance < ExactLength::ZERO {
                    distance.checked_neg().ok_or(Fault::Overflow)?
                } else {
                    distance
                })
            }
            (Value::Anchor(a), Value::Anchor(b)) => {
                if a.axis != b.axis {
                    return Err(self
                        .eval_error(
                            env,
                            span,
                            "'between' needs two parallel planes",
                            "name two faces across the same axis",
                        )
                        .into());
                }
                let distance = a.position.checked_sub(b.position).ok_or(Fault::Overflow)?;
                Ok(if distance < ExactLength::ZERO {
                    distance.checked_neg().ok_or(Fault::Overflow)?
                } else {
                    distance
                })
            }
            _ => Err(self
                .eval_error(
                    env,
                    span,
                    format!(
                        "'between' needs two parts or two faces, found {} and {}",
                        a.description(),
                        b.description()
                    ),
                    "write between(a, b) with two parts",
                )
                .into()),
        }
    }

    fn region_argument(&mut self, expr: &RExpr, env: &Env<'_>) -> Solved<RegionValue> {
        match self.eval(expr, env)? {
            Value::Region(region) => self.region_value(region),
            Value::Regions(list) if list.len() == 1 => Ok(list
                .first()
                .copied()
                .ok_or(Fault::Internal("the region of a list of one"))?),
            other => Err(self
                .eval_error(
                    env,
                    expr.span,
                    format!("this needs a region, found {}", other.description()),
                    "name a region",
                )
                .into()),
        }
    }

    /// `box(across:, up:, deep:)`: the root region, from its low corner at
    /// the origin. Each size is more than zero.
    fn region_box(
        &mut self,
        across: &RExpr,
        up: &RExpr,
        deep: &RExpr,
        env: &Env<'_>,
    ) -> Solved<Value> {
        let mut sizes = [ExactLength::ZERO; 3];
        for ((slot, name), expr) in sizes
            .iter_mut()
            .zip(Builtin::REGION_BOX_ARGUMENTS)
            .zip([across, up, deep])
        {
            let size = self.eval_length(expr, env)?;
            if size <= ExactLength::ZERO {
                return Err(self
                    .error(
                        Code::OutOfRange,
                        format!(
                            "'{name}' must be more than 0mm, found {}",
                            Value::Length(size)
                        ),
                        env.file,
                        expr.span,
                        format!(
                            "make '{name}' more than 0mm: the box needs a size in each direction"
                        ),
                    )
                    .into());
            }
            *slot = size;
        }
        let [across, up, deep] = sizes;
        let zero = ExactLength::ZERO;
        Ok(Value::Regions(vec![RegionValue {
            bounds: ExactBox::new((zero, across), (zero, up), (zero, deep)),
            origin: RegionOrigin::Plain,
        }]))
    }

    fn split(
        &mut self,
        region: &RExpr,
        axis: &RExpr,
        by: &SplitBy,
        env: &Env<'_>,
        span: Span,
    ) -> Solved<Value> {
        let region = self.region_argument(region, env)?;
        let Value::Word(Word::Axis(axis)) = self.eval(axis, env)? else {
            return Err(Fault::Earlier.into());
        };
        let bounds = region.bounds;
        let (low, high) = (bounds.min(axis), bounds.max(axis));
        if let SplitBy::At(at) = by {
            let part = self.eval_part(at, env)?;
            let part_box = self.drawn_box(part)?;
            let (p0, p1) = (part_box.min(axis), part_box.max(axis));
            if p0 < low || p1 > high {
                let name = self.part_name(part);
                return Err(self
                    .eval_error(
                        env,
                        at.span,
                        format!(
                            "'split' needs a part inside the region, but {name} reaches outside it"
                        ),
                        "split at a part that lies within the region",
                    )
                    .into());
            }
            return Ok(Value::Regions(vec![
                RegionValue {
                    bounds: bounds.with(axis, low, p0),
                    origin: RegionOrigin::Plain,
                },
                RegionValue {
                    bounds: bounds.with(axis, p1, high),
                    origin: RegionOrigin::Plain,
                },
            ]));
        }
        let Some((_, count)) = env.array.filter(|(_, n)| *n > 0) else {
            return Err(self
                .eval_error(
                    env,
                    span,
                    "'split' without 'at:' needs an array to fill",
                    "declare the region as an array: region bay[a, b] = split(...)",
                )
                .into());
        };
        let size = high.checked_sub(low).ok_or(Fault::Overflow)?;
        let shares = match by {
            SplitBy::Shares(expr) => self.shares(expr, env, count)?,
            SplitBy::Equal | SplitBy::At(_) => vec![Rational::from(1_i64); count],
        };
        let total = shares
            .iter()
            .try_fold(Rational::from(0_i64), |sum, share| sum.checked_add(*share))
            .ok_or(Fault::Overflow)?;
        let mut pieces = Vec::new();
        let mut start = low;
        let mut before = Rational::from(0_i64);
        for (index, share) in shares.iter().enumerate() {
            before = before.checked_add(*share).ok_or(Fault::Overflow)?;
            // Each end is worked out from the whole, so the last piece ends
            // exactly at the region's end.
            let end = if index + 1 == count {
                high
            } else {
                let fraction = before.checked_div(total).ok_or(Fault::Overflow)?;
                low.checked_add(size.checked_mul(fraction).ok_or(Fault::Overflow)?)
                    .ok_or(Fault::Overflow)?
            };
            pieces.push(RegionValue {
                bounds: bounds.with(axis, start, end),
                origin: RegionOrigin::Split { axis, index, count },
            });
            start = end;
        }
        Ok(Value::Regions(pieces))
    }

    /// `shares: [n, ...]` of a split into `count` pieces: one share for
    /// each piece, each more than zero, for an array whose count can't
    /// change (grammar, "Regions").
    fn shares(&mut self, expr: &RExpr, env: &Env<'_>, count: usize) -> Solved<Vec<Rational>> {
        if let Some((def, _)) = env.array
            && let Some(ArrayDef::Count(counted)) = self.model.defs.array(def)
            && self.reads_ranged_param(counted)
        {
            return Err(self.error(
                Code::TypeMismatch,
                "'shares' needs a fixed number of pieces, found a count that can change",
                env.file,
                expr.span,
                "give the array a fixed count or labels, or leave out shares: to split into equal pieces",
            ).into());
        }
        let items = match self.eval(expr, env)? {
            Value::List(items) => items,
            _ => return Err(Fault::Earlier.into()),
        };
        if items.len() != count {
            let name = env
                .array
                .and_then(|(def, _)| self.model.defs.decl(def))
                .map(|decl| decl.name.clone())
                .unwrap_or_default();
            return Err(self
                .error(
                    Code::TypeMismatch,
                    format!(
                        "'shares' needs {count} numbers, one for each piece of {name}, found {}",
                        items.len()
                    ),
                    env.file,
                    expr.span,
                    format!("give {count} shares, one for each piece in label order"),
                )
                .into());
        }
        let mut shares = Vec::with_capacity(count);
        for (index, item) in items.into_iter().enumerate() {
            // The share as written, when the list is written out.
            let span = match &expr.kind {
                RExprKind::List(written) => written.get(index).map_or(expr.span, |e| e.span),
                _ => expr.span,
            };
            let share = match item {
                Value::Count(n) | Value::Ratio(n) => n,
                _ => return Err(Fault::Earlier.into()),
            };
            if share <= Rational::from(0_i64) {
                return Err(self
                    .error(
                        Code::OutOfRange,
                        format!(
                            "'shares' must each be more than 0, found {}",
                            Value::Count(share)
                        ),
                        env.file,
                        span,
                        "every piece needs some room: give each share a number more than 0",
                    )
                    .into());
            }
            shares.push(share);
        }
        Ok(shares)
    }

    /// `true` when an array's count reads a param with a `within` domain,
    /// so the count can change from one checked value to another.
    fn reads_ranged_param(&self, expr: &RExpr) -> bool {
        match &expr.kind {
            RExprKind::Symbol(Symbol::Def(DefId::Param(param))) => self
                .model
                .defs
                .param(*param)
                .is_some_and(|def| def.domain.is_some()),
            RExprKind::Unary { operand, .. } => self.reads_ranged_param(operand),
            RExprKind::Binary { lhs, rhs, .. } => {
                self.reads_ranged_param(lhs) || self.reads_ranged_param(rhs)
            }
            _ => false,
        }
    }

    fn cover(
        &mut self,
        region: &RExpr,
        mount: &RExpr,
        thickness: &RExpr,
        env: &Env<'_>,
    ) -> Solved<Value> {
        let region = self.region_argument(region, env)?;
        let Value::Mount(mount) = self.eval(mount, env)? else {
            return Err(Fault::Earlier.into());
        };
        let thickness = self.eval_length(thickness, env)?;
        let b = region.bounds;
        let add = |a: ExactLength, d: ExactLength| a.checked_add(d).ok_or(Fault::Overflow);
        let sub = |a: ExactLength, d: ExactLength| a.checked_sub(d).ok_or(Fault::Overflow);
        let bounds = match mount {
            Mount::Overlay {
                left,
                right,
                top,
                bottom,
                meeting,
            } => {
                let shared = |axis: Axis, low_side: bool| match region.origin {
                    RegionOrigin::Split {
                        axis: split_axis,
                        index,
                        count,
                    } if split_axis == axis => {
                        if low_side {
                            index > 0
                        } else {
                            index + 1 < count
                        }
                    }
                    RegionOrigin::Split { .. } | RegionOrigin::Plain => false,
                };
                let pick = |value: ExactLength, axis: Axis, low_side: bool| match meeting {
                    Some(m) if shared(axis, low_side) => m,
                    _ => value,
                };
                let front = b.min(Axis::Deep);
                ExactBox::new(
                    (
                        sub(b.min(Axis::Across), pick(left, Axis::Across, true))?,
                        add(b.max(Axis::Across), pick(right, Axis::Across, false))?,
                    ),
                    (
                        sub(b.min(Axis::Up), pick(bottom, Axis::Up, true))?,
                        add(b.max(Axis::Up), pick(top, Axis::Up, false))?,
                    ),
                    (sub(front, thickness)?, front),
                )
            }
            Mount::Inset { reveal } => {
                let front = b.min(Axis::Deep);
                ExactBox::new(
                    (
                        add(b.min(Axis::Across), reveal)?,
                        sub(b.max(Axis::Across), reveal)?,
                    ),
                    (add(b.min(Axis::Up), reveal)?, sub(b.max(Axis::Up), reveal)?),
                    (front, add(front, thickness)?),
                )
            }
        };
        Ok(Value::Regions(vec![RegionValue {
            bounds,
            origin: RegionOrigin::Plain,
        }]))
    }

    // -----------------------------------------------------------------------
    // Text
    // -----------------------------------------------------------------------

    /// Text with values filled in; lengths in millimetres.
    pub(crate) fn format_text(&mut self, text: &Text, env: &Env<'_>) -> Solved<String> {
        let mut out = String::new();
        for part in &text.parts {
            match part {
                TextPart::Literal(literal) => out.push_str(literal),
                TextPart::Value(expr) => {
                    let value = self.eval(expr, env)?;
                    out.push_str(&self.describe(&value));
                }
            }
        }
        Ok(out)
    }

    /// A value as written in a message.
    pub(crate) fn describe(&self, value: &Value) -> String {
        match value {
            Value::Part(part) => self.part_name(*part),
            Value::Parts(parts) => parts
                .iter()
                .map(|p| self.part_name(*p))
                .collect::<Vec<_>>()
                .join(", "),
            Value::Region(region) => self
                .model
                .regions
                .get(region.index())
                .map(|r| r.name.clone())
                .unwrap_or_default(),
            Value::Stock(stock) => self
                .model
                .defs
                .stock(*stock)
                .map(|s| s.decl.name.clone())
                .unwrap_or_default(),
            Value::Kind(kind) => self
                .model
                .defs
                .kind(kind.kind)
                .map(|k| k.decl.name.clone())
                .unwrap_or_default(),
            other => other.to_string(),
        }
    }
}

/// The midpoint of two values; `None` on overflow.
pub(crate) fn midpoint(a: ExactLength, b: ExactLength) -> Option<ExactLength> {
    a.checked_add(b)?.checked_div(Rational::from(2i64))
}

/// The recipes a `methods:` value names: one recipe or a list of them.
pub(crate) fn recipes(value: &Value) -> Vec<stave_model::ids::RecipeId> {
    match value {
        Value::Recipe(recipe) => vec![*recipe],
        Value::List(items) => items
            .iter()
            .filter_map(|item| match item {
                Value::Recipe(recipe) => Some(*recipe),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}
