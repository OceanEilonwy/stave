//! The help search: when a kind's rule that compares for equality fails,
//! try every value of each of the kind's `within` lists, one param at a time
//! with the others as given, and offer the values that make it hold (see
//! "Checks and `require`" in `docs/grammar.md`).

use std::cell::RefCell;

use stave_lang::ast::{BinaryOp, Span};
use stave_model::defs::{DefId, Domain, RExpr, RExprKind};
use stave_model::value::Value;

use crate::Solver;
use crate::eval::{Env, KindEnv};

/// A list that holds at least one item.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct NonEmpty<T> {
    first: T,
    rest: Vec<T>,
}

impl<T> NonEmpty<T> {
    /// The items, or `None` when there are none.
    fn new(items: Vec<T>) -> Option<Self> {
        let mut items = items.into_iter();
        let first = items.next()?;
        Some(NonEmpty {
            first,
            rest: items.collect(),
        })
    }

    /// The items in order.
    pub(crate) fn iter(&self) -> impl Iterator<Item = &T> {
        std::iter::once(&self.first).chain(&self.rest)
    }
}

/// A value from one of the kind's `within` lists that makes the failed rule
/// hold.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Fitting {
    /// The param's name, as written in the kind.
    pub param: String,
    /// The value from its list.
    pub value: Value,
}

/// A `within` list the search tried, for the help when nothing fits.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SearchedList {
    /// The param's name, as written in the kind.
    pub param: String,
    /// Its values, as written in a message.
    pub values: Vec<String>,
}

/// What the help search found.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum HelpSearch {
    /// The rule doesn't compare for equality, or the kind has no `within`
    /// list: the general help stays.
    NotSearched,
    /// These values fit, in the order the params and lists are written.
    Fits(NonEmpty<Fitting>),
    /// No value of these lists fits.
    NoneFit(NonEmpty<SearchedList>),
}

/// `true` when a condition compares for equality: it is `a == b`, or
/// conditions joined by `&&` with at least one `a == b` among them.
fn compares_for_equality(condition: &RExpr) -> bool {
    match &condition.kind {
        RExprKind::Binary {
            op: BinaryOp::Eq, ..
        } => true,
        RExprKind::Binary {
            op: BinaryOp::And,
            lhs,
            rhs,
        } => compares_for_equality(lhs) || compares_for_equality(rhs),
        RExprKind::Binary {
            op:
                BinaryOp::Or
                | BinaryOp::Ne
                | BinaryOp::Lt
                | BinaryOp::Le
                | BinaryOp::Gt
                | BinaryOp::Ge
                | BinaryOp::Add
                | BinaryOp::Sub
                | BinaryOp::Mul
                | BinaryOp::Div
                | BinaryOp::In,
            ..
        }
        | RExprKind::Length(_)
        | RExprKind::Angle(_)
        | RExprKind::Number(_)
        | RExprKind::Bool(_)
        | RExprKind::Text(_)
        | RExprKind::Fit
        | RExprKind::Word(_)
        | RExprKind::Symbol(_)
        | RExprKind::SelfElement(_)
        | RExprKind::Member { .. }
        | RExprKind::Index { .. }
        | RExprKind::Builtin { .. }
        | RExprKind::KindValue { .. }
        | RExprKind::Unary { .. }
        | RExprKind::List(_)
        | RExprKind::Error => false,
    }
}

