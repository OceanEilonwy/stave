//! Wood species: the words a stock's `species:` field takes, with what the
//! viewer and the cut list need to know about each.

/// An sRGB colour with 8 bits per channel, for the viewer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Srgb {
    /// Red.
    pub red: u8,
    /// Green.
    pub green: u8,
    /// Blue.
    pub blue: u8,
}

impl Srgb {
    const fn new(red: u8, green: u8, blue: u8) -> Self {
        Self { red, green, blue }
    }
}

/// A density in kilograms per cubic metre (air-dried, typical).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Density(u32);

impl Density {
    /// Kilograms per cubic metre.
    pub const fn kilograms_per_cubic_metre(self) -> u32 {
        self.0
    }
}

/// A wood species, as written in `species:`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Species {
    /// `white_oak`
    WhiteOak,
    /// `red_oak`
    RedOak,
    /// `hard_maple`
    HardMaple,
    /// `soft_maple`
    SoftMaple,
    /// `walnut`
    Walnut,
    /// `cherry`
    Cherry,
    /// `ash`
    Ash,
    /// `beech`
    Beech,
    /// `birch`
    Birch,
    /// `pine`
    Pine,
    /// `spruce`
    Spruce,
    /// `fir`
    Fir,
    /// `cedar`
    Cedar,
    /// `hinoki`
    Hinoki,
    /// `poplar`
    Poplar,
    /// `mahogany`
    Mahogany,
    /// `teak`
    Teak,
}

/// What the catalog knows about a species.
struct Facts {
    word: &'static str,
    title: &'static str,
    colour: Srgb,
    density: Option<u32>,
}

impl Species {
    /// Every species, in the order the grammar lists them.
    pub const ALL: &'static [Species] = &[
        Species::WhiteOak,
        Species::RedOak,
        Species::HardMaple,
        Species::SoftMaple,
        Species::Walnut,
        Species::Cherry,
        Species::Ash,
        Species::Beech,
        Species::Birch,
        Species::Pine,
        Species::Spruce,
        Species::Fir,
        Species::Cedar,
        Species::Hinoki,
        Species::Poplar,
        Species::Mahogany,
        Species::Teak,
    ];

    fn facts(self) -> Facts {
        let facts = |word, title, colour, density| Facts {
            word,
            title,
            colour,
            density,
        };
        match self {
            Species::WhiteOak => facts(
                "white_oak",
                "white oak",
                Srgb::new(196, 160, 112),
                Some(755),
            ),
            Species::RedOak => facts("red_oak", "red oak", Srgb::new(190, 130, 100), Some(700)),
            Species::HardMaple => facts(
                "hard_maple",
                "hard maple",
                Srgb::new(232, 208, 166),
                Some(705),
            ),
            Species::SoftMaple => facts(
                "soft_maple",
                "soft maple",
                Srgb::new(222, 196, 156),
                Some(610),
            ),
            Species::Walnut => facts("walnut", "walnut", Srgb::new(110, 76, 52), Some(610)),
            Species::Cherry => facts("cherry", "cherry", Srgb::new(176, 104, 72), Some(580)),
            Species::Ash => facts("ash", "ash", Srgb::new(214, 190, 150), Some(675)),
            Species::Beech => facts("beech", "beech", Srgb::new(218, 176, 132), Some(720)),
            Species::Birch => facts("birch", "birch", Srgb::new(226, 204, 168), Some(670)),
            Species::Pine => facts("pine", "pine", Srgb::new(226, 194, 140), Some(510)),
            Species::Spruce => facts("spruce", "spruce", Srgb::new(232, 214, 176), Some(450)),
            Species::Fir => facts("fir", "fir", Srgb::new(214, 178, 130), Some(530)),
            Species::Cedar => facts("cedar", "cedar", Srgb::new(186, 118, 82), Some(370)),
            Species::Hinoki => facts("hinoki", "hinoki", Srgb::new(236, 214, 170), Some(410)),
            Species::Poplar => facts("poplar", "poplar", Srgb::new(214, 204, 160), Some(455)),
            Species::Mahogany => facts("mahogany", "mahogany", Srgb::new(150, 72, 48), Some(590)),
            Species::Teak => facts("teak", "teak", Srgb::new(170, 120, 70), Some(655)),
        }
    }

    /// The word as written in `species:`.
    pub fn word(self) -> &'static str {
        self.facts().word
    }

    /// The species for a word, if it is one.
    pub fn from_word(word: &str) -> Option<Species> {
        Species::ALL
            .iter()
            .copied()
            .find(|species| species.word() == word)
    }

    /// The name for people: "white oak".
    pub fn title(self) -> &'static str {
        self.facts().title
    }

    /// A typical colour of the finished wood, for the viewer.
    pub fn colour(self) -> Srgb {
        self.facts().colour
    }

    /// A typical air-dried density, when the catalog has one.
    pub fn density(self) -> Option<Density> {
        self.facts().density.map(Density)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{GRAMMAR, between, ticked};

    #[test]
    fn species_match_the_grammar() {
        let row = between(GRAMMAR, "| `species` | species word | yes | yes | ", " |");
        let listed = ticked(row);
        assert_eq!(
            listed,
            Species::ALL.iter().map(|s| s.word()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn species_round_trip_and_have_distinct_titles() {
        for species in Species::ALL {
            assert_eq!(Species::from_word(species.word()), Some(*species));
            assert_eq!(species.title().replace(' ', "_"), species.word());
            assert!(
                species
                    .density()
                    .is_some_and(|d| d.kilograms_per_cubic_metre() > 300)
            );
        }
        assert_eq!(Species::from_word("oak"), None);
    }
}
