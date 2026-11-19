//! How solving fails, and the proof that every failure reaches the user.
//!
//! A step that fails returns a [`Failure`]. A failure is one of three
//! things:
//!
//! - [`Reported`]: a diagnostic has been emitted. Only [`Reported::emit`]
//!   builds one, and it takes the diagnostic, so holding a `Reported` is
//!   proof that the user will see an error.
//! - [`Skipped`]: the step depends on an item with an error already
//!   reported, by an earlier stage (resolving names and types) or earlier
//!   in this one. Only [`Skipped::mark`] builds one, and it sets the
//!   solver's `skipped` flag so the design is withheld.
//! - [`Fault`]: something failed and nothing has said so yet, such as an
//!   exact sum too large to hold. A fault can't be put down: before a
//!   failure is dropped or remembered, [`crate::Solver::settle`] turns it
//!   into an [`Accounted`] failure by reporting it at the nearest place the
//!   solver knows.

use stave_lang::Diagnostic;

/// Proof that a diagnostic about this failure has been emitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Reported(());

impl Reported {
    /// Emits `diagnostic` into `sink` and returns the proof.
    pub(crate) fn emit(sink: &mut Vec<Diagnostic>, diagnostic: Diagnostic) -> Reported {
        sink.push(diagnostic);
        Reported(())
    }
}

/// Proof that the step was skipped because an item it depends on has an
/// error already reported, and that the design will be withheld.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Skipped(());

impl Skipped {
    /// Marks the solve as having skipped something and returns the proof.
    pub(crate) fn mark(skipped: &mut bool) -> Skipped {
        *skipped = true;
        Skipped(())
    }
}

/// A failure nobody has reported yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fault {
    /// An exact value grew too large, or too finely divided, to hold.
    Overflow,
    /// Something the solver relies on was missing: a bug in Stave. The text
    /// names what, in a few plain words.
    Internal(&'static str),
    /// Something an earlier check should have caught was wrong: a
    /// builtin's argument or an expression that did not resolve (checked
    /// by an earlier stage), or an item this stage already failed to work
    /// out. When no error has been reported anywhere it is a bug, and is
    /// reported as internal.
    Earlier,
}

/// A failure that has reached the user, or will through an earlier stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Accounted {
    /// A diagnostic was emitted.
    Reported(Reported),
    /// It was reported earlier.
    Skipped(Skipped),
}

/// Why a step of solving failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Failure {
    /// Reported, or skipped after an earlier stage's error.
    Accounted(Accounted),
    /// Not reported yet; see [`crate::Solver::settle`].
    Fault(Fault),
}

impl From<Reported> for Failure {
    fn from(reported: Reported) -> Self {
        Failure::Accounted(Accounted::Reported(reported))
    }
}

impl From<Skipped> for Failure {
    fn from(skipped: Skipped) -> Self {
        Failure::Accounted(Accounted::Skipped(skipped))
    }
}

impl From<Accounted> for Failure {
    fn from(accounted: Accounted) -> Self {
        Failure::Accounted(accounted)
    }
}

impl From<Fault> for Failure {
    fn from(fault: Fault) -> Self {
        Failure::Fault(fault)
    }
}

impl From<Reported> for Accounted {
    fn from(reported: Reported) -> Self {
        Accounted::Reported(reported)
    }
}

impl From<Skipped> for Accounted {
    fn from(skipped: Skipped) -> Self {
        Accounted::Skipped(skipped)
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use stave_lang::Code;
    use stave_model::Model;
    use stave_modules::{MemoryFiles, ModuleSet};

    use super::*;
    use crate::{Memo, Solver};

    const PROJECT: &str = "stave 1

project \"Test\" {
  units: mm
}

stock pine_19 { species: pine, form: solid, thickness: 19mm, max_length: 2400mm }

part base : pine_19 {
  length: 600mm
  width:  300mm
  place {
    ground
  }
}
";

