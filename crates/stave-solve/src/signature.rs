//! A kind chosen through a param, used where it can't be (grammar, "Kind
//! values"): a component takes a joint or hardware kind as a setting and
//! then uses it, handing it roles and settings of its own. When the kind
//! chosen doesn't take what the component hands it (a setting it doesn't
//! have, a role it doesn't have, several parts where it takes one), the
//! kind can never be used there, whatever the sizes. That is one error,
//! said in the words of the choice: this kind can't be that param of that
//! component, why, and which kinds in scope can.

use stave_lang::ast::{BaseKind, Span};
use stave_lang::{Cause, Code, Diagnostic, FileId, Label};
use stave_model::defs::{DefId, KindCallDef, KindDef, RExpr, RExprKind, Scope, Symbol};
use stave_model::ids::{KindId, ParamDefId};
use stave_model::instance::{Ctx, ItemSlot, ParamSource};
use stave_model::value::Value;
use stave_schema::kinds::RoleType;

use crate::eval::Env;
use crate::failure::Accounted;
use crate::{Solved, Solver};

/// What a joint or hardware declaration hands its kind: the positional
/// argument and the named ones, evaluated.
pub(crate) struct Handed {
    positional: Option<(Value, Span)>,
    /// Name, value, where the name is written, where the value is.
    named: Vec<(String, Value, Span, Span)>,
    /// The call's kind, for a role handed nothing.
    kind_span: Span,
}

/// Why a kind doesn't take what a declaration hands it: the first thing
/// that doesn't match, in the order the call writes them.
#[derive(Debug, Clone, PartialEq)]
enum Mismatch {
    /// A setting (a value that is no part or region) the kind has no
    /// param for.
    NoSetting { name: String, span: Span },
    /// Parts or a region handed as a role the kind doesn't have.
    NoRole {
        name: String,
        value: Value,
        span: Span,
    },
    /// A role handed the wrong sort of thing: several parts where the
    /// kind takes one, a part where it takes a region.
    WrongSort {
        role: String,
        wanted: RoleType,
        value: Value,
        span: Span,
    },
    /// A role of the kind the declaration hands nothing.
    NothingFor {
        role: String,
        wanted: RoleType,
        span: Span,
    },
}

/// Whether `value` is what a role of type `wanted` takes.
fn fits_role(wanted: RoleType, value: &Value) -> bool {
    match wanted {
        RoleType::Part => matches!(value, Value::Part(_)),
        RoleType::Parts => value.parts().is_some(),
        RoleType::Region => matches!(value, Value::Region(_)),
    }
}

/// Whether a value is handed as a role (parts or a region) rather than
/// as a setting.
fn is_role_value(value: &Value) -> bool {
    value.parts().is_some() || matches!(value, Value::Region(_) | Value::Regions(_))
}

/// The settings every kind of a base takes besides its params.
fn takes_setting(kind: &KindDef, name: &str) -> bool {
    if kind.param(name).is_some() {
        return true;
    }
    match kind.base {
        BaseKind::Joint => {
            stave_schema::fields::find(stave_schema::fields::JOINT_IMPLICIT_SETTINGS, name)
                .is_some()
        }
        base if base.is_mounted_hardware() => name == stave_schema::kinds::FACE_SETTING,
        _ => false,
    }
}

/// The signature test: the first thing `kind` doesn't take of what is
/// handed to it, or `None` when it takes all of it.
fn mismatch(kind: &KindDef, handed: &Handed) -> Option<Mismatch> {
    let mut given: Vec<Option<(&Value, Span)>> = vec![None; kind.roles.len()];
    if let Some((value, span)) = &handed.positional {
        // A kind with no roles at all is not a joint or hardware kind;
        // its own error says so.
        let slot = given.first_mut()?;
        *slot = Some((value, *span));
    }
    for (name, value, name_span, value_span) in &handed.named {
        if let Some(index) = kind.role_named(name) {
            if let Some(slot) = given.get_mut(index.index()) {
                *slot = Some((value, *value_span));
            }
            continue;
        }
        if takes_setting(kind, name) {
            continue;
        }
        return Some(if is_role_value(value) {
            Mismatch::NoRole {
                name: name.clone(),
                value: value.clone(),
                span: *name_span,
            }
        } else {
            Mismatch::NoSetting {
                name: name.clone(),
                span: *name_span,
            }
        });
    }
    for (role, slot) in kind.roles.iter().zip(&given) {
        if let Some((value, span)) = slot
            && !fits_role(role.ty, value)
        {
            return Some(Mismatch::WrongSort {
                role: role.name.clone(),
                wanted: role.ty,
                value: (*value).clone(),
                span: *span,
            });
        }
    }
    for (role, slot) in kind.roles.iter().zip(&given) {
        if slot.is_none() {
            return Some(Mismatch::NothingFor {
                role: role.name.clone(),
                wanted: role.ty,
                span: handed.kind_span,
            });
        }
    }
    None
}

