//! Evaluating resolved expressions to exact values, on demand.

use std::cell::RefCell;

use stave_lang::ast::Span;
use stave_lang::placement::{Axis, Face};
use stave_lang::units::{ExactLength, Length, Rational};
use stave_lang::{Code, FileId};
use stave_model::defs::{
    ArrayDef, DefId, Index, Member, RExpr, RExprKind, StockField, Symbol, Text, TextPart, Word,
};
use stave_model::ids::{InstanceId, KindId, PartId, StockId};
use stave_model::instance::{Ctx, ItemSlot, ParamSource, Slot};
use stave_model::value::{
    self, AnchorOwner, AnchorValue, Envelope, ExactBox, KindValue, ModelAlign, ModelRef, Mount,
    RegionOrigin, RegionValue, Spacing, Value,
};
use stave_schema::functions::Builtin;
use stave_schema::kinds::DEFAULT_SCREW_DIAMETER_NM;
use stave_schema::words::StockForm;

use crate::{Frame, Memo, Reported, Solved, Solver};

/// The roles and settings of one application of a kind.
pub(crate) struct KindEnv {
    pub kind: KindId,
    /// Per role: a part or a region; empty for a kind value.
    pub roles: Vec<Value>,
    /// Settings from the kind value and the call.
    pub settings: Vec<(String, Value)>,
    /// Param values, worked out on demand.
    pub params: RefCell<Vec<Option<Value>>>,
    /// Parts the kind made for this pair.
    pub made: Vec<PartId>,
}

impl KindEnv {
    pub fn new(
        kind: KindId,
        roles: Vec<Value>,
        settings: Vec<(String, Value)>,
        param_count: usize,
    ) -> Self {
        KindEnv {
            kind,
            roles,
            settings,
            params: RefCell::new(vec![None; param_count]),
            made: Vec::new(),
        }
    }
}

/// Where an expression is evaluated.
#[derive(Clone, Copy)]
pub(crate) struct Env<'e> {
    pub ctx: Ctx,
    pub file: FileId,
    pub kind: Option<&'e KindEnv>,
    /// The array being declared and its element count, for `[same]` and
    /// `split(region, axis)`.
    pub array: Option<(DefId, usize)>,
}

/// An array's elements, from an indexed name.
enum ArrayItems {
    Parts(Vec<PartId>),
    Regions(Vec<stave_model::ids::RegionId>),
    Assemblies(Vec<stave_model::ids::AssemblyId>),
    Joints(Vec<stave_model::ids::JointId>),
    Hardware(Vec<stave_model::ids::HardwareId>),
}

impl ArrayItems {
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