    fn model() -> Model {
        let files = MemoryFiles::new().with("/project/project.stave", PROJECT);
        let modules = ModuleSet::load_with(Path::new("/project"), &files).unwrap();
        let (defs, diagnostics) = stave_model::resolve(&modules);
        assert!(diagnostics.iter().all(|d| !d.is_error()), "{diagnostics:?}");
        let (model, diagnostics) = stave_model::instance::instantiate(defs, modules.root);
        assert!(diagnostics.iter().all(|d| !d.is_error()), "{diagnostics:?}");
        model
    }

    fn solver(model: &Model) -> Solver<'_> {
        let project = model.defs.project(model.root).unwrap();
        Solver::new(model, project.file, project.span)
    }

    #[test]
    fn a_fault_with_no_error_anywhere_is_reported_as_internal() {
        let model = model();
        let mut solver = solver(&model);
        let (file, span) = (solver.project_file, solver.project_span);
        // `Earlier` first: once an error exists it is the error that
        // explains the failure, and an earlier fault is skipped.
        for fault in [Fault::Earlier, Fault::Internal("a part")] {
            let accounted = solver.settle(fault.into(), file, span);
            assert!(matches!(accounted, Accounted::Reported(_)));
        }
        let codes: Vec<Code> = solver.diagnostics.iter().map(|d| d.code).collect();
        assert_eq!(codes, vec![Code::Internal, Code::Internal]);
        assert!(solver.diagnostics.iter().all(|d| d.is_error()));
        assert!(!solver.skipped);
    }

    #[test]
    fn an_overflow_is_reported_where_it_happened() {
        let model = model();
        let mut solver = solver(&model);
        let (file, span) = (solver.project_file, solver.project_span);
        let accounted = solver.settle(Fault::Overflow.into(), file, span);
        assert!(matches!(accounted, Accounted::Reported(_)));
        let [diagnostic] = solver.diagnostics.as_slice() else {
            panic!("{:?}", solver.diagnostics);
        };
        assert_eq!(diagnostic.code, Code::OutOfRange);
        assert!(diagnostic.is_error());
    }

    #[test]
    fn an_earlier_fault_after_an_error_is_skipped() {
        let model = model();
        let mut solver = solver(&model);
        let (file, span) = (solver.project_file, solver.project_span);
        let _ = solver.error(Code::Ground, "the first mistake", file, span, "fix it");
        let accounted = solver.settle(Fault::Earlier.into(), file, span);
        assert!(matches!(accounted, Accounted::Skipped(_)));
        assert_eq!(solver.diagnostics.len(), 1, "one mistake, one error");
        assert!(solver.skipped, "the design is withheld");
    }

    #[test]
    fn an_accounted_failure_is_not_reported_again() {
        let model = model();
        let mut solver = solver(&model);
        let (file, span) = (solver.project_file, solver.project_span);
        let reported = solver.error(Code::Ground, "the mistake", file, span, "fix it");
        let accounted = solver.settle(reported.into(), file, span);
        assert_eq!(accounted, Accounted::Reported(reported));
        assert_eq!(solver.diagnostics.len(), 1);
    }

    #[test]
    fn a_remembered_failure_replays_its_proof() {
        let model = model();
        let mut solver = solver(&model);
        let (file, span) = (solver.project_file, solver.project_span);
        let (memo, result) = solver.remember::<u32>(Err(Fault::Overflow.into()), file, span);
        assert_eq!(solver.diagnostics.len(), 1);
        let Memo::Failed(accounted) = memo else {
            panic!("the memo holds the failure");
        };
        assert_eq!(result, Err(Failure::Accounted(accounted)));
        let (memo, result) = solver.remember(Ok(7u32), file, span);
        assert!(matches!(memo, Memo::Done(7)));
        assert_eq!(result, Ok(7));
        assert_eq!(solver.diagnostics.len(), 1);
    }

    #[test]
    fn a_design_that_solves_reports_nothing_internal() {
        let model = model();
        let (design, diagnostics) = crate::solve(&model);
        assert!(design.is_some());
        assert!(diagnostics.iter().all(|d| d.code != Code::Internal));
    }
}