/// Where a kind value that reached a declaration through a param was
/// chosen: the value written for the param (an assembly's argument, or
/// the param's default when it was not set), and that param.
struct Choice {
    file: FileId,
    span: Span,
    param: ParamDefId,
}

/// "a" or "an" before a word.
fn article(word: &str) -> &'static str {
    match word.chars().next() {
        Some('a' | 'e' | 'i' | 'o' | 'u') => "an",
        _ => "a",
    }
}

/// `a`, `a and b`, `a, b and c`.
fn and_list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// A count in words, for the small numbers a message names.
fn count_words(count: usize) -> String {
    match count {
        2 => "two".to_owned(),
        3 => "three".to_owned(),
        4 => "four".to_owned(),
        5 => "five".to_owned(),
        6 => "six".to_owned(),
        7 => "seven".to_owned(),
        8 => "eight".to_owned(),
        9 => "nine".to_owned(),
        10 => "ten".to_owned(),
        other => other.to_string(),
    }
}

/// What sort of thing a value handed as a role is: `a part`, `parts`,
/// `a space`.
fn sort_words(value: &Value) -> &'static str {
    match value {
        Value::Part(_) => "a part",
        Value::Region(_) | Value::Regions(_) => "a space",
        other => match other.parts() {
            Some(parts) if parts.len() == 1 => "a part",
            Some(_) => "parts",
            None => other.description(),
        },
    }
}

fn wanted_words(wanted: RoleType) -> &'static str {
    match wanted {
        RoleType::Part => "one part",
        RoleType::Parts => "parts",
        RoleType::Region => "a space",
    }
}

