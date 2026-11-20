//! Field tables of every declaration ("Declarations and their fields").
//!
//! A field that isn't in the table of its declaration is
//! `error[unknown-field]`; a missing required field is `error[missing-field]`.

use stave_lang::ast::TypeName;

use crate::words::WordSet;

/// What a field, argument or setting expects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ValueType {
    /// A value of one of the language's types.
    Type(TypeName),
    /// A word from a closed list.
    Words(WordSet),
    /// A length, or `fit`.
    LengthOrFit,
    /// A length, or the word `through`.
    Depth,
    /// A list whose elements have this type (a single value is not a list).
    ListOf(&'static ValueType),
    /// An item or a list of items: what `show:` and `hide:` take.
    Items,
    /// A position fixed vocabulary that is not an expression: see
    /// [`Special`].
    Special(Special),
}

/// Field values made of calls whose heads are fixed by position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Special {
    /// `insert: [push(...), rotate(...)]`: assembly steps.
    InsertSteps,
    /// `cut_first: into` or `[first, second]`: roles.
    CutFirst,
    /// `adds: extend(...)` or a list of them.
    Adds,
    /// `uses: dowel_pin(...)` or a list of fastener kind values with `count:`.
    Uses,
    /// `needs: [flat_face, before(drill)]`.
    Needs,
    /// `edges: [chamfer(3mm, on: edge(top, front)), roundover(6mm, on:
    /// around(top))]`: a part's edge profiles.
    Edges,
}

/// Whether a field must be written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Requirement {
    /// Always required.
    Required,
    /// Never required.
    Optional,
    /// Required in some cases, which the checker handles by hand; the text
    /// says when.
    Conditional(&'static str),
}

/// One row of a field table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FieldSpec {
    /// The field's name.
    pub name: &'static str,
    /// What it expects.
    pub ty: ValueType,
    /// Whether it must be written.
    pub requirement: Requirement,
    /// `true` if other expressions can read it as a member.
    pub readable: bool,
}

const fn field(
    name: &'static str,
    ty: ValueType,
    requirement: Requirement,
    readable: bool,
) -> FieldSpec {
    FieldSpec {
        name,
        ty,
        requirement,
        readable,
    }
}

use Requirement::{Conditional, Optional, Required};
use ValueType::{Depth, Items, LengthOrFit, ListOf, Type, Words};

const TOOL: ValueType = Type(TypeName::Tool);
const RECIPE: ValueType = Type(TypeName::Recipe);

/// `project "Title" { }`
pub const PROJECT: &[FieldSpec] = &[
    field("goal", Type(TypeName::Text), Optional, false),
    field("units", Words(WordSet::OutputUnit), Optional, false),
    field("resolution", Type(TypeName::Length), Optional, false),
    field("style", Type(TypeName::Text), Optional, false),
    field("tools", ListOf(&TOOL), Optional, false),
    field("methods", ListOf(&RECIPE), Optional, false),
    field("boards", Type(TypeName::Boards), Optional, false),
];

/// `library "Title" { }`
pub const LIBRARY: &[FieldSpec] = &[
    field("about", Type(TypeName::Text), Optional, false),
    field("license", Type(TypeName::Text), Optional, false),
    field("boards", Type(TypeName::Boards), Optional, false),
];

/// `stock name { }`
pub const STOCK: &[FieldSpec] = &[
    field("species", Words(WordSet::Species), Required, true),
    field("form", Words(WordSet::StockForm), Required, true),
    field(
        "thickness",
        Type(TypeName::Length),
        Conditional("yes, except dowel"),
        true,
    ),
    field(
        "diameter",
        Type(TypeName::Length),
        Conditional("dowel only, required"),
        true,
    ),
    field("width", Type(TypeName::Length), Optional, true),
    field("max_width", Type(TypeName::Length), Optional, true),
    field("max_length", Type(TypeName::Length), Optional, true),
];

/// `part name : stock { }` and `makes part name : stock { }`. `place { }`
/// is not a field and is listed separately by the grammar.
pub const PART: &[FieldSpec] = &[
    field("length", LengthOrFit, Required, true),
    field(
        "width",
        LengthOrFit,
        Conditional("yes, unless the stock has width: or is a dowel"),
        true,
    ),
    field("thickness", Type(TypeName::Length), Optional, true),
    field("grain", Words(WordSet::Grain), Optional, false),
    field("shape", Type(TypeName::Shape), Optional, false),
    field("face_side", Words(WordSet::Face), Optional, false),
    field("face_edge", Words(WordSet::Face), Optional, false),
    field("note", Type(TypeName::Text), Optional, false),
    field("boards", Type(TypeName::Boards), Optional, false),
    field("lengths", Type(TypeName::Splices), Optional, false),
    field("loose", Type(TypeName::Bool), Optional, false),
    field("edges", ValueType::Special(Special::Edges), Optional, false),
];

/// Members readable on every part besides its readable fields.
pub const PART_EXTRA_MEMBERS: &[&str] = &["diameter", "stock"];

/// `profile name { }` (the `at <station>: <value>` entries are listed by
/// the grammar separately).
pub const PROFILE: &[FieldSpec] = &[
    field("measure", Words(WordSet::Measure), Required, false),
    field("smooth", Type(TypeName::Bool), Optional, false),
];

/// `scene name "Title" { }` (besides `pose` entries).
pub const SCENE: &[FieldSpec] = &[
    field("show", Items, Optional, false),
    field("hide", Items, Optional, false),
    field("explode", Type(TypeName::Bool), Optional, false),
    field("section", Type(TypeName::Anchor), Optional, false),
];

