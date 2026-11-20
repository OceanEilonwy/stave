//! Built-in functions ("Built-in functions"): looked up only when a bare
//! name is called and no kind with that name is in scope.

use stave_lang::ast::TypeName;

use crate::fields::ValueType;
use crate::words::WordSet;

/// A built-in function.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Builtin {
    /// `between(a, b)`: a length.
    Between,
    /// `extent(items, axis)`: a length. With `of: part`, the axis is one
    /// of that part's own axes (`length`, `width`, `thickness`); with
    /// `of: contact(a, b)`, one of the patch's (`length` and `width` along
    /// it, `thickness` square to it).
    Extent,
    /// `mid(item, axis)`: an anchor.
    Mid,
    /// `opening(left:, right:, bottom:, top:, front:, back:)`: a region.
    Opening,
    /// `split(region, axis)`, `split(region, axis, at: part)` or
    /// `split(region, axis, shares: [n, ...])`: regions.
    Split,
    /// `cover(region, mount, thickness: d)`: a region.
    Cover,
    /// `overlay(left:, right:, top:, bottom:, meeting:)`: a mount.
    Overlay,
    /// `inset(reveal:)`: a mount.
    Inset,
    /// `turned(profile)`: a shape.
    Turned,
    /// `tapered(to:, toward:, on:, square:)`: a shape, a square part
    /// planed thinner toward one end (grammar, "Shaped parts").
    Tapered,
    /// `arched(rise:, on:, ends:)`: a shape, a square part with an arch
    /// sawn into one face (grammar, "Shaped parts").
    Arched,
    /// `joined(by:)`: how the boards of a part are joined.
    Joined,
    /// `spliced(by:, at:, from:)`: where a part is spliced end to end, and
    /// by what joint.
    Spliced,
    /// `box(length:, width:, thickness:)`: an envelope.
    Box,
    /// `box(across:, up:, deep:)`: a region made from three lengths, the
    /// project's root. It shares the name `box` with [`Builtin::Box`]; the
    /// two are told apart by their argument names (see
    /// [`Builtin::for_arguments`]), so it is not in [`Builtin::ALL`].
    RegionBox,
    /// `cylinder(diameter:, length:)`: an envelope.
    Cylinder,
    /// `model(source, align:)`: a model.
    Model,
    /// `ends(inset:)`: a spacing.
    Ends,
    /// `pitch(d)`: a spacing.
    Pitch,
    /// `gap(d)`: a spacing.
    Gap,
    /// `contact(a, b)`: the patch where two parts touch, a [`TypeName::Patch`].
    Contact,
    /// `edge_to_edge(a, b)`: `true` when two parts lie side by side in one
    /// plane and meet along an edge of each, a [`TypeName::Bool`]. The
    /// first question asked of how two parts meet; see
    /// [`Builtin::asks_how_parts_meet`].
    EdgeToEdge,
    /// `wall(cut)`: the least wood left between one of a kind's named cuts
    /// and any face or other cut of the same part, a length; read only in
    /// a joint or hardware kind's `require`.
    Wall,
}

/// A parameter of a built-in function.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Parameter {
    /// The name; positional parameters have one too, for messages.
    pub name: &'static str,
    /// `true` if written without its name, in order.
    pub positional: bool,
    /// What it expects.
    pub ty: ValueType,
    /// `true` if it must be given.
    pub required: bool,
}

pub(crate) const fn positional(name: &'static str, ty: ValueType) -> Parameter {
    Parameter {
        name,
        positional: true,
        ty,
        required: true,
    }
}

pub(crate) const fn named(name: &'static str, ty: ValueType, required: bool) -> Parameter {
    Parameter {
        name,
        positional: false,
        ty,
        required,
    }
}

use ValueType::{Type, Words};
const LENGTH: ValueType = Type(TypeName::Length);
const ANCHOR: ValueType = Type(TypeName::Anchor);
const AXIS: ValueType = Words(WordSet::Axis);
const ITEM: ValueType = ValueType::Items;

impl Builtin {
    /// Every built-in, in the order the grammar lists them.
    pub const ALL: [Builtin; 22] = [
        Builtin::Between,
        Builtin::Extent,
        Builtin::Mid,
        Builtin::Opening,
        Builtin::Split,
        Builtin::Cover,
        Builtin::Overlay,
        Builtin::Inset,
        Builtin::Turned,
        Builtin::Tapered,
        Builtin::Arched,
        Builtin::Joined,
        Builtin::Spliced,
        Builtin::Box,
        Builtin::Cylinder,
        Builtin::Model,
        Builtin::Ends,
        Builtin::Pitch,
        Builtin::Gap,
        Builtin::Contact,
        Builtin::EdgeToEdge,
        Builtin::Wall,
    ];

