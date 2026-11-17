//! The built-in catalog of the `.stave` language (grammar draft 3) as typed
//! data: everything the spec's tables list and the checker must know without
//! reading a `.stave` file.
//!
//! - [`words`]: the closed word lists that bare enum words come from
//!   (namespace 4 of "Names"): species, stock forms, units, measures, ...
//! - [`species`]: wood species with a display colour for the viewer.
//! - [`fields`]: the field tables of every declaration.
//! - [`kinds`]: the base kinds (`joint`, `slide`, `hinge`, `pull`,
//!   `fastener`, `tool`): fixed roles, required and optional entries, the
//!   default mount face of hardware.
//! - [`operations`]: cut operations with their arguments, the face selectors
//!   they may be written under, and their features and measurements.
//! - [`recipes`]: the recipe step vocabulary and `needs:`.
//! - [`placement`]: placement terms, spacing values, made-part terms and
//!   assembly steps (namespace 3: call heads fixed by position).
//! - [`functions`]: built-in function signatures, and the points and
//!   shapes written inside a hardware kind's members.
//!
//! Joint, hardware and tool kinds themselves are *not* built in: they are
//! written in `.stave` in the standard library, mechanisms included. The tests at the bottom of
//! each module read `docs/grammar.md` and fail if the catalog and the spec
//! drift apart.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod fields;
pub mod functions;
pub mod kinds;
pub mod operations;
pub mod placement;
pub mod recipes;
pub mod species;
pub mod words;

pub use stave_lang::ast::{BaseKind, CutOperation, Operation, TypeName};

/// The standard modules whose public items form the prelude, in the order
/// the grammar lists them (`std:joinery`, `std:hardware`, `std:tools`).
pub const PRELUDE: [&str; 3] = ["joinery", "hardware", "tools"];

/// The scheme of a standard-library import source: `std:joinery`.
pub const STD_SCHEME: &str = "std:";

/// The name of the scene every project has without declaring it.
pub const REST_SCENE: &str = "rest";

/// The title of the built-in [`REST_SCENE`].
pub const REST_SCENE_TITLE: &str = "As built";

/// Contact tolerance: two surfaces closer than this touch (0.05mm, in
/// nanometres).
pub const CONTACT_TOLERANCE_NM: i64 = 50_000;

/// Default `rank:` of a recipe.
pub const DEFAULT_RECIPE_RANK: u32 = 100;

#[cfg(test)]
pub(crate) mod spec {
    //! Helpers for reading tables out of `docs/grammar.md` in tests.

    /// The grammar document.
    pub const GRAMMAR: &str = include_str!("../../../docs/grammar.md");

    /// The text between `start` and the next `end` after it.
    pub fn between<'a>(text: &'a str, start: &str, end: &str) -> &'a str {
        let from = text
            .find(start)
            .unwrap_or_else(|| panic!("grammar.md lost {start:?}"));
        let rest = &text[from + start.len()..];
        &rest[..rest
            .find(end)
            .unwrap_or_else(|| panic!("grammar.md lost {end:?} after {start:?}"))]
    }

    /// The rows of the table whose header line is `header`.
    pub fn table_rows(header: &str) -> Vec<Vec<String>> {
        let body = between(GRAMMAR, header, "\n\n");
        body.lines()
            .skip(1) // the |---| line
            .map(|row| {
                let cells: Vec<String> = row.split('|').map(str::trim).map(str::to_owned).collect();
                cells[1..cells.len() - 1].to_vec()
            })
            .collect()
    }

    /// Backticked words in a cell: "`a`, `b` (x)" -> ["a", "b"].
    pub fn ticked(cell: &str) -> Vec<String> {
        cell.split('`')
            .skip(1)
            .step_by(2)
            .map(str::to_owned)
            .collect()
    }

    /// The name of a call written in a cell: "`notch(depth:)`" -> "notch".
    pub fn call_name(cell: &str) -> String {
        let inner = cell.trim_matches('`');
        inner[..inner.find('(').unwrap_or(inner.len())].to_owned()
    }

    /// The argument names of a call written in a cell:
    /// "`slot(width:, depth:)`" -> ["width", "depth"].
    pub fn call_arguments(cell: &str) -> Vec<String> {
        let inner = cell.trim_matches('`');
        let Some(open) = inner.find('(') else {
            return Vec::new();
        };
        let close = inner.rfind(')').unwrap_or(inner.len());
        inner[open + 1..close]
            .split(',')
            .filter_map(|argument| {
                let argument = argument.trim();
                argument.find(':').map(|colon| argument[..colon].to_owned())
            })
            .collect()
    }
}
