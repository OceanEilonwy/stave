//! Applicability (`docs/editing.md`, "Signature matching", step 3): each
//! listed candidate is built in a scratch copy with its defaults and
//! checked, so the menu can grey the ones that can't go here with the
//! checker's own message.
//!
//! ```text
//! [Candidate] ──write_patch──▶ patch ──EditBase::try_patch──▶ Fits    ─▶ Applies
//!      │                                                    └ Refused ─▶ Greyed(the errors)
//!      └ a place param to point at, or a setting to choose ─▶ NeedsChoice(their names)
//! ```
//!
//! A candidate that is not complete is never tried: it *needs a choice*,
//! which is not a refusal. Trying borrows the [`EditBase`], so the prepared
//! base is never changed, and every candidate is tried from the design as
//! it is. [`check_menu`] works through a menu in order and can be stopped
//! part way with a flag: the candidates it reached keep their answer, the
//! rest stay [`Applicability::NotChecked`].

use std::sync::atomic::{AtomicBool, Ordering};

use stave_check::Cancelled;
use stave_lang::ast::{Name, Span};
use stave_lang::{Code, Diagnostic, Severity};
use stave_model::nonempty::NonEmpty;

use super::EditScope;
use crate::edit::{Applicability, Candidate, Verdict};
use crate::scratch::{EditBase, TryError};

/// The name a candidate's patch is tried under: refusals that point into
/// the patch itself name this file.
pub const PATCH_NAME: &str = "edit.stave";

/// What is still to choose before a candidate can be tried: its unbound
/// place params, then its settings with no value, by name. `None` when it
/// is complete.
pub fn still_to_choose(candidate: &Candidate) -> Option<NonEmpty<Name>> {
    let names = candidate
        .unbound_place()
        .map(|param| param.name.clone())
        .chain(
            candidate
                .unset_settings()
                .filter_map(|param| Name::new(param.name.as_str(), Span::default()).ok()),
        )
        .collect();
    NonEmpty::new(names)
}

/// Whether `candidate` applies to the design `base` holds: built in a
/// scratch copy with its defaults (and the settings it copies or was
/// given) and checked in both tiers. `scope` is the editor's view of the
/// same design (`EditScope::new(base.opened())`).
///
/// # Errors
///
/// [`Cancelled`] when `cancel` is set before the check finishes.
pub fn applicability(
    base: &EditBase<'_>,
    scope: &EditScope<'_>,
    candidate: &Candidate,
    cancel: &AtomicBool,
) -> Result<Applicability, Cancelled> {
    if let Some(names) = still_to_choose(candidate) {
        return Ok(Applicability::NeedsChoice(names));
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(Cancelled);
    }
    let written = match scope.write_patch(candidate, None) {
        Ok(written) => written,
        Err(error) => return Ok(Applicability::Greyed(NonEmpty::one(unwritable(&error)))),
    };
    match base.try_patch(PATCH_NAME, &written.parse, cancel) {
        Ok(tried) => Ok(match tried.verdict() {
            Verdict::Fits(_) => Applicability::Applies,
            Verdict::Refused(refusal) => Applicability::Greyed(
                NonEmpty::new(once_each(
                    refusal
                        .problems()
                        .iter()
                        .map(|problem| problem.diagnostic().clone()),
                ))
                .unwrap_or_else(|| NonEmpty::one(unchecked("the check found nothing to say"))),
            ),
            // A finished try is never still checking; not applying is the
            // safe answer.
            Verdict::Checking => {
                Applicability::Greyed(NonEmpty::one(unchecked("the check didn't finish")))
            }
        }),
        Err(TryError::Cancelled(cancelled)) => Err(cancelled),
        Err(TryError::Load(error)) => Ok(Applicability::Greyed(NonEmpty::one(unchecked(
            &error.to_string(),
        )))),
    }
}

/// Works out the applicability of every candidate of `menu` that is not
/// checked yet, in menu order, writing each answer into the candidate as
/// soon as it is known. The menu's order is left alone: sort it again
/// with [`super::sort_menu`] to put the greyed entries after the rest.
///
/// # Errors
///
/// [`Cancelled`] when `cancel` is set: the candidates reached so far keep
/// their answer, the rest stay [`Applicability::NotChecked`].
pub fn check_menu(
    base: &EditBase<'_>,
    scope: &EditScope<'_>,
    menu: &mut [Candidate],
    cancel: &AtomicBool,
) -> Result<(), Cancelled> {
    for candidate in menu.iter_mut() {
        if candidate.applicability != Applicability::NotChecked {
            continue;
        }
        candidate.applicability = applicability(base, scope, candidate, cancel)?;
    }
    Ok(())
}

/// The diagnostics in order, each said once: a later one with the same
/// severity, code and message as an earlier one (the same error for each
/// element of an array, say) is left out.
fn once_each(diagnostics: impl Iterator<Item = Diagnostic>) -> Vec<Diagnostic> {
    let mut out: Vec<Diagnostic> = Vec::new();
    for diagnostic in diagnostics {
        let seen = out.iter().any(|earlier| {
            earlier.severity == diagnostic.severity
                && earlier.code == diagnostic.code
                && earlier.message == diagnostic.message
        });
        if !seen {
            out.push(diagnostic);
        }
    }
    out
}

/// A candidate whose patch can't be written, as a diagnostic for its
/// greyed entry.
fn unwritable(error: &super::WriteError) -> Diagnostic {
    unchecked(&error.to_string())
}

/// A message about a candidate that has no place in a file to point at:
/// a diagnostic without a label.
fn unchecked(message: &str) -> Diagnostic {
    Diagnostic {
        severity: Severity::Error,
        code: Code::NotSupported,
        message: message.to_owned(),
        labels: Vec::new(),
        notes: Vec::new(),
        merged: Vec::new(),
        help: None,
        fixes: Vec::new(),
    }
}
