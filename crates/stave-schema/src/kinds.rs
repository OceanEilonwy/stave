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

/// A named piece of a hardware kind without a body, declared with
/// `member name` and a doc comment (grammar, "Hardware members"), kept for
/// the viewer's list until every member has a body. The words are a closed
/// vocabulary per base: a viewer knows which mesh each one is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MemberName {
    /// A cup hinge's steel cup, sunk in the door, with the boss its arm
    /// and links turn on.
    Cup,
    /// A cup hinge's mounting plate, screwed to the cabinet side.
    Plate,
    /// A cup hinge's arm, clipped onto the plate.
    Arm,
    /// A cup hinge's two links between the arm and the cup's boss.
    Links,
    /// A butt hinge's two leaves.
    Leaves,
    /// A butt hinge's pin, the line it turns about.
    Pin,
    /// A slide's member screwed to the drawer.
    DrawerMember,
    /// A slide's member screwed to the cabinet.
    CabinetMember,
    /// A knob: the part you hold.
    Knob,
    /// A bar pull's bar: the part you hold.
    Bar,
    /// A knob's one screw, from behind through the door or drawer front.
    Screw,
    /// A bar pull's two screws, from behind.
    Screws,
}

impl MemberName {
    /// Every member word.
    pub const ALL: [MemberName; 12] = [
        MemberName::Cup,
        MemberName::Plate,
        MemberName::Arm,
        MemberName::Links,
        MemberName::Leaves,
        MemberName::Pin,
        MemberName::DrawerMember,
        MemberName::CabinetMember,
        MemberName::Knob,
        MemberName::Bar,
        MemberName::Screw,
        MemberName::Screws,
    ];

    /// The word as written: `cup`, `drawer_member`.
    pub fn word(self) -> &'static str {
        match self {
            MemberName::Cup => "cup",
            MemberName::Plate => "plate",
            MemberName::Arm => "arm",
            MemberName::Links => "links",
            MemberName::Leaves => "leaves",
            MemberName::Pin => "pin",
            MemberName::DrawerMember => "drawer_member",
            MemberName::CabinetMember => "cabinet_member",
            MemberName::Knob => "knob",
            MemberName::Bar => "bar",
            MemberName::Screw => "screw",
            MemberName::Screws => "screws",
        }
    }

    /// The member a word names.
    pub fn from_word(word: &str) -> Option<MemberName> {
        MemberName::ALL.into_iter().find(|m| m.word() == word)
    }
}

/// The members a cup hinge may declare, in the order a viewer numbers
/// them: from the cabinet side to the door.
pub const CUP_HINGE_MEMBERS: [MemberName; 4] = [
    MemberName::Plate,
    MemberName::Arm,
    MemberName::Links,
    MemberName::Cup,
];

/// The members a butt hinge may declare.
pub const BUTT_HINGE_MEMBERS: [MemberName; 2] = [MemberName::Leaves, MemberName::Pin];

/// The members a slide may declare.
pub const SLIDE_MEMBERS: [MemberName; 2] = [MemberName::CabinetMember, MemberName::DrawerMember];

/// The members a pull may declare: a knob and its screw, or a bar and
/// its screws.
pub const PULL_MEMBERS: [MemberName; 4] = [
    MemberName::Knob,
    MemberName::Bar,
    MemberName::Screw,
    MemberName::Screws,
];

/// The members a hinge may declare without a body: a cup hinge's and a
/// butt hinge's.
pub const HINGE_MEMBERS: [MemberName; 6] = [
    MemberName::Plate,
    MemberName::Arm,
    MemberName::Links,
    MemberName::Cup,
    MemberName::Leaves,
    MemberName::Pin,
];

/// The member words a kind of this base may declare without a body.
/// Fasteners, tools and joints have none.
pub fn members_of(base: BaseKind) -> &'static [MemberName] {
    match base {
        BaseKind::Hinge => &HINGE_MEMBERS,
        BaseKind::Slide => &SLIDE_MEMBERS,
        BaseKind::Pull => &PULL_MEMBERS,
        BaseKind::Fastener | BaseKind::Tool | BaseKind::Joint => &[],
    }
}

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

    #[test]
    fn hinge_member_words_are_a_cup_hinges_and_a_butt_hinges() {
        let mut both: Vec<MemberName> = CUP_HINGE_MEMBERS
            .iter()
            .chain(&BUTT_HINGE_MEMBERS)
            .copied()
            .collect();
        both.sort();
        let mut hinge = members_of(BaseKind::Hinge).to_vec();
        hinge.sort();
        assert_eq!(hinge, both);
    }
}
