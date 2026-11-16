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
    /// `extent(items, axis)`: a length.
    Extent,
    /// `mid(item, axis)`: an anchor.
    Mid,
    /// `opening(left:, right:, bottom:, top:, front:, back:)`: a region.
    Opening,
    /// `split(region, axis)` or `split(region, axis, at: part)`: regions.
    Split,
    /// `cover(region, mount, thickness: d)`: a region.
    Cover,
    /// `overlay(left:, right:, top:, bottom:, meeting:)`: a mount.
    Overlay,
    /// `inset(reveal:)`: a mount.
    Inset,
    /// `turned(profile)`: a shape.
    Turned,
    /// `box(length:, width:, thickness:)`: an envelope.
    Box,
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

const fn positional(name: &'static str, ty: ValueType) -> Parameter {
    Parameter {
        name,
        positional: true,
        ty,
        required: true,
    }
}

const fn named(name: &'static str, ty: ValueType, required: bool) -> Parameter {
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
    pub const ALL: [Builtin; 15] = [
        Builtin::Between,
        Builtin::Extent,
        Builtin::Mid,
        Builtin::Opening,
        Builtin::Split,
        Builtin::Cover,
        Builtin::Overlay,
        Builtin::Inset,
        Builtin::Turned,
        Builtin::Box,
        Builtin::Cylinder,
        Builtin::Model,
        Builtin::Ends,
        Builtin::Pitch,
        Builtin::Gap,
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
            Builtin::Box => "box",
            Builtin::Cylinder => "cylinder",
            Builtin::Model => "model",
            Builtin::Ends => "ends",
            Builtin::Pitch => "pitch",
            Builtin::Gap => "gap",
        }
    }

    /// The built-in for a name, if it is one.
    pub fn from_word(word: &str) -> Option<Builtin> {
        Builtin::ALL.into_iter().find(|b| b.word() == word)
    }

    /// What it returns. `split` returns a list of regions, which only an
    /// array `region` declaration can hold.
    pub fn returns(self) -> TypeName {
        match self {
            Builtin::Between | Builtin::Extent => TypeName::Length,
            Builtin::Mid => TypeName::Anchor,
            Builtin::Opening | Builtin::Split | Builtin::Cover => TypeName::Region,
            Builtin::Overlay | Builtin::Inset => TypeName::Mount,
            Builtin::Turned => TypeName::Shape,
            Builtin::Box | Builtin::Cylinder => TypeName::Envelope,
            Builtin::Model => TypeName::Model,
            Builtin::Ends | Builtin::Pitch | Builtin::Gap => TypeName::Spacing,
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
                const LIST: &[Parameter] = &[positional("items", ITEM), positional("axis", AXIS)];
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
            Builtin::Box => {
                const LIST: &[Parameter] = &[
                    named("length", LENGTH, true),
                    named("width", LENGTH, true),
                    named("thickness", LENGTH, true),
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
                    named("align", Words(WordSet::Bounds), true),
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
        }
    }
}

/// `node("name")`: the other form of `model(..., align:)`.
pub const MODEL_NODE: &str = "node";

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