impl Solver<'_> {
    /// For a kind that reached a joint or hardware declaration through a
    /// param: `Err` once the one error is reported when the kind doesn't
    /// take what the declaration hands it, `Ok` when it does or the kind
    /// was written in the declaration itself (whose own messages say what
    /// is wrong there).
    pub(crate) fn check_kind_signature(
        &mut self,
        call: &KindCallDef,
        env: &Env<'_>,
        kind: KindId,
        kind_name: &str,
        owner: DefId,
    ) -> Solved<()> {
        let RExprKind::Symbol(Symbol::Def(DefId::Param(param))) = &call.kind.kind else {
            return Ok(());
        };
        let positional = match &call.positional {
            Some(expr) => Some((self.eval(expr, env)?, expr.span)),
            None => None,
        };
        let mut named = Vec::new();
        for setting in &call.named {
            let value = self.eval(&setting.value, env)?;
            named.push((
                setting.name.clone(),
                value,
                setting.name_span,
                setting.value.span,
            ));
        }
        let handed = Handed {
            positional,
            named,
            kind_span: call.kind.span,
        };
        let Some(found) = self
            .model
            .defs
            .kind(kind)
            .and_then(|kind_def| mismatch(kind_def, &handed))
        else {
            return Ok(());
        };
        let Some(kind_def) = self.model.defs.kind(kind) else {
            return Ok(());
        };
        let choice = self.kind_choice(*param, env.ctx).unwrap_or(Choice {
            file: env.file,
            span: call.kind.span,
            param: *param,
        });
        let diagnostic = self.kind_signature_diagnostic(
            kind_def, kind, kind_name, &handed, &found, &choice, *param, env, owner,
        );
        Err(self.report_kind_signature(diagnostic, owner).into())
    }

    /// Follows a param to the value written for it: through an assembly's
    /// argument or its default, and on through a param that only passes
    /// another one along.
    fn kind_choice(&self, param: ParamDefId, ctx: Ctx) -> Option<Choice> {
        let &ItemSlot::Param(mut id) = self.model.item(DefId::Param(param), ctx)? else {
            return None;
        };
        // A chain of params handing one value along ends where the value
        // is written; a cycle is its own error, so stop after a while.
        for _ in 0..64 {
            let inst = self.model.params.get(id.index())?;
            let (expr, file, ctx): (&RExpr, FileId, Ctx) = match inst.source {
                ParamSource::SameAs(other) => {
                    id = other;
                    continue;
                }
                ParamSource::Default => {
                    let def = self.model.defs.param(inst.def)?;
                    (
                        def.default.as_ref()?,
                        def.decl.file,
                        Ctx {
                            instance: inst.instance,
                            element: None,
                        },
                    )
                }
                ParamSource::Argument {
                    assembly,
                    index,
                    ctx,
                } => {
                    let def = self.model.defs.assembly(assembly)?;
                    let stave_model::defs::AssemblyKind::Instance { args, .. } = &def.kind else {
                        return None;
                    };
                    (&args.get(index)?.1, def.decl.file, ctx)
                }
            };
            match &expr.kind {
                RExprKind::Symbol(Symbol::Def(DefId::Param(next))) => {
                    let ItemSlot::Param(next) = self.model.item(DefId::Param(*next), ctx)? else {
                        return None;
                    };
                    id = *next;
                }
                _ => {
                    return Some(Choice {
                        file,
                        span: expr.span,
                        param: inst.def,
                    });
                }
            }
        }
        None
    }

    /// The component a declaration or param is written in, by name, or
    /// `None` at the top level of a module.
    fn component_of(&self, def: DefId) -> Option<String> {
        match self.model.defs.decl(def)?.scope {
            Scope::Component(component) => self
                .model
                .defs
                .decl(DefId::Component(component))
                .map(|decl| decl.name.clone()),
            Scope::Module(_) => None,
        }
    }

    /// Parts or a space as a message names them.
    fn handed_words(&self, value: &Value) -> String {
        match value {
            Value::Region(region) => self
                .model
                .regions
                .get(region.index())
                .map(|r| r.name.clone())
                .unwrap_or_else(|| "a space".to_owned()),
            other => match other.parts() {
                Some(parts) => {
                    let names: Vec<String> = parts.iter().map(|p| self.part_name(*p)).collect();
                    and_list(&names)
                }
                None => other.description().to_owned(),
            },
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn kind_signature_diagnostic(
        &self,
        kind_def: &KindDef,
        kind: KindId,
        kind_name: &str,
        handed: &Handed,
        found: &Mismatch,
        choice: &Choice,
        used: ParamDefId,
        env: &Env<'_>,
        owner: DefId,
    ) -> Diagnostic {
        let param_name = self
            .model
            .defs
            .param(choice.param)
            .map(|p| p.decl.name.clone())
            .unwrap_or_default();
        let message = match self.component_of(DefId::Param(choice.param)) {
            Some(component) => format!("'{kind_name}' can't be the {param_name} of {component}"),
            None => format!("'{kind_name}' can't be the project's {param_name}"),
        };
        let thing = if kind_def.base == BaseKind::Joint {
            "joint"
        } else {
            "hardware"
        };
        let declared = self
            .model
            .defs
            .decl(owner)
            .map(|d| d.name.clone())
            .unwrap_or_default();
        // Who hands the kind what: the component whose declaration uses
        // it, or the project for a declaration of its own.
        let (who, its) = match self.component_of(owner) {
            Some(component) => (component, format!("its {thing}")),
            None => ("the project".to_owned(), declared),
        };
        let roles: Vec<String> = kind_def.roles.iter().map(|r| r.name.clone()).collect();
        let (reason, at) = match found {
            Mismatch::NoSetting { name, span } => (
                format!(
                    "{who} gives {its} {} {name}, and {kind_name} has no {name}",
                    article(name)
                ),
                *span,
            ),
            Mismatch::NoRole { name, value, span } => (
                format!(
                    "{who} hands {its} {} as {name}, and {kind_name} has no {name}: it takes its parts only as {}",
                    sort_words(value),
                    and_list(&roles)
                ),
                *span,
            ),
            Mismatch::WrongSort {
                role,
                wanted,
                value,
                span,
            } => {
                let given = match value.parts() {
                    Some(parts) if parts.len() > 1 => format!(
                        "{} parts as {role} ({})",
                        count_words(parts.len()),
                        self.handed_words(value)
                    ),
                    _ => format!("{} as {role}", self.handed_words(value)),
                };
                (
                    format!(
                        "{who} hands {its} {given}, and {kind_name} takes {} there",
                        wanted_words(*wanted)
                    ),
                    *span,
                )
            }
            Mismatch::NothingFor { role, wanted, span } => (
                format!(
                    "{who} hands {its} nothing as {role}, and {kind_name} needs {} there",
                    wanted_words(*wanted)
                ),
                *span,
            ),
        };
        let accepted = self.kinds_taking(kind_def.base, kind, handed, choice.file);
        let help = if accepted.is_empty() {
            format!(
                "no {thing} kind in scope takes what {who} hands {its}; use another component, or write a kind that does"
            )
        } else {
            format!(
                "choose a kind that takes what {who} hands {its}: {}",
                accepted.join(", ")
            )
        };
        let inner = self
            .model
            .defs
            .decl(DefId::Param(used))
            .map(|d| d.name.clone())
            .unwrap_or_default();
        Diagnostic::error(
            Code::TypeMismatch,
            message,
            Label::new(choice.file, choice.span),
        )
        .with_label(Label::with_message(
            env.file,
            at,
            format!("{who} uses {inner} here"),
        ))
        .with_cause(Cause::KindSignature, reason)
        .with_help(help)
    }

    /// The kinds of the same base in scope where the choice is made that
    /// take what is handed, by the same test, as that file names them:
    /// its own module's first, then its imports', then the prelude's.
    fn kinds_taking(
        &self,
        base: BaseKind,
        refused: KindId,
        handed: &Handed,
        file: FileId,
    ) -> Vec<String> {
        let defs = &self.model.defs;
        let Some(scope) = defs.files.get(&file) else {
            return Vec::new();
        };
        let mut modules = Vec::new();
        modules.extend(scope.module);
        modules.extend(scope.imports.values().copied());
        modules.extend(defs.prelude.iter().copied());
        let mut seen = Vec::new();
        let mut out = Vec::new();
        for module in modules {
            if seen.contains(&module) {
                continue;
            }
            seen.push(module);
            let Some(module_defs) = defs.modules.get(module.index()) else {
                continue;
            };
            for item in &module_defs.items {
                let DefId::Kind(id) = *item else {
                    continue;
                };
                let Some(kind) = defs.kind(id) else {
                    continue;
                };
                let own = scope.module == Some(module);
                if id == refused
                    || kind.base != base
                    || kind.decl.scope != Scope::Module(module)
                    || (kind.decl.private && !own)
                    || defs.is_poisoned(*item)
                    || mismatch(kind, handed).is_some()
                {
                    continue;
                }
                let name = defs.kind_display(id, Some(file));
                if !out.contains(&name) {
                    out.push(name);
                }
            }
        }
        out
    }

    /// One mistake, one error: a declaration whose kind fails the same way
    /// at the same choice as an earlier one (another element of an array,
    /// or a second joint of the same component handed the same) is a note
    /// and a place on the earlier error. Either way the failure is
    /// accounted for: reported here, or folded into an error already
    /// reported, which counts as skipped.
    fn report_kind_signature(&mut self, diagnostic: Diagnostic, owner: DefId) -> Accounted {
        let declared = self
            .model
            .defs
            .decl(owner)
            .map(|d| d.name.clone())
            .unwrap_or_default();
        let earlier = self.diagnostics.iter_mut().find(|earlier| {
            earlier.cause.is_some()
                && earlier.cause == diagnostic.cause
                && earlier.reason() == diagnostic.reason()
                && earlier.message == diagnostic.message
                && earlier.primary() == diagnostic.primary()
        });
        let Some(earlier) = earlier else {
            return self.report(diagnostic).into();
        };
        if let Some(place) = diagnostic.labels.get(1)
            && !earlier.labels.contains(place)
        {
            earlier.labels.push(place.clone());
            let note = format!("the same for {declared}");
            if !earlier.notes.contains(&note) {
                earlier.notes.push(note);
            }
        }
        self.skip().into()
    }
}