    /// The function's name.
    pub fn word(self) -> &'static str {
        match self {
            Builtin::Between => "between",
            Builtin::Extent => "extent",
            Builtin::Mid => "mid",
            Builtin::Opening => "opening",
            Builtin::Split => "split",
            Builtin::Cover => "cover",
            Builtin::Overlay => "overlay",
            Builtin::Inset => "inset",
            Builtin::Turned => "turned",
            Builtin::Tapered => "tapered",
            Builtin::Arched => "arched",
            Builtin::Joined => "joined",
            Builtin::Spliced => "spliced",
            Builtin::Box | Builtin::RegionBox => "box",
            Builtin::Cylinder => "cylinder",
            Builtin::Model => "model",
            Builtin::Ends => "ends",
            Builtin::Pitch => "pitch",
            Builtin::Gap => "gap",
            Builtin::Contact => "contact",
            Builtin::EdgeToEdge => "edge_to_edge",
            Builtin::Wall => "wall",
        }
    }

    /// The built-in for a name, if it is one. For `box` this is the
    /// hardware envelope; [`Builtin::for_arguments`] picks the region.
    pub fn from_word(word: &str) -> Option<Builtin> {
        Builtin::ALL.into_iter().find(|b| b.word() == word)
    }

    /// The argument names of `box(across:, up:, deep:)`, the region.
    pub const REGION_BOX_ARGUMENTS: [&'static str; 3] = ["across", "up", "deep"];

    /// The built-in a call means once its argument names are known: `box`
    /// with any of `across:`, `up:` or `deep:` is the region
    /// [`Builtin::RegionBox`]; every other built-in is itself.
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "the variants it does not name take the general path written last"
    )]
    pub fn for_arguments<'a>(self, mut names: impl Iterator<Item = &'a str>) -> Builtin {
        match self {
            Builtin::Box | Builtin::RegionBox => {
                if names.any(|name| Builtin::REGION_BOX_ARGUMENTS.contains(&name)) {
                    Builtin::RegionBox
                } else {
                    Builtin::Box
                }
            }
            other => other,
        }
    }

    /// `true` for a question about how two parts meet (`edge_to_edge`).
    /// A kind's rule that asks one is checked before its other rules, and
    /// when it fails the joint makes nothing for that pair (grammar,
    /// "Checks and `require`").
    pub fn asks_how_parts_meet(self) -> bool {
        match self {
            Builtin::EdgeToEdge => true,
            Builtin::Between
            | Builtin::Extent
            | Builtin::Mid
            | Builtin::Opening
            | Builtin::Split
            | Builtin::Cover
            | Builtin::Overlay
            | Builtin::Inset
            | Builtin::Turned
            | Builtin::Tapered
            | Builtin::Arched
            | Builtin::Joined
            | Builtin::Spliced
            | Builtin::Box
            | Builtin::RegionBox
            | Builtin::Cylinder
            | Builtin::Model
            | Builtin::Ends
            | Builtin::Pitch
            | Builtin::Gap
            | Builtin::Contact
            | Builtin::Wall => false,
        }
    }

    /// What it returns. `split` returns a list of regions, which only an
    /// array `region` declaration can hold.
    pub fn returns(self) -> TypeName {
        match self {
            Builtin::Between | Builtin::Extent | Builtin::Wall => TypeName::Length,
            Builtin::Mid => TypeName::Anchor,
            Builtin::Opening | Builtin::Split | Builtin::Cover | Builtin::RegionBox => {
                TypeName::Region
            }
            Builtin::Overlay | Builtin::Inset => TypeName::Mount,
            Builtin::Turned | Builtin::Tapered | Builtin::Arched => TypeName::Shape,
            Builtin::Joined => TypeName::Boards,
            Builtin::Spliced => TypeName::Splices,
            Builtin::Box | Builtin::Cylinder => TypeName::Envelope,
            Builtin::Model => TypeName::Model,
            Builtin::Ends | Builtin::Pitch | Builtin::Gap => TypeName::Spacing,
            Builtin::Contact => TypeName::Patch,
            Builtin::EdgeToEdge => TypeName::Bool,
        }
    }

    /// Its parameters: positional ones first, in order.
    pub fn parameters(self) -> &'static [Parameter] {
        match self {
            Builtin::Between => {
                const LIST: &[Parameter] = &[positional("a", ITEM), positional("b", ITEM)];
                LIST
            }
            Builtin::Extent => {
                const LIST: &[Parameter] = &[
                    positional("items", ITEM),
                    positional("axis", AXIS),
                    // A part or a patch: checked by the resolver.
                    named("of", ITEM, false),
                ];
                LIST
            }
            Builtin::Mid => {
                const LIST: &[Parameter] = &[positional("item", ITEM), positional("axis", AXIS)];
                LIST
            }
            Builtin::Opening => {
                const LIST: &[Parameter] = &[
                    named("left", ANCHOR, true),
                    named("right", ANCHOR, true),
                    named("bottom", ANCHOR, true),
                    named("top", ANCHOR, true),
                    named("front", ANCHOR, true),
                    named("back", ANCHOR, true),
                ];
                LIST
            }
            Builtin::Split => {
                const LIST: &[Parameter] = &[
                    positional("region", Type(TypeName::Region)),
                    positional("axis", AXIS),
                    named("at", Type(TypeName::Part), false),
                    // A Count or a Ratio each: checked by the resolver.
                    named("shares", ValueType::ListOf(&ITEM), false),
                ];
                LIST
            }
            Builtin::Cover => {
                const LIST: &[Parameter] = &[
                    positional("region", Type(TypeName::Region)),
                    positional("mount", Type(TypeName::Mount)),
                    named("thickness", LENGTH, true),
                ];
                LIST
            }
            Builtin::Overlay => {
                const LIST: &[Parameter] = &[
                    named("left", LENGTH, false),
                    named("right", LENGTH, false),
                    named("top", LENGTH, false),
                    named("bottom", LENGTH, false),
                    named("meeting", LENGTH, false),
                ];
                LIST
            }
            Builtin::Inset => {
                const LIST: &[Parameter] = &[named("reveal", LENGTH, true)];
                LIST
            }
            Builtin::Turned => {
                const LIST: &[Parameter] = &[positional("profile", Type(TypeName::Profile))];
                LIST
            }
            Builtin::Tapered => {
                const FACE: ValueType = Words(WordSet::Face);
                const LIST: &[Parameter] = &[
                    named("to", LENGTH, true),
                    named("toward", FACE, true),
                    // One face or a list of them: checked by the solver.
                    named("on", ValueType::ListOf(&FACE), true),
                    named("square", LENGTH, false),
                ];
                LIST
            }
            Builtin::Arched => {
                const LIST: &[Parameter] = &[
                    named("rise", LENGTH, true),
                    named("on", Words(WordSet::Face), true),
                    named("ends", LENGTH, false),
                ];
                LIST
            }
            Builtin::Joined => {
                const JOINT_KIND: ValueType = Type(TypeName::JointKind);
                const LIST: &[Parameter] = &[named("by", ValueType::ListOf(&JOINT_KIND), true)];
                LIST
            }
            Builtin::Spliced => {
                const LIST: &[Parameter] = &[
                    named("by", Type(TypeName::JointKind), true),
                    named("at", ValueType::ListOf(&LENGTH), true),
                    named("from", Words(WordSet::Face), true),
                ];
                LIST
            }
            Builtin::Box => {
                const LIST: &[Parameter] = &[
                    named("length", LENGTH, true),
                    named("width", LENGTH, true),
                    named("thickness", LENGTH, true),
                ];
                LIST
            }
            Builtin::RegionBox => {
                const LIST: &[Parameter] = &[
                    named("across", LENGTH, true),
                    named("up", LENGTH, true),
                    named("deep", LENGTH, true),
                ];
                LIST
            }
            Builtin::Cylinder => {
                const LIST: &[Parameter] = &[
                    named("diameter", LENGTH, true),
                    named("length", LENGTH, true),
                ];
                LIST
            }
            Builtin::Model => {
                const LIST: &[Parameter] = &[
                    positional("source", Type(TypeName::Text)),
                    named("align", Type(TypeName::Text), false),
                    named("sha256", Type(TypeName::Text), false),
                ];
                LIST
            }
            Builtin::Ends => {
                const LIST: &[Parameter] = &[named("inset", LENGTH, true)];
                LIST
            }
            Builtin::Pitch | Builtin::Gap => {
                const LIST: &[Parameter] = &[positional("distance", LENGTH)];
                LIST
            }
            Builtin::Contact | Builtin::EdgeToEdge => {
                const LIST: &[Parameter] = &[positional("a", ITEM), positional("b", ITEM)];
                LIST
            }
            // The name of one of the kind's cuts: checked by the resolver.
            Builtin::Wall => {
                const LIST: &[Parameter] = &[positional("cut", ITEM)];
                LIST
            }
        }
    }
}

