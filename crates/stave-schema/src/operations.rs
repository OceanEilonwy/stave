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
    /// `end(p, facing: q)`; `facing:` may be left out
    End,
    /// `side(p, normal: width | thickness, from: q)`; `from:` may be left
    /// out
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
                const LIST: &[ArgumentSpec] = &[
                    argument("facing", PART, Absent),
                    argument("opposite", BOOL, Bool(false)),
                ];
                LIST
            }
            Selector::Side => {
                const LIST: &[ArgumentSpec] = &[
                    argument("normal", NORMAL, Required),
                    argument("from", PART, Absent),
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
    /// Removes the face it is cut on: the end, or the edge for a tongue.
    TheEnd,
    /// A `through` outline removes the edges it crosses.
    EdgesWhenThrough,
    /// Removes every face (turning).
    Every,
}

/// A place on a part that a recipe step refers to (grammar, "Features").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Feature {
    /// `shape`: the turned shape.
    Shape,
    /// `stations`: the stations along a turning.
    Stations,
    /// `blank`: the square blank.
    Blank,
    /// `centre`: the centre.
    Centre,
    /// `walls`: the walls of a recess.
    Walls,
    /// `floor`: the floor of a recess.
    Floor,
    /// `waste`: the waste.
    Waste,
    /// `open_edges`: the edges a recess opens onto.
    OpenEdges,
    /// `outline`: the outline.
    Outline,
    /// `cheeks`: the cheeks.
    Cheeks,
    /// `shoulders`: the shoulders.
    Shoulders,
    /// `shoulder`: the one shoulder of a round tenon.
    Shoulder,
    /// `flat_faces`: the flat faces left on a rounded section.
    FlatFaces,
}

impl Feature {
    /// Every feature, in the order the grammar first lists them.
    pub const ALL: [Feature; 13] = [
        Feature::Shape,
        Feature::Stations,
        Feature::Blank,
        Feature::Centre,
        Feature::Walls,
        Feature::Floor,
        Feature::Waste,
        Feature::OpenEdges,
        Feature::Outline,
        Feature::Cheeks,
        Feature::Shoulders,
        Feature::Shoulder,
        Feature::FlatFaces,
    ];

    /// The word as written.
    pub fn word(self) -> &'static str {
        match self {
            Feature::Shape => "shape",
            Feature::Stations => "stations",
            Feature::Blank => "blank",
            Feature::Centre => "centre",
            Feature::Walls => "walls",
            Feature::Floor => "floor",
            Feature::Waste => "waste",
            Feature::OpenEdges => "open_edges",
            Feature::Outline => "outline",
            Feature::Cheeks => "cheeks",
            Feature::Shoulders => "shoulders",
            Feature::Shoulder => "shoulder",
            Feature::FlatFaces => "flat_faces",
        }
    }

    /// The feature a word names, if it names one.
    pub fn from_word(word: &str) -> Option<Feature> {
        Feature::ALL.into_iter().find(|item| item.word() == word)
    }
}

/// A number of an operation that a recipe step sets a tool to (grammar, "Features").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Measurement {
    /// `length`: the length.
    Length,
    /// `largest`: the largest.
    Largest,
    /// `depth`: the depth.
    Depth,
    /// `width`: the width.
    Width,
    /// `thickness`: the thickness.
    Thickness,
    /// `diameter`: the diameter.
    Diameter,
    /// `flats`: the flats.
    Flats,
    /// `neck`: the neck.
    Neck,
    /// `angle`: the angle.
    Angle,
    /// `slope`: the slope.
    Slope,
}

impl Measurement {
    /// Every measurement, in the order the grammar first lists them.
    pub const ALL: [Measurement; 10] = [
        Measurement::Length,
        Measurement::Largest,
        Measurement::Depth,
        Measurement::Width,
        Measurement::Thickness,
        Measurement::Diameter,
        Measurement::Flats,
        Measurement::Neck,
        Measurement::Angle,
        Measurement::Slope,
    ];

    /// The word as written.
    pub fn word(self) -> &'static str {
        match self {
            Measurement::Length => "length",
            Measurement::Largest => "largest",
            Measurement::Depth => "depth",
            Measurement::Width => "width",
            Measurement::Thickness => "thickness",
            Measurement::Diameter => "diameter",
            Measurement::Flats => "flats",
            Measurement::Neck => "neck",
            Measurement::Angle => "angle",
            Measurement::Slope => "slope",
        }
    }

    /// The measurement a word names, if it names one.
    pub fn from_word(word: &str) -> Option<Measurement> {
        Measurement::ALL
            .into_iter()
            .find(|item| item.word() == word)
    }
}

/// What a step's positional argument or `to:` names: a feature or a
/// measurement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StepTarget {
    /// A place on the part.
    Feature(Feature),
    /// A number a tool is set to.
    Measurement(Measurement),
}

impl StepTarget {
    /// The word as written.
    pub fn word(self) -> &'static str {
        match self {
            StepTarget::Feature(feature) => feature.word(),
            StepTarget::Measurement(measurement) => measurement.word(),
        }
    }
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
    pub features: &'static [Feature],
    /// Numbers a recipe step can set a tool to.
    pub measurements: &'static [Measurement],
    /// What it does to the part's faces.
    pub removes: FaceRemoval,
}

impl OperationSpec {
    /// The argument with this name.
    pub fn argument(&self, name: &str) -> Option<&'static ArgumentSpec> {
        self.arguments.iter().find(|a| a.name == name)
    }
}

/// An argument a cut operation no longer takes, with what to write
/// instead: the resolver says so rather than only that the argument is
/// unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RetiredArgument {
    /// The operation.
    pub operation: CutOperation,
    /// The argument's name.
    pub name: &'static str,
    /// The help line: what to write instead.
    pub help: &'static str,
}

