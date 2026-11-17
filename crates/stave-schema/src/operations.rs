//! Cut operations ("Cut operations", "Features") and the face selectors a
//! `cut` is written under ("Faces to cut").

use stave_lang::ast::{CutOperation, Operation, TypeName};

use crate::fields::ValueType;

/// The value an argument takes when it is left out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArgumentDefault {
    /// The argument must be given.
    Required,
    /// Left out, it is not set (an optional feature such as `flats:`).
    Absent,
    /// A length in nanometres.
    Length(i64),
    /// An angle in micro-degrees.
    Angle(i64),
    /// A count.
    Count(u32),
    /// A truth value.
    Bool(bool),
}

/// One argument of a cut operation, selector or step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ArgumentSpec {
    /// The argument's name.
    pub name: &'static str,
    /// What it expects.
    pub ty: ValueType,
    /// Its default.
    pub default: ArgumentDefault,
}

const fn argument(name: &'static str, ty: ValueType, default: ArgumentDefault) -> ArgumentSpec {
    ArgumentSpec { name, ty, default }
}

/// The three face selectors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Selector {
    /// `end(p, facing: q)`
    End,
    /// `side(p, normal: width | thickness, from: q)`
    Side,
    /// `crossing(p, q)`
    Crossing,
}

impl Selector {
    /// Every selector, in the order the grammar lists them.
    pub const ALL: [Selector; 3] = [Selector::End, Selector::Side, Selector::Crossing];

    /// The selector's name as written after `cut`.
    pub fn word(self) -> &'static str {
        match self {
            Selector::End => "end",
            Selector::Side => "side",
            Selector::Crossing => "crossing",
        }
    }

    /// The selector for a word, if it is one.
    pub fn from_word(word: &str) -> Option<Selector> {
        Selector::ALL.into_iter().find(|s| s.word() == word)
    }

    /// The arguments: the first is positional (the part cut), the rest
    /// named, except `crossing`, whose second part is positional too.
    pub fn arguments(self) -> &'static [ArgumentSpec] {
        use ArgumentDefault::{Absent, Bool, Required};
        const PART: ValueType = ValueType::Type(TypeName::Part);
        const NORMAL: ValueType = ValueType::Words(crate::words::WordSet::PartAxis);
        const BOOL: ValueType = ValueType::Type(TypeName::Bool);
        match self {
            Selector::End => {
                const LIST: &[ArgumentSpec] = &[argument("facing", PART, Required)];
                LIST
            }
            Selector::Side => {
                const LIST: &[ArgumentSpec] = &[
                    argument("normal", NORMAL, Required),
                    argument("from", PART, Required),
                    argument("opposite", BOOL, Bool(false)),
                    argument("flip", BOOL, Bool(false)),
                ];
                LIST
            }
            Selector::Crossing => {
                const LIST: &[ArgumentSpec] = &[
                    argument("normal", NORMAL, Absent),
                    argument("opposite", BOOL, Bool(false)),
                ];
                LIST
            }
        }
    }

    /// How many positional part arguments the selector takes.
    pub fn positional_parts(self) -> usize {
        match self {
            Selector::End | Selector::Side => 1,
            Selector::Crossing => 2,
        }
    }
}

/// What a cut does to the faces of its part, for the order of operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FaceRemoval {
    /// Never removes a face.
    Never,
    /// Removes the end it is cut on.
    TheEnd,
    /// A `through` outline removes the edges it crosses.
    EdgesWhenThrough,
    /// Removes every face (turning).
    Every,
}

/// Everything the catalog knows about an operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OperationSpec {
    /// The operation.
    pub operation: Operation,
    /// Its arguments, all named.
    pub arguments: &'static [ArgumentSpec],
    /// The selectors it may be written under; empty for turning, which is
    /// not written in a `cut` block.
    pub selectors: &'static [Selector],
    /// Places on the part a recipe step can refer to.
    pub features: &'static [&'static str],
    /// Numbers a recipe step can set a tool to.
    pub measurements: &'static [&'static str],
    /// What it does to the part's faces.
    pub removes: FaceRemoval,
}

impl OperationSpec {
    /// The argument with this name.
    pub fn argument(&self, name: &str) -> Option<&'static ArgumentSpec> {
        self.arguments.iter().find(|a| a.name == name)
    }
}

/// `mark(along:, across:)` inside `outline(marks: [...])`.
pub const MARK: &str = "mark";

/// The arguments of `mark(...)`.
pub const MARK_ARGUMENTS: &[ArgumentSpec] = &[
    argument(
        "along",
        ValueType::Type(TypeName::Length),
        ArgumentDefault::Required,
    ),
    argument(
        "across",
        ValueType::Type(TypeName::Length),
        ArgumentDefault::Required,
    ),
];