/// A size of a [`TypeName::Patch`], read as `contact(a, b).length`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PatchSize {
    /// `length`: the longer side of the patch.
    Length,
    /// `width`: the shorter side of the patch.
    Width,
}

impl PatchSize {
    /// Every size, in the order the grammar lists them.
    pub const ALL: [PatchSize; 2] = [PatchSize::Length, PatchSize::Width];

    /// The size's name as written after the `.`.
    pub fn word(self) -> &'static str {
        match self {
            PatchSize::Length => "length",
            PatchSize::Width => "width",
        }
    }

    /// The size for a name, if it is one.
    pub fn from_word(word: &str) -> Option<PatchSize> {
        PatchSize::ALL.into_iter().find(|size| size.word() == word)
    }
}

/// `node("name")`: the node `model(..., align:)` puts on the kind's origin.
pub const MODEL_NODE: &str = "node";

/// `point(along:, across:, depth:)`: a point in a hardware kind's frame,
/// written only inside a member or a mechanism.
pub const POINT: &str = "point";

/// The named arguments of [`POINT`], each a length that defaults to `0mm`.
pub const POINT_ARGUMENTS: [&str; 3] = ["along", "across", "depth"];

/// A shape of a hardware kind's member (grammar, "Shapes"), written in the
/// kind's frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ShapeFunction {
    /// `box(along:, across:, depth:)`
    Box,
    /// `cylinder(diameter:, depth:, at:, axis:, tilt:, toward:)`
    Cylinder,
    /// `prism(points: [...], along:)`
    Prism,
    /// `bar(from:, to:, thickness:, along:)`
    Bar,
}

