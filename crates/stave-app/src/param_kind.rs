//! Turning a plain param into a solved one, and back (`docs/editing.md`,
//! "Grips"; grammar, "Smart edits"): the patch that replaces the param's
//! declaration with the other kind.
//!
//! ```text
//! param overall_width = 900mm                 ──make solved, 600mm..1200mm──▶
//! solved param overall_width = 900mm within 600mm..1200mm
//!                                              ──make plain──▶ param overall_width = 900mm within 600mm..1200mm
//! ```
//!
//! A solved param needs a `within` range, which the caller gives (the
//! user types it); making it plain again keeps the range, which still
//! limits the param. A `locked param` is never changed by an edit, so it
//! is refused: unlock it in the text first. Only the project's own
//! top-level params can be changed: a library's are read-only, and a
//! component's params are set per assembly.

use stave_lang::ast::{DesignItem, ItemKind, ParamMark};

use crate::candidates::EditScope;
use crate::repeat::{GeneratedPatch, PatchBuildError, find};

/// The kind a param is to become.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NewParamKind {
    /// A `solved param` the solver may change within this range: the two
    /// ends as `.stave` text (`600mm`, `1200mm`).
    Solved {
        /// The lowest value.
        low: String,
        /// The highest value.
        high: String,
    },
    /// A plain `param`, keeping any range it has.
    Plain,
}

/// Why a param's kind can't be changed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParamKindError {
    /// The project has no top-level param by that name.
    #[error("there is no param named '{0}' in this project")]
    UnknownParam(String),
    /// The param is locked.
    #[error(
        "'{0}' is a locked param, which no edit changes; take 'locked' off it in the text first"
    )]
    Locked(String),
    /// The param has no default, so there is nothing to solve from.
    #[error("'{0}' has no default value, so it can't be solved for")]
    NoDefault(String),
    /// The param is already of that kind.
    #[error("'{name}' is already a {kind} param")]
    Already {
        /// The param.
        name: String,
        /// Its kind, in words.
        kind: &'static str,
    },
    /// The source text couldn't be read back.
    #[error("the text of '{0}' couldn't be read")]
    Unreadable(String),
    /// The written patch didn't parse.
    #[error(transparent)]
    Build(#[from] PatchBuildError),
}

/// The patch that gives the project's param `name` another kind (see the
/// module's notes): its declaration written again with `solved` before
/// `param` and the range given, or without `solved`.
///
/// # Errors
///
/// See [`ParamKindError`].
pub fn param_kind_patch(
    scope: &EditScope<'_>,
    name: &str,
    kind: &NewParamKind,
) -> Result<GeneratedPatch, ParamKindError> {
    let unknown = || ParamKindError::UnknownParam(name.to_owned());
    let found = find(scope, name).ok_or_else(unknown)?;
    let ItemKind::Design(DesignItem::Param(param)) = &found.item.kind else {
        return Err(unknown());
    };
    let unreadable = || ParamKindError::Unreadable(name.to_owned());
    match (param.mark.as_ref().map(|mark| mark.value), kind) {
        (Some(ParamMark::Locked), _) => return Err(ParamKindError::Locked(name.to_owned())),
        (None, NewParamKind::Plain) => {
            return Err(ParamKindError::Already {
                name: name.to_owned(),
                kind: "plain",
            });
        }
        (Some(ParamMark::Solved), NewParamKind::Plain) | (_, NewParamKind::Solved { .. }) => {}
    }
    let default = param
        .spec
        .default()
        .ok_or_else(|| ParamKindError::NoDefault(name.to_owned()))?;
    let default = found.slice(default.span).ok_or_else(unreadable)?;
    let ty = match param.spec.ty() {
        Some(ty) => format!(" : {}", found.slice(ty.span()).ok_or_else(unreadable)?),
        None => String::new(),
    };
    let within = match (kind, &param.within) {
        (NewParamKind::Solved { low, high }, _) => format!(" within {low}..{high}"),
        (NewParamKind::Plain, Some(domain)) => {
            let span = match domain {
                stave_lang::ast::Domain::Range { span, .. }
                | stave_lang::ast::Domain::OneOf { span, .. } => *span,
            };
            format!(" within {}", found.slice(span).ok_or_else(unreadable)?)
        }
        (NewParamKind::Plain, None) => String::new(),
    };
    let private = if found.item.private.is_some() {
        "private "
    } else {
        ""
    };
    let (mark, summary) = match kind {
        NewParamKind::Solved { .. } => ("solved ", format!("Let the solver change {name}")),
        NewParamKind::Plain => ("", format!("Make {name} a plain param")),
    };
    let op = format!("replace {private}{mark}param {name}{ty} = {default}{within}");
    Ok(GeneratedPatch::new(&summary, &[op])?)
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::atomic::AtomicBool;

    use super::*;
    use crate::repeat::test_support::{clean_base, commit, small_project};
    use crate::scratch::EditBase;

    #[test]
    fn a_plain_param_becomes_solved_and_plain_again() {
        let (folder, files) = small_project(&[]);
        let base = clean_base(&folder, &files);
        let scope = EditScope::new(base.opened()).unwrap();
        let solved = NewParamKind::Solved {
            low: "300mm".to_owned(),
            high: "500mm".to_owned(),
        };
        let patch = param_kind_patch(&scope, "width", &solved).unwrap();
        assert!(
            patch
                .text
                .contains("replace solved param width = 400mm within 300mm..500mm"),
            "{}",
            patch.text
        );
        let tried = base
            .try_patch("kind.stave", &patch.parse, &AtomicBool::new(false))
            .unwrap();
        assert!(tried.fits(), "{:?}", tried.shown());
        let mut files = files;
        commit(&mut files, &tried);
        let text = crate::repeat::test_support::text(&files, &folder, "case.stave");
        // The formatter lines the params up.
        assert!(
            text.contains("solved param width  = 400mm within 300mm..500mm\n       param height"),
            "{text}"
        );

        // And back: the range stays.
        let base = EditBase::new(Path::new(&folder), &files).unwrap();
        let scope = EditScope::new(base.opened()).unwrap();
        let plain = param_kind_patch(&scope, "width", &NewParamKind::Plain).unwrap();
        assert!(
            plain
                .text
                .contains("replace param width = 400mm within 300mm..500mm"),
            "{}",
            plain.text
        );
        let tried = base
            .try_patch("kind.stave", &plain.parse, &AtomicBool::new(false))
            .unwrap();
        assert!(tried.fits(), "{:?}", tried.shown());

        // Refusals: an unknown name, a plain param made plain.
        assert_eq!(
            param_kind_patch(&scope, "nothing", &NewParamKind::Plain).unwrap_err(),
            ParamKindError::UnknownParam("nothing".to_owned())
        );
        let (folder, files) = small_project(&["locked param reveal = 2mm"]);
        let base = clean_base(&folder, &files);
        let scope = EditScope::new(base.opened()).unwrap();
        assert!(matches!(
            param_kind_patch(&scope, "height", &NewParamKind::Plain),
            Err(ParamKindError::Already { .. })
        ));
        assert!(matches!(
            param_kind_patch(&scope, "reveal", &solved),
            Err(ParamKindError::Locked(_))
        ));
    }
}