impl Solver<'_> {
    /// Searches the kind's `within` lists for values that make `condition`
    /// hold, given the roles and settings of `given`. Each trial is
    /// evaluated quietly: what it would report is dropped, and a trial that
    /// can't be evaluated doesn't fit.
    pub(crate) fn help_search(&mut self, condition: &RExpr, given: &KindEnv) -> HelpSearch {
        if !compares_for_equality(condition) {
            return HelpSearch::NotSearched;
        }
        let Some(kind) = self.model.defs.kind(given.kind) else {
            return HelpSearch::NotSearched;
        };
        let Some(ctx) = self.module_ctx(DefId::Kind(given.kind)) else {
            return HelpSearch::NotSearched;
        };
        let base = Env {
            ctx,
            file: kind.decl.file,
            kind: None,
            array: None,
        };
        self.search_lists(given, condition.span, &mut |solver, trial| {
            let trial_env = Env {
                kind: Some(trial),
                ..base
            };
            solver.eval_bool(condition, &trial_env).ok() == Some(true)
        })
    }

    /// The general search (grammar, "Checks and `require`"): tries every
    /// value of each of the kind's `within` lists, one param at a time with
    /// the others as `given` has them, and keeps the values for which
    /// `fits` says yes. Each trial runs quietly: what it would report is
    /// dropped.
    pub(crate) fn search_lists(
        &mut self,
        given: &KindEnv,
        span: Span,
        fits: &mut dyn FnMut(&mut Self, &KindEnv) -> bool,
    ) -> HelpSearch {
        let Some(kind) = self.model.defs.kind(given.kind) else {
            return HelpSearch::NotSearched;
        };
        let Some(ctx) = self.module_ctx(DefId::Kind(given.kind)) else {
            return HelpSearch::NotSearched;
        };
        let file = kind.decl.file;
        let lists: Vec<(String, Vec<RExpr>)> = kind
            .params
            .iter()
            .filter_map(|param| match &param.domain {
                Some(Domain::OneOf { values, .. }) => {
                    Some((param.decl.name.clone(), values.clone()))
                }
                Some(Domain::Range { .. }) | None => None,
            })
            .collect();
        let param_count = kind.params.len();
        let module_env = Env {
            ctx,
            file,
            kind: None,
            array: None,
        };
        let mut found = Vec::new();
        let mut searched = Vec::new();
        for (param, values) in lists {
            let current = self.quietly(|solver| {
                let index = solver
                    .model
                    .defs
                    .kind(given.kind)
                    .and_then(|kind| kind.param(&param));
                match index {
                    Some(index) => solver.kind_param(given, index, file, span).ok(),
                    None => None,
                }
            });
            let mut texts = Vec::new();
            for value_expr in &values {
                let Some(value) = self.quietly(|solver| solver.eval(value_expr, &module_env).ok())
                else {
                    continue;
                };
                texts.push(self.describe(&value));
                if current.as_ref() == Some(&value) {
                    continue;
                }
                let mut settings: Vec<(String, Value)> = given
                    .settings
                    .iter()
                    .filter(|(name, _)| *name != param)
                    .cloned()
                    .collect();
                settings.push((param.clone(), value.clone()));
                let trial = KindEnv {
                    kind: given.kind,
                    roles: given.roles.clone(),
                    settings,
                    params: RefCell::new(vec![None; param_count]),
                    made: given.made.clone(),
                    mount: given.mount,
                };
                if self.quietly(|solver| fits(solver, &trial)) {
                    found.push(Fitting {
                        param: param.clone(),
                        value,
                    });
                }
            }
            searched.push(SearchedList {
                param,
                values: texts,
            });
        }
        if let Some(found) = NonEmpty::new(found) {
            return HelpSearch::Fits(found);
        }
        match NonEmpty::new(searched) {
            Some(searched) => HelpSearch::NoneFit(searched),
            None => HelpSearch::NotSearched,
        }
    }

    /// The help line for what the search found, or `None` to keep the
    /// general help.
    pub(crate) fn search_help(&self, search: &HelpSearch) -> Option<String> {
        match search {
            HelpSearch::NotSearched => None,
            HelpSearch::Fits(fits) => {
                let values: Vec<String> = fits
                    .iter()
                    .map(|fit| format!("{}: {}", fit.param, self.describe(&fit.value)))
                    .collect();
                let plural = if values.len() == 1 { "" } else { "s" };
                Some(format!(
                    "use {}, from the kind's list{plural}",
                    values.join(" or ")
                ))
            }
            HelpSearch::NoneFit(searched) => {
                let lists: Vec<String> = searched
                    .iter()
                    .map(|list| format!("{} within [{}]", list.param, list.values.join(", ")))
                    .collect();
                Some(format!(
                    "no value in the kind's lists makes this hold ({}); change the parts' sizes",
                    lists.join(", ")
                ))
            }
        }
    }

    /// Runs `trial` and drops whatever it reported, so a search can try
    /// values without leaving diagnostics behind.
    fn quietly<T>(&mut self, trial: impl FnOnce(&mut Self) -> T) -> T {
        let diagnostics = self.diagnostics.len();
        let cycles = self.reported_cycles.len();
        let stack = self.stack.len();
        let skipped = self.skipped;
        let result = trial(self);
        self.diagnostics.truncate(diagnostics);
        self.reported_cycles.truncate(cycles);
        self.stack.truncate(stack);
        self.skipped = skipped;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_non_empty_list_keeps_its_order_and_an_empty_one_is_refused() {
        let list = NonEmpty::new(vec![3, 1, 2]).unwrap();
        assert_eq!(list.iter().copied().collect::<Vec<_>>(), vec![3, 1, 2]);
        assert!(NonEmpty::<i32>::new(Vec::new()).is_none());
    }
}