impl ShapeFunction {
    /// Every shape, in the order the grammar lists them.
    pub const ALL: [ShapeFunction; 4] = [
        ShapeFunction::Box,
        ShapeFunction::Cylinder,
        ShapeFunction::Prism,
        ShapeFunction::Bar,
    ];

    /// The function's name.
    pub fn word(self) -> &'static str {
        match self {
            ShapeFunction::Box => "box",
            ShapeFunction::Cylinder => "cylinder",
            ShapeFunction::Prism => "prism",
            ShapeFunction::Bar => "bar",
        }
    }

    /// The shape for a name, if it is one.
    pub fn from_word(word: &str) -> Option<ShapeFunction> {
        ShapeFunction::ALL
            .into_iter()
            .find(|shape| shape.word() == word)
    }

    /// Its named arguments. A cylinder's range is written under the name
    /// of its axis (`depth:` unless `axis:` lays it down).
    pub fn arguments(self) -> &'static [&'static str] {
        match self {
            ShapeFunction::Box => &["along", "across", "depth"],
            ShapeFunction::Cylinder => &[
                "diameter", "along", "across", "depth", "at", "axis", "tilt", "toward",
            ],
            ShapeFunction::Prism => &["points", "along"],
            ShapeFunction::Bar => &["from", "to", "thickness", "along"],
        }
    }

    /// `true` for the shapes that are not also envelopes outside a member
    /// (`prism` and `bar`): like `point`, they are written only inside a
    /// hardware kind's member.
    pub fn only_in_members(self) -> bool {
        match self {
            ShapeFunction::Prism | ShapeFunction::Bar => true,
            ShapeFunction::Box | ShapeFunction::Cylinder => false,
        }
    }
}

/// The words written only inside a hardware kind's member or mechanism:
/// `point`, `prism` and `bar`.
pub fn is_member_only_function(word: &str) -> bool {
    word == POINT || ShapeFunction::from_word(word).is_some_and(ShapeFunction::only_in_members)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{GRAMMAR, between, call_arguments, table_rows, ticked};

    #[test]
    fn builtins_match_the_grammar_and_the_parser() {
        assert_eq!(
            Builtin::ALL.map(Builtin::word),
            stave_lang::BUILTIN_FUNCTIONS
        );
        let rows = table_rows("| Function | Returns | Meaning |\n");
        for row in rows {
            let returns = row[1].trim_matches('`');
            for call in ticked(&row[0]) {
                let name = &call[..call.find('(').unwrap()];
                let builtin = Builtin::from_word(name).unwrap();
                assert_eq!(builtin.returns().word(), returns, "{name}");
                let named: Vec<String> = builtin
                    .parameters()
                    .iter()
                    .filter(|p| !p.positional)
                    .map(|p| p.name.to_owned())
                    .collect();
                let written = call_arguments(&format!("`{call}`"));
                if call.contains(":)") || call.contains(":,") || call.ends_with(":)") {
                    for name in &written {
                        assert!(named.contains(name), "{call}: {name}");
                    }
                }
            }
        }
        let regions = between(GRAMMAR, "| Function | Result |\n|---|---|\n", "\n\n");
        assert!(regions.contains("`split(region, axis, at: part)`"));
        assert!(regions.contains("`cover(region, mount, thickness: d)`"));
    }
}
