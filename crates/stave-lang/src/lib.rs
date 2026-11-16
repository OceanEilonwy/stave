//! The `.stave` project format.
//!
//! - [`ast`]: the syntax tree produced by the parser. Every node carries a
//!   [`ast::Span`] so errors can point at source text.
//! - [`units`]: exact lengths, angles and fractions, and conversion from literals.
//! - [`placement`]: the typed placement vocabulary that `place { }` blocks
//!   lower into after name resolution. It has no way to express a coordinate.
//!
//! See `docs/grammar.md` for the grammar.

pub mod ast;
pub mod placement;
pub mod units;

/// Reserved words. Face and direction names such as `top`, `left` or `up` are
/// deliberately not reserved, so they can be used as part names and labels.
/// `as` and `in` are contextual (special only in imports and `add ... in`), so
/// they are not listed.
pub const KEYWORDS: &[&str] = &[
    "stave",
    "project",
    "library",
    "import",
    "private",
    "kind",
    "recipe",
    "param",
    "let",
    "region",
    "stock",
    "part",
    "joint",
    "hardware",
    "assembly",
    "profile",
    "component",
    "scene",
    "require",
    "patch",
    "add",
    "set",
    "remove",
    "place",
    "pose",
    "at",
    "within",
    "locked",
    "fit",
    "true",
    "false",
];

/// Current version written in the `stave <version>` header.
pub const FORMAT_VERSION: u32 = 1;

pub fn is_keyword(word: &str) -> bool {
    KEYWORDS.contains(&word)
}
