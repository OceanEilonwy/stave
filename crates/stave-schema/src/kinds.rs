//! Base kinds: what `kind name : <base>` can declare.
//!
//! A joint kind declares its own roles (default `piece: Parts`,
//! `into: Parts`); hardware bases have fixed roles. A hardware kind's
//! required entries may be written as fields or declared as params of the
//! same name.

use stave_lang::ast::{BaseKind, TypeName};
use stave_lang::placement::Face;

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
    /// The role type for a type name written in a `roles { }` block. Only
    /// `Part` and `Parts` may be written there.
    pub fn from_type(ty: TypeName) -> Option<RoleType> {
        match ty {
            TypeName::Part => Some(RoleType::Part),
            TypeName::Parts => Some(RoleType::Parts),
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

/// How a piece of hardware picks the face of its `piece` it is mounted on
/// when the call has no `face:`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MountRule {
    /// The `left` or `right` face of the piece, whichever is nearer the
    /// same-named face of the `in` region (slides).
    NearerRegionSide,
    /// Always this face (`back` for hinges, `front` for pulls).
    Face(Face),
}

/// How many axes of the mount face a hardware `place { }` sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OpenAxes {
    /// None: the base fixes everything; a `place` is `error[axis-set-twice]`.
    None,
    /// The one axis along the piece's length.
    AlongPieceLength,
    /// Both axes of the mount face.
    Both,
}

/// Everything the checker knows about a base kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BaseSpec {
    /// The base.
    pub base: BaseKind,
    /// Fixed roles; empty for joints (which declare their own), fasteners
    /// and tools.
    pub roles: &'static [RoleSpec],
    /// Entries the kind must have (as a field or a param).
    pub required: &'static [FieldSpec],
    /// Entries the kind may have.
    pub optional: &'static [FieldSpec],
    /// The default mount face, for mounted hardware.
    pub mount: Option<MountRule>,
    /// What a hardware `place { }` may set, for mounted hardware.
    pub open_axes: Option<OpenAxes>,
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
    roles: &[],
    required: &[],
    optional: &[],
    mount: None,
    open_axes: None,
};

const SLIDE: BaseSpec = BaseSpec {
    base: BaseKind::Slide,
    roles: &[
        RoleSpec {
            name: "piece",
            ty: RoleType::Part,
        },
        RoleSpec {
            name: "in",
            ty: RoleType::Region,
        },
    ],
    required: &[
        entry("length", LENGTH, Required),
        entry("clearance", LENGTH, Required),
        entry("travel", LENGTH, Required),
        entry("envelope", Type(TypeName::Envelope), Required),
    ],
    optional: &HARDWARE_OPTIONAL,
    mount: Some(MountRule::NearerRegionSide),
    open_axes: Some(OpenAxes::None),
};

const HINGE: BaseSpec = BaseSpec {
    base: BaseKind::Hinge,
    roles: &[
        RoleSpec {
            name: "piece",
            ty: RoleType::Part,
        },
        RoleSpec {
            name: "on",
            ty: RoleType::Part,
        },
    ],
    required: &[
        entry("opening_angle", Type(TypeName::Angle), Required),
        entry("cup_diameter", LENGTH, Required),
        entry("cup_depth", LENGTH, Required),
        entry("cup_inset", LENGTH, Required),
        entry("envelope", Type(TypeName::Envelope), Required),
    ],
    optional: &HARDWARE_OPTIONAL,
    mount: Some(MountRule::Face(Face::Back)),
    open_axes: Some(OpenAxes::AlongPieceLength),
};

const PULL: BaseSpec = BaseSpec {
    base: BaseKind::Pull,
    roles: &[RoleSpec {
        name: "piece",
        ty: RoleType::Part,
    }],
    required: &[entry("envelope", Type(TypeName::Envelope), Required)],
    optional: &[
        entry("centres", LENGTH, Optional),
        entry("diameter", LENGTH, Optional),
        entry("screw_diameter", LENGTH, Optional),
        entry("model", Type(TypeName::Model), Optional),
        entry("license", Type(TypeName::Text), Optional),
    ],
    mount: Some(MountRule::Face(Face::Front)),
    open_axes: Some(OpenAxes::Both),
};

const FASTENER: BaseSpec = BaseSpec {
    base: BaseKind::Fastener,
    roles: &[],
    required: &[
        entry("diameter", LENGTH, Required),
        entry("length", LENGTH, Required),
    ],
    optional: &[
        entry("envelope", Type(TypeName::Envelope), Optional),
        entry("model", Type(TypeName::Model), Optional),
        entry("license", Type(TypeName::Text), Optional),
    ],
    mount: None,
    open_axes: None,
};

const TOOL: BaseSpec = BaseSpec {
    base: BaseKind::Tool,
    roles: &[],
    required: &[],
    optional: &[
        entry("title", Type(TypeName::Text), Optional),
        entry("powered", Type(TypeName::Bool), Optional),
    ],
    mount: None,
    open_axes: None,
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

/// The default `screw_diameter` of a pull, in nanometres (4mm).
pub const DEFAULT_SCREW_DIAMETER_NM: i64 = 4_000_000;

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
        // "`length`, `clearance`, `travel`: `Length`; `envelope`: `Envelope`"
        cell.split(';')
            .flat_map(|group| {
                let names = group.split(':').next().unwrap_or("");
                ticked(names)
            })
            .collect()
    }

    #[test]
    fn hardware_bases_match_the_grammar() {
        let rows = table_rows("| Base | Roles (fixed) | Required | Optional |\n");
        assert_eq!(rows.len(), 5);
        for row in rows {
            let base_word = row[0].trim_matches('`');
            let spec = base(BaseKind::from_word(base_word).unwrap());
            let roles: Vec<String> = ticked(&row[1])
                .into_iter()
                .map(|role| role.split(':').next().unwrap().to_owned())
                .collect();
            assert_eq!(
                roles,
                spec.roles
                    .iter()
                    .map(|r| r.name.to_owned())
                    .collect::<Vec<_>>(),
                "{base_word}"
            );
            let required: Vec<String> = word_list(&row[2])
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
            let optional: Vec<String> = ticked(&row[3])
                .into_iter()
                .filter(|word| {
                    word.chars().next().is_some_and(char::is_lowercase)
                        && !word.contains(' ')
                        && word != "4mm"
                        && word != "false"
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

    #[test]
    fn mount_faces_match_the_grammar() {
        let rows = table_rows(
            "| Base | Mount face (default) | Fixed by the base | Set by `place { }` |\n",
        );
        let words: Vec<String> = rows
            .iter()
            .map(|r| r[0].trim_matches('`').to_owned())
            .collect();
        assert_eq!(words, ["slide", "hinge", "pull"]);
        assert_eq!(
            base(BaseKind::Hinge).mount,
            Some(MountRule::Face(Face::Back))
        );
        assert!(rows[1][1].contains("`back`"));
        assert!(rows[2][1].contains("`front`"));
        assert_eq!(base(BaseKind::Slide).open_axes, Some(OpenAxes::None));
        assert!(base(BaseKind::Joint).mount.is_none());
    }

    #[test]
    fn joint_defaults_match_the_grammar() {
        let row = between(GRAMMAR, "| `roles { }` | ", " |\n");
        assert!(row.contains("default `piece: Parts`, `into: Parts`"));
        assert_eq!(JOINT_DEFAULT_ROLES[0].name, "piece");
        assert!(GRAMMAR.contains(
            "A kind can't declare a role, param or made part\nnamed `glue` or `methods`."
        ));
        assert!(GRAMMAR.contains("`screw_diameter` (default `4mm`)"));
    }
}
