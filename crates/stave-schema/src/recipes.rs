//! The recipe step vocabulary ("Steps", "Needs").

use stave_lang::ast::TypeName;

use crate::fields::ValueType;
use crate::operations::{ArgumentDefault, ArgumentSpec};

/// A marking step, written in `mark { }`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MarkingStep {
    /// `knife(feature)`
    Knife,
    /// `pencil(feature)`
    Pencil,
    /// `line(feature)`: with the tool named by `with:`.
    Line,
    /// `gauge(measurement, on: feature)`
    Gauge,
    /// `square_around(feature)`
    SquareAround,
    /// `centre_mark(feature)`
    CentreMark,
    /// `transfer(feature)`
    Transfer,
    /// `hatch(feature)`
    Hatch,
    /// `template(feature)`
    Template,
}

impl MarkingStep {
    /// Every marking step, in the order the grammar lists them.
    pub const ALL: [MarkingStep; 9] = [
        MarkingStep::Knife,
        MarkingStep::Pencil,
        MarkingStep::Line,
        MarkingStep::Gauge,
        MarkingStep::SquareAround,
        MarkingStep::CentreMark,
        MarkingStep::Transfer,
        MarkingStep::Hatch,
        MarkingStep::Template,
    ];

    /// The step as written.
    pub fn word(self) -> &'static str {
        match self {
            MarkingStep::Knife => "knife",
            MarkingStep::Pencil => "pencil",
            MarkingStep::Line => "line",
            MarkingStep::Gauge => "gauge",
            MarkingStep::SquareAround => "square_around",
            MarkingStep::CentreMark => "centre_mark",
            MarkingStep::Transfer => "transfer",
            MarkingStep::Hatch => "hatch",
            MarkingStep::Template => "template",
        }
    }

    /// The step for a word, if it is one.
    pub fn from_word(word: &str) -> Option<MarkingStep> {
        MarkingStep::ALL.into_iter().find(|s| s.word() == word)
    }

    /// `true` if the positional argument is a measurement (only `gauge`);
    /// otherwise it is a feature.
    pub fn takes_measurement(self) -> bool {
        self == MarkingStep::Gauge
    }

    /// Named arguments besides `with:` and `note:`.
    pub fn arguments(self) -> &'static [ArgumentSpec] {
        match self {
            MarkingStep::Gauge => &[ArgumentSpec {
                name: "on",
                ty: ValueType::Type(TypeName::Text),
                default: ArgumentDefault::Required,
            }],
            MarkingStep::Knife
            | MarkingStep::Pencil
            | MarkingStep::Line
            | MarkingStep::SquareAround
            | MarkingStep::CentreMark
            | MarkingStep::Transfer
            | MarkingStep::Hatch
            | MarkingStep::Template => &[],
        }
    }
}

/// A cutting step, written in `cut { }` of a recipe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CuttingStep {
    /// `saw`
    Saw,
    /// `chop`
    Chop,
    /// `pare`
    Pare,
    /// `plane`
    Plane,
    /// `rout`
    Rout,
    /// `bore`
    Bore,
    /// `turn`
    Turn,
    /// `shave`
    Shave,
}

impl CuttingStep {
    /// Every cutting step, in the order the grammar lists them.
    pub const ALL: [CuttingStep; 8] = [
        CuttingStep::Saw,
        CuttingStep::Chop,
        CuttingStep::Pare,
        CuttingStep::Plane,
        CuttingStep::Rout,
        CuttingStep::Bore,
        CuttingStep::Turn,
        CuttingStep::Shave,
    ];

