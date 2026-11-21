//! Static types of resolved expressions.
//!
//! Every resolved expression carries a [`Ty`]. Types are worked out once,
//! while names are resolved, so a unit or type mistake is reported where it
//! is written (layer 2) and later stages can rely on them.
//! [`Ty::Unknown`] marks an expression whose type depends on a value only
//! known when the design is solved (a member of a kind value passed in as a
//! param); it never causes an error by itself.

use std::fmt;

use stave_lang::ast::{BaseKind, TypeName};
use stave_schema::fields::ValueType;
use stave_schema::words::WordSet;

use crate::ids::{ComponentId, KindId};

/// What a kind value is a value of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KindTy {
    /// The base; `None` for `HardwareKind`, which is a slide, hinge or pull.
    pub base: Option<BaseKind>,
    /// The kind, when it is known where the expression is written.
    pub kind: Option<KindId>,
}

/// The static type of an expression.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Ty {
    /// A length.
    Length,
    /// An angle.
    Angle,
    /// An exact unitless fraction.
    Ratio,
    /// A whole number; usable wherever a ratio is expected.
    Count,
    /// `true` or `false`.
    Bool,
    /// A string.
    Text,
    /// Exactly one part.
    Part,
    /// One part or a list of parts.
    Parts,
    /// A stock item.
    Stock,
    /// A region.
    Region,
    /// A list of regions: what `split` returns.
    Regions,
    /// A profile item.
    Profile,
    /// A face or a mid-plane.
    Anchor,
    /// `overlay(...)` or `inset(...)`.
    Mount,
    /// `turned(profile)`.
    Shape,
    /// `joined(by: ...)`.
    Boards,
    /// `spliced(by: ..., at: ..., from: ...)`.
    Splices,
    /// `box(...)` or `cylinder(...)`.
    Envelope,
    /// `model(...)`.
    Model,
    /// A spacing for `spaced`.
    Spacing,
    /// `contact(a, b)`: where two parts touch.
    Patch,
    /// A bare word from a closed list (axes and directions included).
    Word(WordSet),
    /// A kind value.
    Kind(KindTy),
    /// A tool kind.
    Tool,
    /// A recipe.
    Recipe,
    /// An assembly (an instance of this component).
    Assembly(ComponentId),
    /// A joint.
    Joint,
    /// A piece of hardware.
    Hardware,
    /// An array of elements of the inner type, not yet indexed.
    Array(Box<Ty>),
    /// A list of values of the inner type.
    List(Box<Ty>),
    /// `fit`.
    Fit,
    /// Not known until the design is solved.
    Unknown,
}

impl Ty {
    /// The type a `param` or role may name.
    pub fn from_type_name(name: TypeName) -> Ty {
        let kind = |base| {
            Ty::Kind(KindTy {
                base: Some(base),
                kind: None,
            })
        };
        match name {
            TypeName::Length => Ty::Length,
            TypeName::Angle => Ty::Angle,
            TypeName::Ratio => Ty::Ratio,
            TypeName::Count => Ty::Count,
            TypeName::Bool => Ty::Bool,
            TypeName::Text => Ty::Text,
            TypeName::Part => Ty::Part,
            TypeName::Parts => Ty::Parts,
            TypeName::Stock => Ty::Stock,
            TypeName::Region => Ty::Region,
            TypeName::Profile => Ty::Profile,
            TypeName::Anchor => Ty::Anchor,
            TypeName::Axis => Ty::Word(WordSet::Axis),
            TypeName::Direction => Ty::Word(WordSet::Direction),
            TypeName::Mount => Ty::Mount,
            TypeName::Shape => Ty::Shape,
            TypeName::Boards => Ty::Boards,
            TypeName::Splices => Ty::Splices,
            TypeName::Envelope => Ty::Envelope,
            TypeName::Model => Ty::Model,
            TypeName::Spacing => Ty::Spacing,
            TypeName::Patch => Ty::Patch,
            TypeName::JointKind => kind(BaseKind::Joint),
            TypeName::HardwareKind => Ty::Kind(KindTy {
                base: None,
                kind: None,
            }),
            TypeName::SlideKind => kind(BaseKind::Slide),
            TypeName::HingeKind => kind(BaseKind::Hinge),
            TypeName::PullKind => kind(BaseKind::Pull),
            TypeName::SupportKind => kind(BaseKind::Support),
            TypeName::FittingKind => kind(BaseKind::Fitting),
            TypeName::FastenerKind => kind(BaseKind::Fastener),
            TypeName::Tool => Ty::Tool,
            TypeName::Recipe => Ty::Recipe,
        }
    }