    fn all(&self) -> Value {
        match self {
            ArrayItems::Parts(v) => Value::Parts(v.clone()),
            ArrayItems::Regions(v) => Value::List(v.iter().map(|r| Value::Region(*r)).collect()),
            ArrayItems::Assemblies(v) => Value::Assemblies(v.clone()),
            ArrayItems::Joints(v) => Value::List(v.iter().map(|j| Value::Joint(*j)).collect()),
            ArrayItems::Hardware(v) => Value::List(v.iter().map(|h| Value::Hardware(*h)).collect()),
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

pub(crate) fn mm(nanometres: i64) -> ExactLength {
    Length::from_nm(nanometres).to_exact()
}

impl Solver<'_> {
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
            RExprKind::Fit => Err(self.eval_error(
                env,
                span,
                "'fit' is only a part's length or width",
                "write a length",
            )),
            RExprKind::Word(word) => Ok(Value::Word(*word)),
            RExprKind::Symbol(symbol) => self.symbol(*symbol, env, span),
            RExprKind::SelfElement(def) => {
                let slot = self.model.item(*def, env.ctx).cloned();
                slot.and_then(|slot| slot_value(&slot, env.ctx.element))
                    .ok_or(Reported)
            }
            RExprKind::Member { base, member } => {
                let base_value = self.eval(base, env)?;
                self.member(&base_value, member, env, span)
            }
            RExprKind::Index { base, index } => self.index(base, index, env, span),
            RExprKind::Builtin { function, args } => self.builtin(*function, args, env, span),
            RExprKind::KindValue { kind, settings } => {
                let base = self.eval(kind, env)?;
                let Value::Kind(mut value) = base else {
                    return Err(self.eval_error(
                        env,
                        kind.span,
                        format!("'{}' is not a kind", base.description()),
                        "name a kind",
                    ));
                };
                for setting in settings {
                    if value.setting(&setting.name).is_some() {
                        return Err(self.error(
                            Code::DuplicateName,
                            format!("'{}' is declared twice", setting.name),
                            env.file,
                            setting.name_span,
                            "the kind value already gives this setting",
                        ));
                    }
                    let v = self.eval(&setting.value, env)?;
                    value.settings.push((setting.name.clone(), v));
                }
                Ok(Value::Kind(value))
            }
            RExprKind::Unary { op, operand } => {
                let operand = self.eval(operand, env)?;
                value::unary(*op, &operand).map_err(|e| self.arithmetic(e, env, span))
            }
            RExprKind::Binary { op, lhs, rhs } => {
                let lhs = self.eval(lhs, env)?;
                let rhs = self.eval(rhs, env)?;
                value::binary(*op, &lhs, &rhs).map_err(|e| self.arithmetic(e, env, span))
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
            RExprKind::Error => Err(Reported),
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
            other => Err(self.error(
                Code::UnitMismatch,
                format!("this needs a length, found {}", other.description()),
                env.file,
                expr.span,
                "write a length with a unit, such as 19mm",
            )),
        }
    }

    pub(crate) fn eval_angle(
        &mut self,
        expr: &RExpr,
        env: &Env<'_>,
    ) -> Solved<stave_lang::units::ExactAngle> {
        match self.eval(expr, env)? {
            Value::Angle(angle) => Ok(angle),
            other => Err(self.error(
                Code::UnitMismatch,
                format!("this needs an angle, found {}", other.description()),
                env.file,
                expr.span,
                "write an angle, such as 45deg",
            )),
        }
    }

    pub(crate) fn eval_bool(&mut self, expr: &RExpr, env: &Env<'_>) -> Solved<bool> {
        match self.eval(expr, env)? {
            Value::Bool(b) => Ok(b),
            other => Err(self.eval_error(
                env,
                expr.span,
                format!("this needs true or false, found {}", other.description()),
                "write a comparison",
            )),
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
                })
            }
            _ => Err(self.eval_error(
                env,
                expr.span,
                format!("this needs a whole number, found {value}"),
                "write a count such as 4",
            )),
        }
    }

    pub(crate) fn eval_part(&mut self, expr: &RExpr, env: &Env<'_>) -> Solved<PartId> {
        match self.eval(expr, env)? {
            Value::Part(part) => Ok(part),
            other => Err(self.eval_error(
                env,
                expr.span,
                format!("this needs a part, found {}", other.description()),
                "name a part",
            )),
        }
    }

    pub(crate) fn eval_anchor(&mut self, expr: &RExpr, env: &Env<'_>) -> Solved<AnchorValue> {
        match self.eval(expr, env)? {
            Value::Anchor(anchor) => Ok(anchor),
            other => Err(self.eval_error(
                env,
                expr.span,
                format!(
                    "this needs a face or mid-plane, found {}",
                    other.description()
                ),
                "name a face, such as base.top",
            )),
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
                .and_then(|k| k.roles.get(index).cloned())
                .ok_or_else(|| self.eval_error(env, span, "a role has no part here", "roles are only bound where the kind is used in a joint or hardware declaration")),
            Symbol::KindParam(_, index) => match env.kind {
                Some(kind_env) => self.kind_param(kind_env, index, env.file, span),
                None => Err(self.eval_error(env, span, "a kind's param has no value here", "use the kind in a joint or hardware declaration")),
            },
            Symbol::MadePart(_, index) => env
                .kind
                .and_then(|k| k.made.get(index).copied())
                .map(Value::Part)
                .ok_or_else(|| self.eval_error(env, span, "the made part doesn't exist yet here", "refer to made parts only in cuts and assembly steps")),
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
                    .is_some_and(|k| k.base == stave_lang::ast::BaseKind::Tool);
                return Ok(if is_tool {
                    Value::Tool(id)
                } else {
                    Value::Kind(KindValue {
                        kind: id,
                        name: self.model.defs.kind_display(id, Some(env.file)),
                        settings: Vec::new(),
                    })
                });
            }
            DefId::Component(_) | DefId::Scene(_) => {
                return Err(self.eval_error(
                    env,
                    span,
                    "this is not a value",
                    "name a part, region or value",
                ));
            }
            _ => {}
        }
        let Some(slot) = self.model.item(def, env.ctx).cloned() else {
            return Err(self.eval_error(
                env,
                span,
                "this item doesn't exist here",
                "items of a project can't be used from a library",
            ));
        };
        match slot {
            ItemSlot::Param(param) => self.param_value(param),
            ItemSlot::Let(binding) => self.let_value(binding),
            ItemSlot::Require(require) => {
                let Some(inst) = self.model.requires.get(require.index()).cloned() else {
                    return Err(Reported);
                };
                let Some(def) = self.model.defs.require(inst.def) else {
                    return Err(Reported);
                };
                let env = Env {
                    ctx: inst.ctx,
                    file: def.decl.file,
                    kind: None,
                    array: None,
                };
                Ok(Value::Bool(self.eval_bool(&def.condition, &env)?))
            }
            other => slot_value(&other, None).ok_or_else(|| {
                self.eval_error(
                    env,
                    span,
                    "this is an array",
                    "index it: name[label], or name[*] for all of them",
                )
            }),
        }
    }

    pub(crate) fn param_value(&mut self, param: stave_model::ids::ParamId) -> Solved<Value> {
        match self.params.get(param.index()) {
            Some(Memo::Done(value)) => return Ok(value.clone()),
            Some(Memo::Failed) => return Err(Reported),
            Some(Memo::Busy) => return Err(self.cycle(Frame::Param(param))),
            Some(Memo::Todo) => {}
            None => return Err(Reported),
        }
        let Some(inst) = self.model.params.get(param.index()).cloned() else {
            return Err(Reported);
        };
        let Some(def) = self.model.defs.param(inst.def) else {
            return Err(Reported);
        };
        if self.poisoned(DefId::Param(inst.def)) {
            return Err(self.skip());
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
                    if let (Ok(value), stave_model::defs::Scope::Module(_)) =
                        (&value, def.decl.scope)
                    {
                        let domain_ctx = env.ctx;
                        self.check_domain(
                            def,
                            value,
                            def.decl.file,
                            default.span,
                            &inst.name,
                            domain_ctx,
                        )?;
                    }
                    value
                }
                None => Err(Reported),
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
                            args.get(index).map(|(_, e)| e.clone())
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
                        let value = self.eval(&arg, &env);
                        if let Ok(value) = &value {
                            let domain_ctx = Ctx {
                                instance: inst.instance,
                                element: None,
                            };
                            self.check_domain(def, value, file, arg.span, &inst.name, domain_ctx)?;
                        }
                        value
                    }
                    None => Err(Reported),
                }
            }
        };
        self.leave();
        if let Some(slot) = self.params.get_mut(param.index()) {
            *slot = match &result {
                Ok(value) => Memo::Done(value.clone()),
                Err(_) => Memo::Failed,
            };
        }
        result
    }

    /// `out-of-range` for a value outside its param's `within`. The domain
    /// is evaluated in `domain_ctx`, where the param is declared.
    pub(crate) fn check_domain(
        &mut self,
        def: &stave_model::defs::ParamDef,
        value: &Value,
        file: FileId,
        span: Span,
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
        let (inside, text) = match domain {
            stave_model::defs::Domain::Range { low, high, .. } => {
                let low = self.eval(low, &module_env)?;
                let high = self.eval(high, &module_env)?;
                let above = value::binary(stave_lang::ast::BinaryOp::Ge, value, &low).ok()
                    == Some(Value::Bool(true));
                let below = value::binary(stave_lang::ast::BinaryOp::Le, value, &high).ok()
                    == Some(Value::Bool(true));
                (above && below, format!("{low}..{high}"))
            }
            stave_model::defs::Domain::OneOf { values, .. } => {
                let mut list = Vec::new();
                let mut inside = false;
                for v in values {
                    let v = self.eval(v, &module_env)?;
                    inside |= &v == value;
                    list.push(v.to_string());
                }
                (inside, format!("[{}]", list.join(", ")))
            }
        };
        if inside {
            return Ok(());
        }
        let short = name.rsplit('.').next().unwrap_or(name);
        Err(self.error(
            Code::OutOfRange,
            format!("'{short}' must be within {text}, found {value}"),
            file,
            span,
            format!("give a value within {text}"),
        ))
    }

    pub(crate) fn let_value(&mut self, binding: stave_model::ids::LetId) -> Solved<Value> {
        match self.lets.get(binding.index()) {
            Some(Memo::Done(value)) => return Ok(value.clone()),
            Some(Memo::Failed) => return Err(Reported),
            Some(Memo::Busy) => return Err(self.cycle(Frame::Let(binding))),
            Some(Memo::Todo) => {}
            None => return Err(Reported),
        }
        let Some(inst) = self.model.lets.get(binding.index()).cloned() else {
            return Err(Reported);
        };
        let Some(def) = self.model.defs.let_def(inst.def) else {
            return Err(Reported);
        };
        if self.poisoned(DefId::Let(inst.def)) {
            return Err(self.skip());
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
        if let Some(slot) = self.lets.get_mut(binding.index()) {
            *slot = match &result {
                Ok(value) => Memo::Done(value.clone()),
                Err(_) => Memo::Failed,
            };
        }
        result
    }

    /// A param of a kind application: the setting, or its default for
    /// this pair.
    pub(crate) fn kind_param(
        &mut self,
        kind_env: &KindEnv,
        index: usize,
        file: FileId,
        span: Span,
    ) -> Solved<Value> {
        if self.poisoned(DefId::Kind(kind_env.kind)) {
            return Err(self.skip());
        }
        if let Some(Some(value)) = kind_env.params.borrow().get(index) {
            return Ok(value.clone());
        }
        let Some(kind) = self.model.defs.kind(kind_env.kind) else {
            return Err(Reported);
        };
        let Some(param) = kind.params.get(index) else {
            return Err(Reported);
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
                        return Err(Reported);
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
                    return Err(self.error(
                        Code::MissingField,
                        format!(
                            "kind '{}' is missing '{}'",
                            self.model.defs.kind_display(kind_env.kind, Some(file)),
                            param.decl.name
                        ),
                        file,
                        span,
                        format!("give it a value: {}: ...", param.decl.name),
                    ));
                }
            },
        };
        if let Some(slot) = kind_env.params.borrow_mut().get_mut(index) {
            *slot = Some(value.clone());
        }
        Ok(value)
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
            return Err(self.skip());
        }
        if let Some(value) = kind_value.setting(name) {
            return Ok(value.clone());
        }
        let Some(kind) = self.model.defs.kind(kind_value.kind) else {
            return Err(Reported);
        };
        let kind_env = KindEnv::new(
            kind_value.kind,
            Vec::new(),
            kind_value.settings.clone(),
            kind.params.len(),
        );
        if let Some(index) = kind.param(name) {
            return self.kind_param(&kind_env, index, file, span);
        }
        if let Some(field) = kind.field(name) {
            let value_expr = field.value.clone();
            let Some(ctx) = self.module_ctx(DefId::Kind(kind_value.kind)) else {
                return Err(Reported);
            };
            let env = Env {
                ctx,
                file: kind.decl.file,
                kind: Some(&kind_env),
                array: None,
            };
            return self.eval(&value_expr, &env);
        }
        if name == "screw_diameter" && kind.base == stave_lang::ast::BaseKind::Pull {
            return Ok(Value::Length(mm(DEFAULT_SCREW_DIAMETER_NM)));
        }
        let kind_name = self.model.defs.kind_display(kind_value.kind, Some(file));
        Err(self.error(
            Code::UnknownField,
            format!("the kind '{kind_name}' has no field '{name}'"),
            file,
            span,
            stave_model::suggest::did_you_mean(
                name,
                kind.params
                    .iter()
                    .map(|p| p.decl.name.as_str())
                    .chain(kind.fields.iter().map(|f| f.name.as_str())),
            )
            .unwrap_or_else(|| "a kind value can be read for its params and fields".to_owned()),
        ))
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
                bounds.size(*axis).map(Value::Length).ok_or(Reported)
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
                    return Err(Reported);
                };
                let ctx = Ctx {
                    instance: inner,
                    element: None,
                };
                let inner_env = Env { ctx, ..*env };
                self.def_value(*def, &inner_env, span)
            }
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
                    _ => None,
                };
                match typed {
                    Some(member) => self.member(value, &member, env, span),
                    None => Err(self.error(
                        Code::UnknownField,
                        format!("{} has no field '{name}'", value.description()),
                        env.file,
                        span,
                        "check the name of the field",
                    )),
                }
            }
            (value, _) => Err(self.eval_error(
                env,
                span,
                format!("{} has no such field", value.description()),
                "check what this refers to",
            )),
        }
    }

    fn array_items(&mut self, base: &RExpr, env: &Env<'_>) -> Solved<(ArrayItems, DefId)> {
        let (def, ctx) = match &base.kind {
            RExprKind::Symbol(Symbol::Def(def)) => (*def, env.ctx),
            RExprKind::Member {
                base: inner,
                member: Member::AssemblyItem(def),
            } => {
                let Value::Assembly(assembly) = self.eval(inner, env)? else {
                    return Err(Reported);
                };
                let Some(instance) = self.model.assemblies.get(assembly.index()).map(|a| a.inner)
                else {
                    return Err(Reported);
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
                return Err(self.eval_error(
                    env,
                    base.span,
                    "this can't be indexed",
                    "index an array declared with [labels] or [count]",
                ));
            }
        };
        let Some(slot) = self.model.item(def, ctx).cloned() else {
            return Err(Reported);
        };
        let items = match slot {
            ItemSlot::Part(slot) => ArrayItems::Parts(slot.all()),
            ItemSlot::Region(slot) => ArrayItems::Regions(slot.all()),
            ItemSlot::Assembly(slot) => ArrayItems::Assemblies(slot.all()),
            ItemSlot::Joint(slot) => ArrayItems::Joints(slot.all()),
            ItemSlot::Hardware(slot) => ArrayItems::Hardware(slot.all()),
            ItemSlot::Param(_) | ItemSlot::Let(_) | ItemSlot::Require(_) => return Err(Reported),
        };
        Ok((items, def))
    }

    pub(crate) fn array_len(&self, def: DefId, ctx: Ctx) -> usize {
        match self.model.item(def, ctx) {
            Some(ItemSlot::Part(slot)) => slot.all().len(),
            Some(ItemSlot::Region(slot)) => slot.all().len(),
            Some(ItemSlot::Assembly(slot)) => slot.all().len(),
            Some(ItemSlot::Joint(slot)) => slot.all().len(),
            Some(ItemSlot::Hardware(slot)) => slot.all().len(),
            _ => match self.model.defs.array(def) {
                Some(ArrayDef::Labels(labels)) => labels.len(),
                _ => 0,
            },
        }
    }

    fn labels(&self, def: DefId) -> Option<Vec<String>> {
        match self.model.defs.array(def)? {
            ArrayDef::Labels(labels) => Some(labels.iter().map(|(l, _)| l.clone()).collect()),
            ArrayDef::Count(_) => None,
        }
    }

    fn index(&mut self, base: &RExpr, index: &Index, env: &Env<'_>, span: Span) -> Solved<Value> {
        let (items, def) = self.array_items(base, env)?;
        match index {
            Index::All => Ok(items.all()),
            Index::Label(position) => items.get(*position).ok_or(Reported),
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
                })
            }
            Index::Same => {
                let (Some((declared, count)), Some(element)) = (env.array, env.ctx.element) else {
                    return Err(self.eval_error(
                        env,
                        span,
                        "'[same]' is only allowed inside the declaration of an array",
                        "name the element instead",
                    ));
                };
                let same_shape = match (self.labels(declared), self.labels(def)) {
                    (Some(a), Some(b)) => a == b,
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
                    ));
                }
                items.get(element).ok_or(Reported)
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
            return Err(self.skip());
        }
        let Some(def) = self.model.defs.stock(stock) else {
            return Err(Reported);
        };
        let expr = match field {
            StockField::Species => return Ok(Value::Word(Word::Species(def.species))),
            StockField::Form => return Ok(Value::Word(Word::StockForm(def.form))),
            StockField::Thickness => match (&def.thickness, def.form) {
                (Some(t), _) => t.clone(),
                (None, StockForm::Dowel) => match &def.diameter {
                    Some(d) => d.clone(),
                    None => return Err(Reported),
                },
                (None, _) => return Err(Reported),
            },
            StockField::Diameter => def.diameter.clone().ok_or_else(|| {
                self.error(
                    Code::UnknownField,
                    format!("stock '{}' has no field 'diameter'", def.decl.name),
                    def.decl.file,
                    span,
                    "only dowel stock has a diameter",
                )
            })?,
            StockField::Width => match (&def.width, def.form) {
                (Some(w), _) => w.clone(),
                (None, StockForm::Dowel) => def.diameter.clone().ok_or(Reported)?,
                (None, _) => {
                    return Err(self.error(
                        Code::UnknownField,
                        format!("stock '{}' has no field 'width'", def.decl.name),
                        def.decl.file,
                        span,
                        "the stock has no fixed width",
                    ));
                }
            },
            StockField::MaxWidth => def.max_width.clone().ok_or_else(|| {
                self.error(
                    Code::UnknownField,
                    format!("stock '{}' has no field 'max_width'", def.decl.name),
                    def.decl.file,
                    span,
                    "the stock sets no max_width",
                )
            })?,
            StockField::MaxLength => def.max_length.clone().ok_or_else(|| {
                self.error(
                    Code::UnknownField,
                    format!("stock '{}' has no field 'max_length'", def.decl.name),
                    def.decl.file,
                    span,
                    "the stock sets no max_length",
                )
            })?,
        };
        let file = def.decl.file;
        let Some(ctx) = self.module_ctx(DefId::Stock(stock)) else {
            return Err(Reported);
        };
        let env = Env {
            ctx,
            file,
            kind: None,
            array: None,
        };
        self.eval(&expr, &env)
    }

    pub(crate) fn stock_length(
        &mut self,
        stock: StockId,
        field: StockField,
    ) -> Solved<Option<ExactLength>> {
        let Some(def) = self.model.defs.stock(stock) else {
            return Err(Reported);
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
            _ => Err(Reported),
        }
    }

    pub(crate) fn profile_length(
        &mut self,
        profile: stave_model::ids::ProfileId,
    ) -> Solved<ExactLength> {
        let stations = self.profile_stations(profile)?;
        stations.last().map(|(at, _)| *at).ok_or(Reported)
    }

    /// The stations of a profile as (distance, diameter).
    pub(crate) fn profile_stations(
        &mut self,
        profile: stave_model::ids::ProfileId,
    ) -> Solved<Vec<(ExactLength, ExactLength)>> {
        if self.poisoned(DefId::Profile(profile)) {
            return Err(self.skip());
        }
        let Some(def) = self.model.defs.profile(profile).cloned() else {
            return Err(Reported);
        };
        let Some(ctx) = self.module_ctx(DefId::Profile(profile)) else {
            return Err(Reported);
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
                size = size.checked_mul(Rational::from(2i64)).ok_or(Reported)?;
            }
            if previous.is_some_and(|p| at_value < p) {
                return Err(self.error(
                    Code::TypeMismatch,
                    format!("profile '{}' has stations that go backwards", def.decl.name),
                    def.decl.file,
                    at.span,
                    "stations must not decrease",
                ));
            }
            previous = Some(at_value);
            stations.push((at_value, size));
        }
        Ok(stations)
    }

    // -----------------------------------------------------------------------
    // Built-in functions
    // -----------------------------------------------------------------------

    fn arg<'a>(args: &'a [stave_model::defs::Setting], name: &str) -> Option<&'a RExpr> {
        args.iter().find(|a| a.name == name).map(|a| &a.value)
    }

    fn builtin(
        &mut self,
        function: Builtin,
        args: &[stave_model::defs::Setting],
        env: &Env<'_>,
        span: Span,
    ) -> Solved<Value> {
        let length = |this: &mut Self, name: &str| -> Solved<Option<ExactLength>> {
            match Self::arg(args, name) {
                Some(expr) => this.eval_length(expr, env).map(Some),
                None => Ok(None),
            }
        };
        match function {
            Builtin::Between => {
                let (Some(a), Some(b)) = (Self::arg(args, "a"), Self::arg(args, "b")) else {
                    return Err(Reported);
                };
                let a = self.eval(a, env)?;
                let b = self.eval(b, env)?;
                self.between(&a, &b, env, span).map(Value::Length)
            }
            Builtin::Extent => {
                let (Some(items), Some(axis)) = (Self::arg(args, "items"), Self::arg(args, "axis"))
                else {
                    return Err(Reported);
                };
                let items = self.eval(items, env)?;
                let Value::Word(Word::Axis(axis)) = self.eval(axis, env)? else {
                    return Err(Reported);
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
                        high.checked_sub(low).map(Value::Length).ok_or(Reported)
                    }
                    _ => Ok(Value::Length(ExactLength::ZERO)),
                }
            }
            Builtin::Mid => {
                let (Some(item), Some(axis)) = (Self::arg(args, "item"), Self::arg(args, "axis"))
                else {
                    return Err(Reported);
                };
                let item = self.eval(item, env)?;
                let Value::Word(Word::Axis(axis)) = self.eval(axis, env)? else {
                    return Err(Reported);
                };
                let (bounds, owner) = match item {
                    Value::Part(part) => (self.drawn_box(part)?, AnchorOwner::Part(part)),
                    Value::Region(region) => (
                        self.region_value(region)?.bounds,
                        AnchorOwner::Region(region),
                    ),
                    other => {
                        return Err(self.eval_error(
                            env,
                            span,
                            format!(
                                "'mid' needs a part or a region, found {}",
                                other.description()
                            ),
                            "name a part or region",
                        ));
                    }
                };
                let position = midpoint(bounds.min(axis), bounds.max(axis)).ok_or(Reported)?;
                Ok(Value::Anchor(AnchorValue {
                    axis,
                    position,
                    face: None,
                    owner,
                }))
            }
            Builtin::Opening => {
                let mut planes = Vec::new();
                for (name, face) in [
                    ("left", Face::Left),
                    ("right", Face::Right),
                    ("bottom", Face::Bottom),
                    ("top", Face::Top),
                    ("front", Face::Front),
                    ("back", Face::Back),
                ] {
                    let Some(expr) = Self::arg(args, name) else {
                        return Err(Reported);
                    };
                    let anchor = self.eval_anchor(expr, env)?;
                    if anchor.axis != face.axis() {
                        return Err(self.eval_error(
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
                        ));
                    }
                    planes.push(anchor.position);
                }
                let [left, right, bottom, top, front, back] = planes.as_slice() else {
                    return Err(Reported);
                };
                let bounds = ExactBox::new((*left, *right), (*bottom, *top), (*front, *back));
                Ok(Value::Regions(vec![RegionValue {
                    bounds,
                    origin: RegionOrigin::Plain,
                }]))
            }
            Builtin::Split => self.split(args, env, span),
            Builtin::Cover => self.cover(args, env, span),
            Builtin::Overlay => {
                let zero = ExactLength::ZERO;
                Ok(Value::Mount(Mount::Overlay {
                    left: length(self, "left")?.unwrap_or(zero),
                    right: length(self, "right")?.unwrap_or(zero),
                    top: length(self, "top")?.unwrap_or(zero),
                    bottom: length(self, "bottom")?.unwrap_or(zero),
                    meeting: length(self, "meeting")?,
                }))
            }
            Builtin::Inset => Ok(Value::Mount(Mount::Inset {
                reveal: length(self, "reveal")?.ok_or(Reported)?,
            })),
            Builtin::Turned => match Self::arg(args, "profile")
                .map(|e| self.eval(e, env))
                .transpose()?
            {
                Some(Value::Profile(profile)) => Ok(Value::Turned(profile)),
                _ => Err(Reported),
            },
            Builtin::Box => Ok(Value::Envelope(Envelope::Box {
                length: length(self, "length")?.ok_or(Reported)?,
                width: length(self, "width")?.ok_or(Reported)?,
                thickness: length(self, "thickness")?.ok_or(Reported)?,
            })),
            Builtin::Cylinder => Ok(Value::Envelope(Envelope::Cylinder {
                diameter: length(self, "diameter")?.ok_or(Reported)?,
                length: length(self, "length")?.ok_or(Reported)?,
            })),
            Builtin::Model => {
                // node("name") is lowered as a model call with a `name` argument.
                if let Some(name) = Self::arg(args, "name") {
                    return self.eval(name, env);
                }
                let source = match Self::arg(args, "source")
                    .map(|e| self.eval(e, env))
                    .transpose()?
                {
                    Some(Value::Text(text)) => text,
                    _ => return Err(Reported),
                };
                let align = match Self::arg(args, "align")
                    .map(|e| self.eval(e, env))
                    .transpose()?
                {
                    Some(Value::Text(node)) => ModelAlign::Node(node),
                    _ => ModelAlign::Bounds,
                };
                Ok(Value::Model(ModelRef { source, align }))
            }
            Builtin::Ends => Ok(Value::Spacing(Spacing::Ends(
                length(self, "inset")?.unwrap_or(ExactLength::ZERO),
            ))),
            Builtin::Pitch => Ok(Value::Spacing(Spacing::Pitch(
                length(self, "distance")?.ok_or(Reported)?,
            ))),
            Builtin::Gap => Ok(Value::Spacing(Spacing::Gap(
                length(self, "distance")?.ok_or(Reported)?,
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
                                .ok_or(Reported)?,
                        );
                    } else if box_b.max(axis) <= box_a.min(axis) {
                        apart.push(
                            box_a
                                .min(axis)
                                .checked_sub(box_b.max(axis))
                                .ok_or(Reported)?,
                        );
                    }
                }
                match apart.as_slice() {
                    [distance] => Ok(*distance),
                    _ => {
                        let (name_a, name_b) = (self.part_name(*a), self.part_name(*b));
                        Err(self.eval_error(env, span, format!("'between' needs parts apart along exactly one axis, but {name_a} and {name_b} are apart along {}", apart.len()), "measure between two parallel faces instead: between(a.top, b.bottom)"))
                    }
                }
            }
            (Value::Anchor(a), Value::Anchor(b)) => {
                if a.axis != b.axis {
                    return Err(self.eval_error(
                        env,
                        span,
                        "'between' needs two parallel planes",
                        "name two faces across the same axis",
                    ));
                }
                let distance = a.position.checked_sub(b.position).ok_or(Reported)?;
                Ok(if distance < ExactLength::ZERO {
                    distance.checked_neg().ok_or(Reported)?
                } else {
                    distance
                })
            }
            _ => Err(self.eval_error(
                env,
                span,
                format!(
                    "'between' needs two parts or two faces, found {} and {}",
                    a.description(),
                    b.description()
                ),
                "write between(a, b) with two parts",
            )),
        }
    }

    fn region_argument(&mut self, expr: &RExpr, env: &Env<'_>) -> Solved<RegionValue> {
        match self.eval(expr, env)? {
            Value::Region(region) => self.region_value(region),
            Value::Regions(list) if list.len() == 1 => list.first().copied().ok_or(Reported),
            other => Err(self.eval_error(
                env,
                expr.span,
                format!("this needs a region, found {}", other.description()),
                "name a region",
            )),
        }
    }

    fn split(
        &mut self,
        args: &[stave_model::defs::Setting],
        env: &Env<'_>,
        span: Span,
    ) -> Solved<Value> {
        let (Some(region), Some(axis)) = (Self::arg(args, "region"), Self::arg(args, "axis"))
        else {
            return Err(Reported);
        };
        let region = self.region_argument(region, env)?;
        let Value::Word(Word::Axis(axis)) = self.eval(axis, env)? else {
            return Err(Reported);
        };
        let bounds = region.bounds;
        let (low, high) = (bounds.min(axis), bounds.max(axis));
        if let Some(at) = Self::arg(args, "at") {
            let part = self.eval_part(at, env)?;
            let part_box = self.drawn_box(part)?;
            let (p0, p1) = (part_box.min(axis), part_box.max(axis));
            if p0 < low || p1 > high {
                let name = self.part_name(part);
                return Err(self.eval_error(
                    env,
                    at.span,
                    format!(
                        "'split' needs a part inside the region, but {name} reaches outside it"
                    ),
                    "split at a part that lies within the region",
                ));
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
            return Err(self.eval_error(
                env,
                span,
                "'split' without 'at:' needs an array to fill",
                "declare the region as an array: region bay[a, b] = split(...)",
            ));
        };
        let size = high.checked_sub(low).ok_or(Reported)?;
        let n = i64::try_from(count).map_err(|_| Reported)?;
        let piece = size.checked_div(Rational::from(n)).ok_or(Reported)?;
        let mut pieces = Vec::new();
        for index in 0..count {
            let i = i64::try_from(index).map_err(|_| Reported)?;
            let start = low
                .checked_add(piece.checked_mul(Rational::from(i)).ok_or(Reported)?)
                .ok_or(Reported)?;
            let end = if index + 1 == count {
                high
            } else {
                start.checked_add(piece).ok_or(Reported)?
            };
            pieces.push(RegionValue {
                bounds: bounds.with(axis, start, end),
                origin: RegionOrigin::Split { axis, index, count },
            });
        }
        Ok(Value::Regions(pieces))
    }

    fn cover(
        &mut self,
        args: &[stave_model::defs::Setting],
        env: &Env<'_>,
        _span: Span,
    ) -> Solved<Value> {
        let (Some(region), Some(mount), Some(thickness)) = (
            Self::arg(args, "region"),
            Self::arg(args, "mount"),
            Self::arg(args, "thickness"),
        ) else {
            return Err(Reported);
        };
        let region = self.region_argument(region, env)?;
        let Value::Mount(mount) = self.eval(mount, env)? else {
            return Err(Reported);
        };
        let thickness = self.eval_length(thickness, env)?;
        let b = region.bounds;
        let add = |a: ExactLength, d: ExactLength| a.checked_add(d).ok_or(Reported);
        let sub = |a: ExactLength, d: ExactLength| a.checked_sub(d).ok_or(Reported);
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
