//! Call heads fixed by position that place things or put them together:
//! placement terms, spacing values, the terms of a made part's `place`, and
//! assembly steps.

/// A term in `place { }` of a part or a piece of hardware.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PlacementTerm {
    /// `ground`: all three axes.
    Ground,
    /// `against(face)`, `against(face, gap: d)`: one axis.
    Against,
    /// `flush(face)`, `flush(face, inset: d)`: one axis.
    Flush,
    /// `center(item, axis)` or `center(from: a, to: b)`: one axis.
    Center,
    /// `spaced(spacing, from: a, to: b)`: one axis, arrays only.
    Spaced,
    /// `corners(part, inset: d)`: `across` and `deep`, arrays only.
    Corners,
    /// `orient(length: dir, width: dir)`: rotation.
    Orient,
}

impl PlacementTerm {
    /// Every term, in the order the grammar lists them.
    pub const ALL: [PlacementTerm; 7] = [
        PlacementTerm::Ground,
        PlacementTerm::Against,
        PlacementTerm::Flush,
        PlacementTerm::Center,
        PlacementTerm::Spaced,
        PlacementTerm::Corners,
        PlacementTerm::Orient,
    ];

    /// The term as written.
    pub fn word(self) -> &'static str {
        match self {
            PlacementTerm::Ground => "ground",
            PlacementTerm::Against => "against",
            PlacementTerm::Flush => "flush",
            PlacementTerm::Center => "center",
            PlacementTerm::Spaced => "spaced",
            PlacementTerm::Corners => "corners",
            PlacementTerm::Orient => "orient",
        }
    }

    /// The term for a word, if it is one.
    pub fn from_word(word: &str) -> Option<PlacementTerm> {
        PlacementTerm::ALL.into_iter().find(|t| t.word() == word)
    }

    /// `true` for the terms hardware may use.
    pub fn applies_to_hardware(self) -> bool {
        match self {
            PlacementTerm::Against
            | PlacementTerm::Flush
            | PlacementTerm::Center
            | PlacementTerm::Spaced => true,
            PlacementTerm::Ground | PlacementTerm::Corners | PlacementTerm::Orient => false,
        }
    }
}

/// A spacing value of `spaced`, written as a bare word or a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SpacingKind {
    /// `evenly`
    Evenly,
    /// `ends` or `ends(inset: d)`
    Ends,
    /// `pitch(d)`
    Pitch,
    /// `gap(d)`
    Gap,
}

impl SpacingKind {
    /// Every spacing, in the order the grammar lists them.
    pub const ALL: [SpacingKind; 4] = [
        SpacingKind::Evenly,
        SpacingKind::Ends,
        SpacingKind::Pitch,
        SpacingKind::Gap,
    ];

    /// The word as written.
    pub fn word(self) -> &'static str {
        match self {
            SpacingKind::Evenly => "evenly",
            SpacingKind::Ends => "ends",
            SpacingKind::Pitch => "pitch",
            SpacingKind::Gap => "gap",
        }
    }

    /// The spacing for a word, if it is one.
    pub fn from_word(word: &str) -> Option<SpacingKind> {
        SpacingKind::ALL.into_iter().find(|s| s.word() == word)
    }
}

/// A term in the `place { }` of a part a joint makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MadePartTerm {
    /// `orient(length: a, width: b, of: role)`: required.
    Orient,
    /// `center(selector)` or `center(selector, axis)`.
    Center,
    /// `against(selector)`, `against(selector, gap: d)`.
    Against,
    /// `flush(selector)`, `flush(selector, inset: d)`.
    Flush,
}

impl MadePartTerm {
    /// Every term.
    pub const ALL: [MadePartTerm; 4] = [
        MadePartTerm::Orient,
        MadePartTerm::Center,
        MadePartTerm::Against,
        MadePartTerm::Flush,
    ];

    /// The term as written.
    pub fn word(self) -> &'static str {
        match self {
            MadePartTerm::Orient => "orient",
            MadePartTerm::Center => "center",
            MadePartTerm::Against => "against",
            MadePartTerm::Flush => "flush",
        }
    }

    /// The term for a word, if it is one.
    pub fn from_word(word: &str) -> Option<MadePartTerm> {
        MadePartTerm::ALL.into_iter().find(|t| t.word() == word)
    }
}

/// A step in a joint kind's `insert:`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AssemblyStepKind {
    /// `push(p, along: axis)`, with optional `by:` and `angle:`.
    Push,
    /// `rotate(p, by: angle, about: axis)`
    Rotate,
    /// `drive(p, along: axis)`: a part the joint makes.
    Drive,
}

impl AssemblyStepKind {
    /// Every step, in the order the grammar lists them.
    pub const ALL: [AssemblyStepKind; 3] = [
        AssemblyStepKind::Push,
        AssemblyStepKind::Rotate,
        AssemblyStepKind::Drive,
    ];

    /// The step as written.
    pub fn word(self) -> &'static str {
        match self {
            AssemblyStepKind::Push => "push",
            AssemblyStepKind::Rotate => "rotate",
            AssemblyStepKind::Drive => "drive",
        }
    }

    /// The step for a word, if it is one.
    pub fn from_word(word: &str) -> Option<AssemblyStepKind> {
        AssemblyStepKind::ALL.into_iter().find(|s| s.word() == word)
    }
}

/// `extend(role, by: d)` in a joint kind's `adds:`.
pub const EXTEND: &str = "extend";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{GRAMMAR, between, call_name, table_rows, ticked};

    #[test]
    fn placement_terms_match_the_grammar() {
        let rows = table_rows("| Term | Fixes | Meaning |\n");
        let mut listed: Vec<String> = rows.iter().map(|row| call_name(&row[0])).collect();
        listed.dedup();
        assert_eq!(
            listed,
            PlacementTerm::ALL
                .iter()
                .map(|t| t.word().to_owned())
                .collect::<Vec<_>>()
        );
        let hardware = between(GRAMMAR, "hardware's envelope. ", " don't apply");
        for term in PlacementTerm::ALL {
            assert_eq!(
                hardware.contains(&format!("`{}`", term.word())),
                !term.applies_to_hardware(),
                "{}",
                term.word()
            );
        }
    }

    #[test]
    fn spacing_values_match_the_grammar() {
        let rows = table_rows("| Spacing | Meaning |\n");
        let mut listed: Vec<String> = rows.iter().map(|row| call_name(&row[0])).collect();
        listed.dedup();
        assert_eq!(
            listed,
            SpacingKind::ALL
                .iter()
                .map(|s| s.word().to_owned())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn made_part_terms_and_steps_match_the_grammar() {
        let rows = table_rows("| Term in a made part's `place` | Meaning |\n");
        let mut listed: Vec<String> = rows
            .iter()
            .flat_map(|row| ticked(&row[0]))
            .map(|cell| call_name(&cell))
            .collect();
        listed.dedup();
        assert_eq!(
            listed,
            MadePartTerm::ALL
                .iter()
                .map(|t| t.word().to_owned())
                .collect::<Vec<_>>()
        );
        let rows = table_rows("| Step | Meaning |\n|---|---|\n| `push(p, along: axis)` |");
        assert!(!rows.is_empty());
        for step in AssemblyStepKind::ALL {
            assert!(
                GRAMMAR.contains(&format!("| `{}(p, ", step.word())),
                "{}",
                step.word()
            );
        }
        assert!(GRAMMAR.contains("`extend(role, by: d)` lengthens"));
    }
}