    /// The type a schema position expects. `None` for the special field
    /// vocabularies, which are lowered by hand.
    pub fn from_value_type(ty: ValueType) -> Option<Ty> {
        match ty {
            ValueType::Type(name) => Some(Ty::from_type_name(name)),
            ValueType::Words(set) => Some(Ty::Word(set)),
            ValueType::LengthOrFit | ValueType::Depth => Some(Ty::Length),
            ValueType::ListOf(inner) => Ty::from_value_type(*inner).map(|t| Ty::List(Box::new(t))),
            ValueType::Items => Some(Ty::Unknown),
            ValueType::Special(_) => None,
        }
    }

    /// `true` for the four number-like types, whose mismatches are
    /// `unit-mismatch` rather than `type-mismatch`.
    pub fn is_quantity(&self) -> bool {
        matches!(self, Ty::Length | Ty::Angle | Ty::Ratio | Ty::Count)
    }

    /// `true` if a value of this type can stand where `expected` is
    /// required.
    pub fn fits(&self, expected: &Ty) -> bool {
        match (self, expected) {
            (Ty::Unknown, _) | (_, Ty::Unknown) => true,
            (Ty::Count, Ty::Ratio) => true,
            (Ty::Part, Ty::Parts) => true,
            (Ty::List(inner), Ty::Parts) => inner.fits(&Ty::Parts),
            (Ty::Parts, Ty::List(inner)) => Ty::Part.fits(inner),
            // `joined(by:)` takes one joint kind or a list of them, and
            // `spliced(at:)` one length or a list of them.
            (Ty::Kind(_), Ty::List(inner)) if matches!(**inner, Ty::Kind(_)) => self.fits(inner),
            (Ty::Length, Ty::List(inner)) if **inner == Ty::Length => true,
            // `tapered(on:)` one face or a list of them.
            (Ty::Word(found), Ty::List(inner)) if **inner == Ty::Word(*found) => true,
            (Ty::Word(found), Ty::Word(wanted)) => {
                found == wanted || (*found == WordSet::Grain && *wanted == WordSet::PartAxis)
            }
            (Ty::Kind(found), Ty::Kind(wanted)) => {
                let base_fits = match (found.base, wanted.base) {
                    (_, None) => found.base.is_none_or(|base| base.is_mounted_hardware()),
                    (None, Some(base)) => base.is_mounted_hardware(),
                    (Some(found), Some(wanted)) => found == wanted,
                };
                let kind_fits = match (found.kind, wanted.kind) {
                    (_, None) | (None, _) => true,
                    (Some(found), Some(wanted)) => found == wanted,
                };
                base_fits && kind_fits
            }
            (Ty::List(found), Ty::List(wanted)) => found.fits(wanted),
            (found, wanted) => found == wanted,
        }
    }