    /// The step as written.
    pub fn word(self) -> &'static str {
        match self {
            CuttingStep::Saw => "saw",
            CuttingStep::Chop => "chop",
            CuttingStep::Pare => "pare",
            CuttingStep::Plane => "plane",
            CuttingStep::Rout => "rout",
            CuttingStep::Bore => "bore",
            CuttingStep::Turn => "turn",
            CuttingStep::Shave => "shave",
        }
    }

    /// The step for a word, if it is one.
    pub fn from_word(word: &str) -> Option<CuttingStep> {
        CuttingStep::ALL.into_iter().find(|s| s.word() == word)
    }

    /// `true` if the step accepts `diameter:` (only `bore`).
    pub fn takes_diameter(self) -> bool {
        self == CuttingStep::Bore
    }
}

/// The named arguments of a cutting step: `to:`, `passes:`, `guide:`, and
/// `diameter:` for `bore`.
pub const CUTTING_STEP_ARGUMENTS: [&str; 4] = ["to", "passes", "guide", "diameter"];

/// Arguments every step accepts.
pub const COMMON_STEP_ARGUMENTS: [&str; 2] = ["with", "note"];

/// A word or call in a recipe's `needs:`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum NeedKind {
    /// `flat_face`
    FlatFace,
    /// `square_stock`
    SquareStock,
    /// `before(operation)`
    Before,
    /// `after(operation)`
    After,
}

impl NeedKind {
    /// Every need, in the order the grammar lists them.
    pub const ALL: [NeedKind; 4] = [
        NeedKind::FlatFace,
        NeedKind::SquareStock,
        NeedKind::Before,
        NeedKind::After,
    ];

    /// The need as written.
    pub fn word(self) -> &'static str {
        match self {
            NeedKind::FlatFace => "flat_face",
            NeedKind::SquareStock => "square_stock",
            NeedKind::Before => "before",
            NeedKind::After => "after",
        }
    }

    /// The need for a word, if it is one.
    pub fn from_word(word: &str) -> Option<NeedKind> {
        NeedKind::ALL.into_iter().find(|n| n.word() == word)
    }

    /// `true` for the needs written as a call with an operation.
    pub fn takes_operation(self) -> bool {
        match self {
            NeedKind::Before | NeedKind::After => true,
            NeedKind::FlatFace | NeedKind::SquareStock => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{GRAMMAR, between, call_name, table_rows, ticked};

    #[test]
    fn marking_steps_match_the_grammar() {
        let rows = table_rows("| Step | Meaning |\n|---|---|\n| `knife");
        let mut listed = vec!["knife".to_owned()];
        listed.extend(rows.iter().map(|row| call_name(&row[0])));
        // table_rows skipped the first row along with the header remainder.
        let all: Vec<String> = MarkingStep::ALL
            .iter()
            .map(|s| s.word().to_owned())
            .collect();
        assert!(
            all.iter()
                .all(|word| GRAMMAR.contains(&format!("| `{word}(feature"))
                    || GRAMMAR.contains(&format!("| `{word}(measurement")))
        );
        assert_eq!(listed.len(), all.len());
    }

    #[test]
    fn cutting_steps_match_the_grammar() {
        let sentence = between(
            GRAMMAR,
            "**Cutting steps** (in `cut { }`): ",
            ". Each takes",
        );
        assert_eq!(
            ticked(sentence),
            CuttingStep::ALL
                .iter()
                .map(|s| s.word().to_owned())
                .collect::<Vec<_>>()
        );
        let arguments = table_rows("| Argument | Type | Meaning |\n");
        let listed: Vec<String> = arguments
            .iter()
            .map(|row| row[0].trim_matches('`').trim_end_matches(':').to_owned())
            .collect();
        assert_eq!(listed, CUTTING_STEP_ARGUMENTS);
    }

    #[test]
    fn needs_match_the_grammar() {
        let rows = table_rows("| Need | Meaning |\n");
        let listed: Vec<String> = rows.iter().map(|row| call_name(&row[0])).collect();
        assert_eq!(
            listed,
            NeedKind::ALL
                .iter()
                .map(|n| n.word().to_owned())
                .collect::<Vec<_>>()
        );
    }
}
