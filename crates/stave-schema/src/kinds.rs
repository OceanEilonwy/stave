//! Base kinds: what `kind name : <base>` can declare.
//!
//! A joint kind declares its own roles (default `piece: Parts`,
//! `into: Parts`); a slide, hinge or pull kind declares its own roles too,
//! and its catalogue data is any field it likes. Fasteners and tools have a
//! fixed set of entries, which may be written as fields or declared as
//! params of the same name.

use stave_lang::ast::{BaseKind, TypeName};

use crate::fields::{FieldSpec, Requirement, ValueType};

/// What a role binds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RoleType {
    /// Exactly one part.
    Part,
    /// One part or a list of parts.
    Parts,
    /// A region (the `in` role of a slide).
    Region,
}

impl RoleType {
    /// The role type for a type name written in a `roles { }` block:
    /// `Part`, `Parts` or `Region`.
    pub fn from_type(ty: TypeName) -> Option<RoleType> {
        match ty {
            TypeName::Part => Some(RoleType::Part),
            TypeName::Parts => Some(RoleType::Parts),
            TypeName::Region => Some(RoleType::Region),
            _ => None,
        }
    }
}

/// A fixed role of a base kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RoleSpec {
    /// The role's name.
    pub name: &'static str,
    /// What it binds.
    pub ty: RoleType,
}

/// Everything the checker knows about a base kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BaseSpec {
    /// The base.
    pub base: BaseKind,
    /// Entries the kind must have (as a field or a param).
    pub required: &'static [FieldSpec],
    /// Entries the kind may have, with their types. A slide, hinge or pull
    /// kind may also have any other field (its catalogue data).
    pub optional: &'static [FieldSpec],
}

impl BaseSpec {
    /// The entry with this name, required or optional.
    pub fn entry(&self, name: &str) -> Option<&'static FieldSpec> {
        self.required
            .iter()
            .chain(self.optional)
            .find(|spec| spec.name == name)
    }
}

const fn entry(name: &'static str, ty: ValueType, requirement: Requirement) -> FieldSpec {
    FieldSpec {
        name,
        ty,
        requirement,
        readable: true,
    }
}

use Requirement::{Optional, Required};
use ValueType::Type;

const LENGTH: ValueType = Type(TypeName::Length);

/// The default roles of a joint kind without a `roles { }` block.
pub const JOINT_DEFAULT_ROLES: &[RoleSpec] = &[
    RoleSpec {
        name: "piece",
        ty: RoleType::Parts,
    },
    RoleSpec {
        name: "into",
        ty: RoleType::Parts,
    },
];

const HARDWARE_OPTIONAL: [FieldSpec; 3] = [
    entry("model", Type(TypeName::Model), Optional),
    entry("moving", Type(TypeName::Text), Optional),
    entry("license", Type(TypeName::Text), Optional),
];

const JOINT: BaseSpec = BaseSpec {
    base: BaseKind::Joint,
    required: &[],
    optional: &[],
};

const SLIDE: BaseSpec = BaseSpec {
    base: BaseKind::Slide,
    required: &[],
    optional: &HARDWARE_OPTIONAL,
};

const HINGE: BaseSpec = BaseSpec {
    base: BaseKind::Hinge,
    required: &[],
    optional: &HARDWARE_OPTIONAL,
};

const PULL: BaseSpec = BaseSpec {
    base: BaseKind::Pull,
    required: &[],
    optional: &HARDWARE_OPTIONAL,
};

const FASTENER: BaseSpec = BaseSpec {
    base: BaseKind::Fastener,
    required: &[
        entry("diameter", LENGTH, Required),
        entry("length", LENGTH, Required),
    ],
    optional: &[
        entry("envelope", Type(TypeName::Envelope), Optional),
        entry("model", Type(TypeName::Model), Optional),
        entry("license", Type(TypeName::Text), Optional),
        entry(
            "species",
            ValueType::Words(crate::words::WordSet::Species),
            Optional,
        ),
    ],
};

const TOOL: BaseSpec = BaseSpec {
    base: BaseKind::Tool,
    required: &[],
    optional: &[
        entry("title", Type(TypeName::Text), Optional),
        entry("powered", Type(TypeName::Bool), Optional),
    ],
};

/// The catalog entry of a base kind.
pub fn base(base: BaseKind) -> &'static BaseSpec {
    match base {
        BaseKind::Joint => &JOINT,
        BaseKind::Slide => &SLIDE,
        BaseKind::Hinge => &HINGE,
        BaseKind::Pull => &PULL,
        BaseKind::Fastener => &FASTENER,
        BaseKind::Tool => &TOOL,
    }
}

/// The setting every hardware call accepts to name its mount face.
pub const FACE_SETTING: &str = "face";

/// Names a joint kind can't give a role, param or made part, because every
/// joint call accepts them.
pub const RESERVED_JOINT_SETTINGS: [&str; 2] = ["glue", "methods"];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{GRAMMAR, between, table_rows, ticked};

    fn word_list(cell: &str) -> Vec<String> {
        // "`diameter`, `length`: `Length`"
        cell.split(';')
            .flat_map(|group| {
                let names = group.split(':').next().unwrap_or("");
                ticked(names)
            })
            .collect()
    }

    /// The fastener and tool rows: the two bases whose entries are fixed.
    #[test]
    fn fixed_hardware_entries_match_the_grammar() {
        let rows = table_rows("| Base | Required | Optional |\n");
        assert_eq!(rows.len(), 2);
        for row in rows {
            let base_word = row[0].trim_matches('`');
            let spec = base(BaseKind::from_word(base_word).unwrap());
            let required: Vec<String> = word_list(&row[1])
                .into_iter()
                .filter(|word| word.chars().next().is_some_and(char::is_lowercase))
                .collect();
            assert_eq!(
                required,
                spec.required
                    .iter()
                    .map(|f| f.name.to_owned())
                    .collect::<Vec<_>>(),
                "{base_word}"
            );
            let optional: Vec<String> = ticked(&row[2])
                .into_iter()
                .filter(|word| {
                    word.chars().next().is_some_and(char::is_lowercase) && word != "false"
                })
                .collect();
            assert_eq!(
                optional,
                spec.optional
                    .iter()
                    .map(|f| f.name.to_owned())
                    .collect::<Vec<_>>(),
                "{base_word}"
            );
        }
    }

    /// A slide, hinge or pull kind has no fixed entries but the ones every
    /// piece of hardware may have: the grammar's table of what such a kind
    /// may contain lists them.
    #[test]
    fn mounted_hardware_entries_match_the_grammar() {
        let rows = table_rows("| Entry | Required | Meaning |\n");
        let entries: Vec<String> = rows.iter().flat_map(|row| ticked(&row[0])).collect();
        for base_kind in [BaseKind::Slide, BaseKind::Hinge, BaseKind::Pull] {
            let spec = base(base_kind);
            assert!(spec.required.is_empty());
            for field in spec.optional {
                assert!(entries.iter().any(|e| e == field.name), "{}", field.name);
            }
        }
        for word in ["roles { }", "mount { }", "member", "mechanism { }", "uses"] {
            assert!(entries.iter().any(|e| e == word), "{word}");
        }
    }

    #[test]
    fn joint_defaults_match_the_grammar() {
        let row = between(GRAMMAR, "| `roles { }` | ", " |\n");
        assert!(row.contains("default `piece: Parts`, `into: Parts`"));
        assert_eq!(JOINT_DEFAULT_ROLES[0].name, "piece");
        assert!(GRAMMAR.contains(
            "A kind can't declare a role, param or made part\nnamed `glue` or `methods`."
        ));
    }
}