    /// The type of one element, for arrays and lists.
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "picks out only the variants it names: any other, a new one included, is not one of them"
    )]
    pub fn element(&self) -> Option<&Ty> {
        match self {
            Ty::Array(inner) | Ty::List(inner) => Some(inner),
            _ => None,
        }
    }

    /// The description used in messages: "a length", "a part".
    pub fn description(&self) -> String {
        match self {
            Ty::Length => "a length".into(),
            Ty::Angle => "an angle".into(),
            Ty::Ratio => "a ratio".into(),
            Ty::Count => "a count".into(),
            Ty::Bool => "true or false".into(),
            Ty::Text => "a text".into(),
            Ty::Part => "a part".into(),
            Ty::Parts => "parts".into(),
            Ty::Stock => "a stock".into(),
            Ty::Region => "a region".into(),
            Ty::Regions => "regions".into(),
            Ty::Profile => "a profile".into(),
            Ty::Anchor => "a face or mid-plane".into(),
            Ty::Mount => "a mount".into(),
            Ty::Shape => "a shape".into(),
            Ty::Boards => "a way of joining boards".into(),
            Ty::Splices => "a way of splicing a part".into(),
            Ty::Envelope => "an envelope".into(),
            Ty::Model => "a model".into(),
            Ty::Spacing => "a spacing".into(),
            Ty::Patch => "a contact patch".into(),
            Ty::Word(set) => set.description().into(),
            Ty::Kind(kind) => match kind.base {
                Some(BaseKind::Joint) => "a joint kind".into(),
                Some(BaseKind::Slide) => "a slide kind".into(),
                Some(BaseKind::Hinge) => "a hinge kind".into(),
                Some(BaseKind::Pull) => "a pull kind".into(),
                Some(BaseKind::Support) => "a support kind".into(),
                Some(BaseKind::Fitting) => "a fitting kind".into(),
                Some(BaseKind::Fastener) => "a fastener kind".into(),
                Some(BaseKind::Tool) => "a tool".into(),
                None => "a hardware kind".into(),
            },
            Ty::Tool => "a tool".into(),
            Ty::Recipe => "a recipe".into(),
            Ty::Assembly(_) => "an assembly".into(),
            Ty::Joint => "a joint".into(),
            Ty::Hardware => "a piece of hardware".into(),
            Ty::Array(inner) => format!("an array of {}", plural(inner)),
            Ty::List(inner) => format!("a list of {}", plural(inner)),
            Ty::Fit => "'fit'".into(),
            Ty::Unknown => "a value".into(),
        }
    }
}

fn plural(ty: &Ty) -> String {
    let single = ty.description();
    let bare = single
        .strip_prefix("a ")
        .or_else(|| single.strip_prefix("an "))
        .unwrap_or(&single);
    if bare.ends_with('s') {
        bare.to_owned()
    } else {
        format!("{bare}s")
    }
}

impl fmt::Display for Ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.description())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_stand_for_ratios_and_parts_for_lists() {
        assert!(Ty::Count.fits(&Ty::Ratio));
        assert!(!Ty::Ratio.fits(&Ty::Count));
        assert!(Ty::Part.fits(&Ty::Parts));
        assert!(Ty::List(Box::new(Ty::Part)).fits(&Ty::Parts));
        assert!(!Ty::Region.fits(&Ty::Part));
        assert!(Ty::Unknown.fits(&Ty::Length));
    }

    #[test]
    fn hardware_kinds_accept_any_mounted_base() {
        let slide = Ty::from_type_name(TypeName::SlideKind);
        let hardware = Ty::from_type_name(TypeName::HardwareKind);
        assert!(slide.fits(&hardware));
        assert!(!Ty::from_type_name(TypeName::JointKind).fits(&hardware));
        assert!(!slide.fits(&Ty::from_type_name(TypeName::PullKind)));
    }

    #[test]
    fn descriptions_read_as_english() {
        assert_eq!(Ty::Length.to_string(), "a length");
        assert_eq!(Ty::Angle.to_string(), "an angle");
        assert_eq!(Ty::List(Box::new(Ty::Tool)).to_string(), "a list of tools");
        assert_eq!(Ty::Word(WordSet::Axis).to_string(), "an axis");
    }
}