/// The field entries of a `kind name : joint { }` (besides `roles`,
/// `param`, `require`, `cut` and `makes part`).
pub const JOINT_KIND: &[FieldSpec] = &[
    field(
        "insert",
        ValueType::Special(Special::InsertSteps),
        Optional,
        false,
    ),
    field(
        "cut_first",
        ValueType::Special(Special::CutFirst),
        Optional,
        false,
    ),
    field("adds", ValueType::Special(Special::Adds), Optional, false),
    field("uses", ValueType::Special(Special::Uses), Optional, false),
    field("note", Type(TypeName::Text), Optional, false),
];

/// Settings every joint kind accepts in its call without declaring them.
pub const JOINT_IMPLICIT_SETTINGS: &[FieldSpec] = &[
    field(
        crate::kinds::JointSetting::Glue.word(),
        Type(TypeName::Bool),
        Optional,
        false,
    ),
    field(
        crate::kinds::JointSetting::Methods.word(),
        ListOf(&RECIPE),
        Optional,
        false,
    ),
];

/// The extra argument a fastener kind value takes in a joint's `uses:`.
pub const USES_COUNT: &str = "count";

/// `recipe name : operation { }` (besides `mark { }` and `cut { }`; `cut`
/// is required, `mark` is not).
pub const RECIPE_FIELDS: &[FieldSpec] = &[
    field("tools", ListOf(&TOOL), Required, false),
    field("rank", Type(TypeName::Count), Optional, false),
    field("needs", ValueType::Special(Special::Needs), Optional, false),
    field("check", Type(TypeName::Text), Optional, false),
];

/// Looks up a field in a table.
pub fn find(table: &'static [FieldSpec], name: &str) -> Option<&'static FieldSpec> {
    table.iter().find(|spec| spec.name == name)
}

/// The default `resolution:` of a project, in nanometres (0.5mm).
pub const DEFAULT_RESOLUTION_NM: i64 = 500_000;

/// `Depth` as a value type, for operation arguments.
pub const DEPTH: ValueType = Depth;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{GRAMMAR, between};

    fn names(table: &[FieldSpec]) -> Vec<String> {
        table.iter().map(|spec| spec.name.to_owned()).collect()
    }

    #[test]
    fn project_and_library_fields_match_the_grammar() {
        let project = between(GRAMMAR, "### `project \"Title\" { }`", "### `library");
        let rows: Vec<String> = project
            .lines()
            .filter(|line| line.starts_with("| `"))
            .map(|line| line[3..line[3..].find('`').unwrap() + 3].to_owned())
            .collect();
        assert_eq!(rows, names(PROJECT));
        let library = between(GRAMMAR, "### `library \"Title\" { }`", "### `param");
        let rows: Vec<String> = library
            .lines()
            .filter(|line| line.starts_with("| `"))
            .map(|line| line[3..line[3..].find('`').unwrap() + 3].to_owned())
            .collect();
        assert_eq!(rows, names(LIBRARY));
    }

    #[test]
    fn stock_and_part_fields_match_the_grammar() {
        let stock = between(GRAMMAR, "### `stock name { }`", "### `part");
        let rows: Vec<String> = stock
            .lines()
            .filter(|line| line.starts_with("| `"))
            .map(|line| line[3..line[3..].find('`').unwrap() + 3].to_owned())
            .collect();
        assert_eq!(rows, names(STOCK));
        let part = between(GRAMMAR, "### `part name : stock { }`", "Also readable");
        let rows: Vec<String> = part
            .lines()
            .filter(|line| line.starts_with("| `") && !line.starts_with("| `place"))
            .map(|line| line[3..line[3..].find('`').unwrap() + 3].to_owned())
            .collect();
        assert_eq!(rows, names(PART));
        assert!(
            GRAMMAR.contains("Also readable on every part: `diameter` (round parts only), `stock`")
        );
    }

    #[test]
    fn profile_scene_and_kind_entries_match_the_grammar() {
        let scene = between(GRAMMAR, "### `scene name \"Title\" { }`", "### `require");
        for spec in SCENE {
            assert!(
                scene.contains(&format!("| `{}` |", spec.name)),
                "{}",
                spec.name
            );
        }
        let joint = between(
            GRAMMAR,
            "### `kind name : joint { }`",
            "Every joint kind also",
        );
        for spec in JOINT_KIND {
            assert!(
                joint.contains(&format!("| `{}` |", spec.name)),
                "{}",
                spec.name
            );
        }
        assert!(GRAMMAR.contains("`glue: Bool` (default `true`) and `methods:`"));
        let recipe = between(GRAMMAR, "### `recipe name : operation { }`", "\n---\n");
        for spec in RECIPE_FIELDS {
            assert!(
                recipe.contains(&format!("| `{}` |", spec.name)),
                "{}",
                spec.name
            );
        }
        let profile = between(GRAMMAR, "### `profile name { }`", "Two stations");
        for spec in PROFILE {
            assert!(
                profile.contains(&format!("| `{}` |", spec.name)),
                "{}",
                spec.name
            );
        }
    }

    #[test]
    fn lookups_find_fields() {
        assert_eq!(find(PART, "length").map(|s| s.ty), Some(LengthOrFit));
        assert!(find(PART, "place").is_none());
        assert_eq!(
            find(STOCK, "species").map(|s| s.requirement),
            Some(Required)
        );
    }
}
