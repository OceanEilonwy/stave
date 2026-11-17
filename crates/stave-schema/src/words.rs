//! Closed word lists: the bare enum words of namespace 4 ("Names"). A bare
//! identifier is one of these words only where the expected type has the
//! list; anywhere else it is an item.
//!
//! Faces, directions, axes and part axes are the placement vocabulary in
//! [`stave_lang::placement`]; this module adds the rest and a [`WordSet`]
//! that names every list.

use stave_lang::ast::Unit;
use stave_lang::placement::{Axis, Direction, Face, PartAxis};

pub use crate::species::Species;

macro_rules! word_enum {
    (
        $(#[doc = $doc:literal])*
        $name:ident { $( $(#[doc = $vdoc:literal])* $variant:ident = $word:literal ),+ $(,)? }
    ) => {
        $(#[doc = $doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum $name {
            $( $(#[doc = $vdoc])* $variant ),+
        }

        impl $name {
            /// Every word, in the order the grammar lists them.
            pub const ALL: &'static [$name] = &[$( $name::$variant ),+];

            /// The word as written in source.
            pub fn word(self) -> &'static str {
                match self {
                    $( $name::$variant => $word ),+
                }
            }

            /// The value for a word, if it is one.
            pub fn from_word(word: &str) -> Option<$name> {
                $name::ALL.iter().copied().find(|value| value.word() == word)
            }
        }
    };
}

word_enum! {
    /// What a stock is bought as: the `form:` field.
    StockForm {
        /// Solid wood boards.
        Solid = "solid",
        /// Plywood sheets.
        Plywood = "plywood",
        /// MDF sheets.
        Mdf = "mdf",
        /// Round stock, sized by `diameter:`.
        Dowel = "dowel",
    }
}

word_enum! {
    /// What a profile's station values are: the `measure:` field.
    Measure {
        /// The values are diameters.
        Diameter = "diameter",
        /// The values are radii.
        Radius = "radius",
    }
}

word_enum! {
    /// A motion state in a scene's `pose`.
    MotionWord {
        /// The hinge's opening angle or the slide's travel.
        Open = "open",
        /// The rest position.
        Closed = "closed",
    }
}

word_enum! {
    /// Which way the grain runs: the `grain:` field.
    Grain {
        /// Along the part's length (the default).
        Length = "length",
        /// Along the part's width.
        Width = "width",
    }
}

word_enum! {
    /// The units a project's output uses: the `units:` field. Angles are not
    /// output units.
    OutputUnit {
        /// Millimetres (the default).
        Millimetre = "mm",
        /// Centimetres.
        Centimetre = "cm",
        /// Metres.
        Metre = "m",
        /// Inches.
        Inch = "in",
        /// Feet.
        Foot = "ft",
    }
}

impl OutputUnit {
    /// The same unit as a literal suffix.
    pub fn unit(self) -> Unit {
        match self {
            OutputUnit::Millimetre => Unit::Millimeter,
            OutputUnit::Centimetre => Unit::Centimeter,
            OutputUnit::Metre => Unit::Meter,
            OutputUnit::Inch => Unit::Inch,
            OutputUnit::Foot => Unit::Foot,
        }
    }
}

/// The word `through`: a depth that goes all the way through a part.
pub const THROUGH: &str = "through";

/// The bare spacing words of `spaced` (the other spacings are calls).
pub const SPACING_WORDS: [&str; 2] = ["evenly", "ends"];

/// Every closed word list a position can expect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum WordSet {
    /// `up`, `across`, `deep`.
    Axis,
    /// `up`, `down`, `left`, `right`, `front`, `back`.
    Direction,
    /// `top`, `bottom`, `left`, `right`, `front`, `back`.
    Face,
    /// `length`, `width`, `thickness`.
    PartAxis,
    /// `mm`, `cm`, `m`, `in`, `ft`.
    OutputUnit,
    /// The species words.
    Species,
    /// `solid`, `plywood`, `mdf`, `dowel`.
    StockForm,
    /// `diameter`, `radius`.
    Measure,
    /// `open`, `closed`.
    Motion,
    /// `length`, `width`.
    Grain,
    /// `through`.
    Through,
    /// `evenly`, `ends`.
    Spacing,
}

impl WordSet {
    /// Every word list.
    pub const ALL: [WordSet; 12] = [
        WordSet::Axis,
        WordSet::Direction,
        WordSet::Face,
        WordSet::PartAxis,
        WordSet::OutputUnit,
        WordSet::Species,
        WordSet::StockForm,
        WordSet::Measure,
        WordSet::Motion,
        WordSet::Grain,
        WordSet::Through,
        WordSet::Spacing,
    ];

    /// `true` if `word` is in the list.
    pub fn contains(self, word: &str) -> bool {
        match self {
            WordSet::Axis => Axis::from_word(word).is_some(),
            WordSet::Direction => Direction::from_word(word).is_some(),
            WordSet::Face => Face::from_word(word).is_some(),
            WordSet::PartAxis => PartAxis::from_word(word).is_some(),
            WordSet::OutputUnit => OutputUnit::from_word(word).is_some(),
            WordSet::Species => Species::from_word(word).is_some(),
            WordSet::StockForm => StockForm::from_word(word).is_some(),
            WordSet::Measure => Measure::from_word(word).is_some(),
            WordSet::Motion => MotionWord::from_word(word).is_some(),
            WordSet::Grain => Grain::from_word(word).is_some(),
            WordSet::Through => word == THROUGH,
            WordSet::Spacing => SPACING_WORDS.contains(&word),
        }
    }

    /// The words of the list, for completion and help lines.
    pub fn words(self) -> Vec<&'static str> {
        match self {
            WordSet::Axis => Axis::ALL.iter().map(|axis| axis.word()).collect(),
            WordSet::Direction => Direction::ALL.iter().map(|d| d.word()).collect(),
            WordSet::Face => Face::ALL.iter().map(|face| face.word()).collect(),
            WordSet::PartAxis => PartAxis::ALL.iter().map(|axis| axis.word()).collect(),
            WordSet::OutputUnit => OutputUnit::ALL.iter().map(|unit| unit.word()).collect(),
            WordSet::Species => Species::ALL.iter().map(|species| species.word()).collect(),
            WordSet::StockForm => StockForm::ALL.iter().map(|form| form.word()).collect(),
            WordSet::Measure => Measure::ALL.iter().map(|measure| measure.word()).collect(),
            WordSet::Motion => MotionWord::ALL.iter().map(|motion| motion.word()).collect(),
            WordSet::Grain => Grain::ALL.iter().map(|grain| grain.word()).collect(),
            WordSet::Through => vec![THROUGH],
            WordSet::Spacing => SPACING_WORDS.to_vec(),
        }
    }

    /// How the list is described in a message: "an axis", "a species".
    pub fn description(self) -> &'static str {
        match self {
            WordSet::Axis => "an axis",
            WordSet::Direction => "a direction",
            WordSet::Face => "a face",
            WordSet::PartAxis => "a part axis",
            WordSet::OutputUnit => "a unit",
            WordSet::Species => "a species",
            WordSet::StockForm => "a stock form",
            WordSet::Measure => "'diameter' or 'radius'",
            WordSet::Motion => "'open' or 'closed'",
            WordSet::Grain => "'length' or 'width'",
            WordSet::Through => "'through'",
            WordSet::Spacing => "a spacing",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{GRAMMAR, between, ticked};

    #[test]
    fn stock_forms_match_the_grammar() {
        let row = between(GRAMMAR, "| `form` | ", " |");
        assert_eq!(
            ticked(row),
            StockForm::ALL.iter().map(|f| f.word()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn units_match_the_grammar() {
        let row = between(GRAMMAR, "| `units` | ", " |");
        assert_eq!(
            ticked(row),
            OutputUnit::ALL.iter().map(|u| u.word()).collect::<Vec<_>>()
        );
        for unit in OutputUnit::ALL {
            assert_eq!(unit.unit().suffix(), unit.word());
        }
    }

    #[test]
    fn measures_grain_and_motion_match_the_grammar() {
        let measure = between(GRAMMAR, "| `measure` | ", " |");
        assert_eq!(ticked(measure), ["diameter", "radius"]);
        let grain = between(GRAMMAR, "| `grain` | ", " |");
        assert_eq!(ticked(grain), ["length", "width"]);
        let pose = between(GRAMMAR, "| `pose <target> = <value>` | ", " |");
        assert!(ticked(pose).starts_with(&["open".to_owned(), "closed".to_owned()]));
        assert!(GRAMMAR.contains("`through` as a depth means all the way through"));
        assert!(GRAMMAR.contains("`align: node(\"mount\")` (optional)"));
    }

    #[test]
    fn every_word_belongs_to_its_list() {
        for set in WordSet::ALL {
            let words = set.words();
            assert!(!words.is_empty());
            for word in words {
                assert!(set.contains(word), "{set:?} {word}");
            }
            assert!(!set.contains("nonsense"));
        }
        assert!(WordSet::Grain.contains("width") && !WordSet::Grain.contains("thickness"));
    }
}