/// Every retired argument.
pub const RETIRED_ARGUMENTS: &[RetiredArgument] = &[RetiredArgument {
    operation: CutOperation::Drill,
    name: "count",
    help: "a drill makes one hole; for a row of holes, give the joint kind a member for what goes in them, with count: and spaced:, and drill crossing(role, member): one hole for every copy, lined up in both parts",
}];

/// The retired argument of an operation with this name, if it is one.
pub fn retired_argument(operation: CutOperation, name: &str) -> Option<&'static RetiredArgument> {
    RETIRED_ARGUMENTS
        .iter()
        .find(|retired| retired.operation == operation && retired.name == name)
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
use Feature as F;
use Measurement as M;
use Selector::{Crossing, End, Side};
const LENGTH: ValueType = ValueType::Type(TypeName::Length);
const ANGLE: ValueType = ValueType::Type(TypeName::Angle);
const RATIO: ValueType = ValueType::Type(TypeName::Ratio);
const BOOL: ValueType = ValueType::Type(TypeName::Bool);
const DEPTH: ValueType = ValueType::Depth;
const MARKS: ValueType = ValueType::ListOf(&ValueType::Type(TypeName::Anchor));

const TURNING: OperationSpec = OperationSpec {
    operation: Operation::Turning,
    arguments: &[],
    selectors: &[],
    features: &[F::Shape, F::Stations, F::Blank, F::Centre],
    measurements: &[M::Length, M::Largest],
    removes: FaceRemoval::Every,
};

/// The catalog entry of a cut operation.
pub fn cut_operation(cut: CutOperation) -> &'static OperationSpec {
    const NOTCH: OperationSpec = OperationSpec {
        operation: Operation::Cut(CutOperation::Notch),
        arguments: &[argument("depth", DEPTH, D::Required)],
        selectors: &[Crossing],
        features: &[F::Walls, F::Floor, F::Waste, F::OpenEdges],
        measurements: &[M::Depth],
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
        features: &[F::Walls, F::Floor, F::Waste, F::OpenEdges],
        measurements: &[M::Width, M::Depth],
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
        features: &[F::Walls, F::Floor, F::Waste, F::Outline],
        measurements: &[M::Width, M::Thickness, M::Depth],
        removes: FaceRemoval::Never,
    };
    const TENON: OperationSpec = OperationSpec {
        operation: Operation::Cut(CutOperation::Tenon),
        arguments: &[
            argument("length", LENGTH, D::Required),
            argument("width", LENGTH, D::Required),
            argument("thickness", LENGTH, D::Required),
        ],
        selectors: &[End, Crossing],
        features: &[F::Cheeks, F::Shoulders, F::Waste],
        measurements: &[M::Length, M::Width, M::Thickness],
        removes: FaceRemoval::TheEnd,
    };
    const SPIGOT: OperationSpec = OperationSpec {
        operation: Operation::Cut(CutOperation::Spigot),
        arguments: &[
            argument("length", LENGTH, D::Required),
            argument("diameter", LENGTH, D::Required),
        ],
        selectors: &[End],
        features: &[F::Shoulder, F::Waste, F::Centre],
        measurements: &[M::Length, M::Diameter],
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
        features: &[F::Shoulders, F::Waste, F::Centre, F::FlatFaces],
        measurements: &[M::Diameter, M::Flats, M::Length],
        removes: FaceRemoval::Never,
    };
    const DRILL: OperationSpec = OperationSpec {
        operation: Operation::Cut(CutOperation::Drill),
        arguments: &[
            argument("diameter", LENGTH, D::Required),
            argument("depth", DEPTH, D::Required),
            argument("from_depth", LENGTH, D::Length(0)),
        ],
        selectors: &[Crossing, End, Side],
        features: &[F::Centre],
        measurements: &[M::Diameter, M::Depth],
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
        selectors: &[End, Crossing],
        features: &[F::Walls, F::Floor, F::Waste, F::Outline],
        measurements: &[M::Neck, M::Depth, M::Angle, M::Slope],
        removes: FaceRemoval::Never,
    };
    const DOVETAIL_TAIL: OperationSpec = OperationSpec {
        operation: Operation::Cut(CutOperation::DovetailTail),
        arguments: DOVETAIL_ARGUMENTS,
        selectors: &[End, Crossing],
        features: &[F::Cheeks, F::Shoulders, F::Waste, F::Outline],
        measurements: &[M::Neck, M::Depth, M::Angle, M::Slope],
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
        features: &[F::Outline, F::Waste],
        measurements: &[M::Depth],
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
    fn every_feature_and_measurement_belongs_to_an_operation() {
        let specs: Vec<&OperationSpec> = Operation::all().map(super::operation).collect();
        for feature in Feature::ALL {
            assert!(
                specs.iter().any(|spec| spec.features.contains(&feature)),
                "{}",
                feature.word()
            );
            assert_eq!(Feature::from_word(feature.word()), Some(feature));
        }
        for measurement in Measurement::ALL {
            assert!(
                specs
                    .iter()
                    .any(|spec| spec.measurements.contains(&measurement)),
                "{}",
                measurement.word()
            );
            assert_eq!(
                Measurement::from_word(measurement.word()),
                Some(measurement)
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
            let features: Vec<&str> = spec.features.iter().map(|f| f.word()).collect();
            let measurements: Vec<&str> = spec.measurements.iter().map(|m| m.word()).collect();
            assert_eq!(words(&row[1]), features, "{}", operation.word());
            assert_eq!(words(&row[2]), measurements, "{}", operation.word());
            for feature in spec.features {
                assert!(Measurement::from_word(feature.word()).is_none());
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
