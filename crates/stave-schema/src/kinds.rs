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
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "picks out only the variants it names: any other, a new one included, is not one of them"
    )]
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

/// An entry a base kind fixes (grammar, "Kinds"): the closed vocabulary of
/// fastener, tool and hardware entries, written as a field or declared as a
/// param of the same name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BaseField {
    /// `diameter`: a fastener's diameter.
    Diameter,
    /// `length`: a fastener's length.
    Length,
    /// `envelope`: a fastener's envelope.
    Envelope,
    /// `model`: a 3D model of the hardware or fastener.
    Model,
    /// `moving`: which node of a hardware model moves.
    Moving,
    /// `license`: the licence of the model.
    License,
    /// `species`: the wood a fastener (a dowel, a biscuit) is made of.
    Species,
    /// `title`: a tool's name for people.
    Title,
    /// `powered`: `true` for a power tool.
    Powered,
}

impl BaseField {
    /// Every base field.
    pub const ALL: [BaseField; 9] = [
        BaseField::Diameter,
        BaseField::Length,
        BaseField::Envelope,
        BaseField::Model,
        BaseField::Moving,
        BaseField::License,
        BaseField::Species,
        BaseField::Title,
        BaseField::Powered,
    ];

    /// The entry's name as written.
    pub const fn word(self) -> &'static str {
        match self {
            BaseField::Diameter => "diameter",
            BaseField::Length => "length",
            BaseField::Envelope => "envelope",
            BaseField::Model => "model",
            BaseField::Moving => "moving",
            BaseField::License => "license",
            BaseField::Species => "species",
            BaseField::Title => "title",
            BaseField::Powered => "powered",
        }
    }

    /// The base field with this name, if it is one.
    pub fn from_word(word: &str) -> Option<BaseField> {
        BaseField::ALL
            .into_iter()
            .find(|field| field.word() == word)
    }
}

const fn entry(field: BaseField, ty: ValueType, requirement: Requirement) -> FieldSpec {
    FieldSpec {
        name: field.word(),
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
    entry(BaseField::Model, Type(TypeName::Model), Optional),
    entry(BaseField::Moving, Type(TypeName::Text), Optional),
    entry(BaseField::License, Type(TypeName::Text), Optional),
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

const SUPPORT: BaseSpec = BaseSpec {
    base: BaseKind::Support,
    required: &[],
    optional: &HARDWARE_OPTIONAL,
};

const FASTENER: BaseSpec = BaseSpec {
    base: BaseKind::Fastener,
    required: &[
        entry(BaseField::Diameter, LENGTH, Required),
        entry(BaseField::Length, LENGTH, Required),
    ],
    optional: &[
        entry(BaseField::Envelope, Type(TypeName::Envelope), Optional),
        entry(BaseField::Model, Type(TypeName::Model), Optional),
        entry(BaseField::License, Type(TypeName::Text), Optional),
        entry(
            BaseField::Species,
            ValueType::Words(crate::words::WordSet::Species),
            Optional,
        ),
    ],
};

const TOOL: BaseSpec = BaseSpec {
    base: BaseKind::Tool,
    required: &[],
    optional: &[
        entry(BaseField::Title, Type(TypeName::Text), Optional),
        entry(BaseField::Powered, Type(TypeName::Bool), Optional),
    ],
};

/// The catalog entry of a base kind.
pub fn base(base: BaseKind) -> &'static BaseSpec {
    match base {
        BaseKind::Joint => &JOINT,
        BaseKind::Slide => &SLIDE,
        BaseKind::Hinge => &HINGE,
        BaseKind::Pull => &PULL,
        BaseKind::Support => &SUPPORT,
        BaseKind::Fastener => &FASTENER,
        BaseKind::Tool => &TOOL,
    }
}

/// The setting every hardware call accepts to name its mount face.
pub const FACE_SETTING: &str = "face";

/// Names a joint kind can't give a role, param or made part, because every
/// joint call accepts them.
pub const RESERVED_JOINT_SETTINGS: [&str; 2] =
    [JointSetting::Glue.word(), JointSetting::Methods.word()];

/// A setting every joint kind's call accepts without declaring it
/// (grammar, "Joints"), apart from the kind's own params.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum JointSetting {
    /// `glue: false`: the joint is not glued.
    Glue,
    /// `methods: [recipe, ...]`: the recipes the joint may be made with.
    Methods,
}

impl JointSetting {
    /// Both settings.
    pub const ALL: [JointSetting; 2] = [JointSetting::Glue, JointSetting::Methods];

    /// The setting's name as written.
    pub const fn word(self) -> &'static str {
        match self {
            JointSetting::Glue => "glue",
            JointSetting::Methods => "methods",
        }
    }

    /// The joint setting with this name, if it is one.
    pub fn from_word(word: &str) -> Option<JointSetting> {
        JointSetting::ALL
            .into_iter()
            .find(|setting| setting.word() == word)
    }
}

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

    /// Every entry a base fixes is a [`BaseField`], and every base field is
    /// an entry of some base.
    #[test]
    fn base_fields_are_the_bases_entries() {
        let bases = [
            BaseKind::Joint,
            BaseKind::Slide,
            BaseKind::Hinge,
            BaseKind::Pull,
            BaseKind::Support,
            BaseKind::Fastener,
            BaseKind::Tool,
        ];
        for spec in bases.iter().flat_map(|b| {
            let spec = base(*b);
            spec.required.iter().chain(spec.optional)
        }) {
            assert!(BaseField::from_word(spec.name).is_some(), "{}", spec.name);
        }
        for field in BaseField::ALL {
            assert!(
                bases.iter().any(|b| base(*b).entry(field.word()).is_some()),
                "{}",
                field.word()
            );
            assert_eq!(BaseField::from_word(field.word()), Some(field));
        }
        for setting in JointSetting::ALL {
            assert_eq!(JointSetting::from_word(setting.word()), Some(setting));
        }
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
        for base_kind in [
            BaseKind::Slide,
            BaseKind::Hinge,
            BaseKind::Pull,
            BaseKind::Support,
        ] {
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