/// The catalog entry of an operation.
pub fn operation(operation: Operation) -> &'static OperationSpec {
    match operation {
        Operation::Cut(cut) => cut_operation(cut),
        Operation::Turning => &TURNING,
    }
}

use ArgumentDefault as D;
use Selector::{Crossing, End, Side};
const LENGTH: ValueType = ValueType::Type(TypeName::Length);
const ANGLE: ValueType = ValueType::Type(TypeName::Angle);
const RATIO: ValueType = ValueType::Type(TypeName::Ratio);
const COUNT: ValueType = ValueType::Type(TypeName::Count);
const BOOL: ValueType = ValueType::Type(TypeName::Bool);
const DEPTH: ValueType = ValueType::Depth;
const MARKS: ValueType = ValueType::ListOf(&ValueType::Type(TypeName::Anchor));

const TURNING: OperationSpec = OperationSpec {
    operation: Operation::Turning,
    arguments: &[],
    selectors: &[],
    features: &["shape", "stations", "blank", "centre"],
    measurements: &["length", "largest"],
    removes: FaceRemoval::Every,
};

/// The catalog entry of a cut operation.
pub fn cut_operation(cut: CutOperation) -> &'static OperationSpec {
    const NOTCH: OperationSpec = OperationSpec {
        operation: Operation::Cut(CutOperation::Notch),
        arguments: &[argument("depth", DEPTH, D::Required)],
        selectors: &[Crossing],
        features: &["walls", "floor", "waste", "open_edges"],
        measurements: &["depth"],
        removes: FaceRemoval::Never,
    };
    const SLOT: OperationSpec = OperationSpec {
        operation: Operation::Cut(CutOperation::Slot),
        arguments: &[
            argument("width", LENGTH, D::Required),
            argument("depth", DEPTH, D::Required),
            argument("angle", ANGLE, D::Angle(0)),
            argument("from_centre", BOOL, D::Bool(false)),
        ],
        selectors: &[Crossing],
        features: &["walls", "floor", "waste", "open_edges"],
        measurements: &["width", "depth"],
        removes: FaceRemoval::Never,
    };
    const POCKET: OperationSpec = OperationSpec {
        operation: Operation::Cut(CutOperation::Pocket),
        arguments: &[
            argument("width", LENGTH, D::Required),
            argument("thickness", LENGTH, D::Required),
            argument("depth", DEPTH, D::Required),
        ],
        selectors: &[Crossing, End, Side],
        features: &["walls", "floor", "waste", "outline"],
        measurements: &["width", "thickness", "depth"],
        removes: FaceRemoval::Never,
    };
    const TENON: OperationSpec = OperationSpec {
        operation: Operation::Cut(CutOperation::Tenon),
        arguments: &[
            argument("length", LENGTH, D::Required),
            argument("width", LENGTH, D::Required),
            argument("thickness", LENGTH, D::Required),
        ],
        selectors: &[End],
        features: &["cheeks", "shoulders", "waste"],
        measurements: &["length", "width", "thickness"],
        removes: FaceRemoval::TheEnd,
    };
    const SPIGOT: OperationSpec = OperationSpec {
        operation: Operation::Cut(CutOperation::Spigot),
        arguments: &[
            argument("length", LENGTH, D::Required),
            argument("diameter", LENGTH, D::Required),
        ],
        selectors: &[End],
        features: &["shoulder", "waste", "centre"],
        measurements: &["length", "diameter"],
        removes: FaceRemoval::TheEnd,
    };
    const ROUND: OperationSpec = OperationSpec {
        operation: Operation::Cut(CutOperation::Round),
        arguments: &[
            argument("diameter", LENGTH, D::Required),
            argument("flats", LENGTH, D::Absent),
            argument("flats_angle", ANGLE, D::Angle(0)),
        ],
        selectors: &[Crossing],
        features: &["shoulders", "waste", "centre", "flat_faces"],
        measurements: &["diameter", "flats", "length"],
        removes: FaceRemoval::Never,
    };
    const DRILL: OperationSpec = OperationSpec {
        operation: Operation::Cut(CutOperation::Drill),
        arguments: &[
            argument("diameter", LENGTH, D::Required),
            argument("depth", DEPTH, D::Required),
            argument("count", COUNT, D::Count(1)),
        ],
        selectors: &[Crossing, End, Side],
        features: &["centre"],
        measurements: &["diameter", "depth"],
        removes: FaceRemoval::Never,
    };
    // Exactly one of `angle` and `slope` is given; the solver says so when
    // it isn't.
    const DOVETAIL_ARGUMENTS: &[ArgumentSpec] = &[
        argument("neck", LENGTH, D::Required),
        argument("depth", LENGTH, D::Required),
        argument("angle", ANGLE, D::Absent),
        argument("slope", RATIO, D::Absent),
    ];
    const DOVETAIL_SOCKET: OperationSpec = OperationSpec {
        operation: Operation::Cut(CutOperation::DovetailSocket),
        arguments: DOVETAIL_ARGUMENTS,
        selectors: &[End],
        features: &["walls", "floor", "waste", "outline"],
        measurements: &["neck", "depth", "angle", "slope"],
        removes: FaceRemoval::Never,
    };
    const DOVETAIL_TAIL: OperationSpec = OperationSpec {
        operation: Operation::Cut(CutOperation::DovetailTail),
        arguments: DOVETAIL_ARGUMENTS,
        selectors: &[End],
        features: &["cheeks", "shoulders", "waste", "outline"],
        measurements: &["neck", "depth", "angle", "slope"],
        removes: FaceRemoval::TheEnd,
    };
    const OUTLINE: OperationSpec = OperationSpec {
        operation: Operation::Cut(CutOperation::Outline),
        arguments: &[
            argument("marks", MARKS, D::Required),
            argument("depth", DEPTH, D::Required),
            argument("from_depth", LENGTH, D::Length(0)),
        ],
        selectors: &[Side],
        features: &["outline", "waste"],
        measurements: &["depth"],
        removes: FaceRemoval::EdgesWhenThrough,
    };
    match cut {
        CutOperation::Notch => &NOTCH,
        CutOperation::Slot => &SLOT,
        CutOperation::Pocket => &POCKET,
        CutOperation::Tenon => &TENON,
        CutOperation::Spigot => &SPIGOT,
        CutOperation::Round => &ROUND,
        CutOperation::Drill => &DRILL,
        CutOperation::DovetailSocket => &DOVETAIL_SOCKET,
        CutOperation::DovetailTail => &DOVETAIL_TAIL,
        CutOperation::Outline => &OUTLINE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{GRAMMAR, between, call_arguments, call_name, table_rows};

    fn words(cell: &str) -> Vec<String> {
        cell.split(',')
            .map(|w| w.trim().trim_matches('`').to_owned())
            .filter(|w| !w.is_empty())
            .collect()
    }

    #[test]
    fn cut_operations_match_the_grammar() {
        let rows = table_rows("| Operation | On | Removes |\n");
        let listed: Vec<String> = rows.iter().map(|row| call_name(&row[0])).collect();
        assert_eq!(
            listed,
            CutOperation::ALL
                .iter()
                .map(|op| op.word().to_owned())
                .collect::<Vec<_>>()
        );
        for row in &rows {
            let cut = CutOperation::from_word(&call_name(&row[0])).unwrap();
            let spec = cut_operation(cut);
            assert_eq!(
                call_arguments(&row[0]),
                spec.arguments
                    .iter()
                    .map(|a| a.name.to_owned())
                    .collect::<Vec<_>>(),
                "{}",
                cut.word()
            );
            let on = words(&row[1]);
            assert_eq!(
                on,
                spec.selectors
                    .iter()
                    .map(|s| s.word().to_owned())
                    .collect::<Vec<_>>(),
                "{}",
                cut.word()
            );
        }
    }

    #[test]
    fn features_and_measurements_match_the_grammar() {
        let rows = table_rows("| Operation | Features | Measurements |\n");
        assert_eq!(rows.len(), 11);
        for row in rows {
            let operation = Operation::from_word(row[0].trim_matches('`')).unwrap();
            let spec = super::operation(operation);
            assert_eq!(words(&row[1]), spec.features, "{}", operation.word());
            assert_eq!(words(&row[2]), spec.measurements, "{}", operation.word());
            for feature in spec.features {
                assert!(!spec.measurements.contains(feature));
            }
        }
    }

    #[test]
    fn selectors_and_face_removal_match_the_grammar() {
        let rows = table_rows("| Selector | The face | Measuring |\n");
        let listed: Vec<String> = rows.iter().map(|row| call_name(&row[0])).collect();
        assert_eq!(listed, ["end", "side", "crossing"]);
        let removes = between(
            GRAMMAR,
            "A cut **removes a face** when",
            "\n\nOrder between",
        );
        for cut in CutOperation::ALL {
            let spec = cut_operation(cut);
            if spec.removes == FaceRemoval::TheEnd {
                assert!(
                    removes.contains(&format!("`{}`", cut.word())),
                    "{}",
                    cut.word()
                );
            }
        }
        assert!(
            removes
                .contains("`notch`, `slot`, `pocket`, `drill` and `dovetail_socket` never remove")
        );
        assert!(GRAMMAR.contains("With `from_depth` greater than zero"));
        assert!(GRAMMAR.contains("`mark(along:, across:)` points"));
    }
}
