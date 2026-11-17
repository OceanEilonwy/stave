//! Exact values and their arithmetic.
//!
//! Expressions evaluate to [`Value`]s. Numbers are exact fractions
//! ([`Rational`], [`ExactLength`], [`ExactAngle`]); there are no floats.
//! [`binary`] and [`unary`] implement the grammar's arithmetic table
//! ("Types, units and arithmetic") on values; the static checker
//! ([`crate::resolve()`]) uses the same table on types, so a well-typed
//! expression only fails here on overflow or division by zero.

use std::cmp::Ordering;
use std::fmt;

use stave_lang::ast::{BinaryOp, UnaryOp};
use stave_lang::placement::{Axis, Face};
use stave_lang::units::{ExactAngle, ExactLength, Rational};

use crate::defs::Word;
use crate::ids::{
    AssemblyId, HardwareId, JointId, KindId, PartId, ProfileId, RecipeId, RegionId, StockId,
};

/// An axis-aligned box in world coordinates, exact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExactBox {
    min: [ExactLength; 3],
    max: [ExactLength; 3],
}

fn slot(axis: Axis) -> usize {
    match axis {
        Axis::Across => 0,
        Axis::Up => 1,
        Axis::Deep => 2,
    }
}

impl ExactBox {
    /// A box from its extent along each axis; each pair is sorted.
    pub fn new(
        across: (ExactLength, ExactLength),
        up: (ExactLength, ExactLength),
        deep: (ExactLength, ExactLength),
    ) -> Self {
        let sorted = |(a, b): (ExactLength, ExactLength)| if a <= b { (a, b) } else { (b, a) };
        let (across, up, deep) = (sorted(across), sorted(up), sorted(deep));
        ExactBox {
            min: [across.0, up.0, deep.0],
            max: [across.1, up.1, deep.1],
        }
    }

    /// The low end along an axis (left, bottom or front).
    pub fn min(&self, axis: Axis) -> ExactLength {
        self.min[slot(axis)]
    }

    /// The high end along an axis (right, top or back).
    pub fn max(&self, axis: Axis) -> ExactLength {
        self.max[slot(axis)]
    }

    /// The extent along an axis; `None` on overflow.
    pub fn size(&self, axis: Axis) -> Option<ExactLength> {
        self.max(axis).checked_sub(self.min(axis))
    }

    /// The position of a face's plane.
    pub fn face(&self, face: Face) -> ExactLength {
        if face.is_positive() {
            self.max(face.axis())
        } else {
            self.min(face.axis())
        }
    }

    /// The same box with one axis replaced.
    pub fn with(mut self, axis: Axis, low: ExactLength, high: ExactLength) -> Self {
        let (low, high) = if low <= high {
            (low, high)
        } else {
            (high, low)
        };
        self.min[slot(axis)] = low;
        self.max[slot(axis)] = high;
        self
    }

    /// The part of this box that `other` covers. Where they don't overlap
    /// along an axis, the result is the flat plane between them on that
    /// axis (touching boxes give their contact face).
    pub fn intersection(&self, other: &ExactBox) -> ExactBox {
        let mut result = *self;
        for axis in Axis::ALL {
            let low = self.min(axis).max(other.min(axis));
            let high = self.max(axis).min(other.max(axis));
            result = if low <= high {
                result.with(axis, low, high)
            } else {
                result.with(axis, high, high)
            };
        }
        result
    }

    /// `true` if the two boxes share volume or touch over an area (not
    /// only along an edge or at a corner).
    pub fn meets(&self, other: &ExactBox) -> bool {
        let mut touching = 0;
        for axis in Axis::ALL {
            let low = self.min(axis).max(other.min(axis));
            let high = self.max(axis).min(other.max(axis));
            match low.cmp(&high) {
                Ordering::Greater => return false,
                Ordering::Equal => touching += 1,
                Ordering::Less => {}
            }
        }
        touching <= 1
    }

    /// The axis along which two meeting boxes only touch, if they touch
    /// rather than overlap.
    pub fn touching_axis(&self, other: &ExactBox) -> Option<Axis> {
        if !self.meets(other) {
            return None;
        }
        Axis::ALL
            .into_iter()
            .find(|axis| self.max(*axis) == other.min(*axis) || other.max(*axis) == self.min(*axis))
    }
}

/// A plane that placement measures from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AnchorValue {
    /// The axis the plane is perpendicular to.
    pub axis: Axis,
    /// Where it is along that axis.
    pub position: ExactLength,
    /// The face, when the anchor is a face (`against` and `flush` need it).
    pub face: Option<Face>,
    /// What the plane belongs to.
    pub owner: AnchorOwner,
}

/// What an anchor belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnchorOwner {
    /// A part.
    Part(PartId),
    /// A region.
    Region(RegionId),
}

/// How a front relates to its opening.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mount {
    /// In front of the opening, past each edge by these amounts.
    Overlay {
        /// Past the left edge.
        left: ExactLength,
        /// Past the right edge.
        right: ExactLength,
        /// Past the top edge.
        top: ExactLength,
        /// Past the bottom edge.
        bottom: ExactLength,
        /// Replaces the value on edges shared with a neighbour.
        meeting: Option<ExactLength>,
    },
    /// Inside the opening with an even gap all round.
    Inset {
        /// The gap.
        reveal: ExactLength,
    },
}

/// A hardware envelope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Envelope {
    /// `box(length:, width:, thickness:)`
    Box {
        /// Along the piece's length.
        length: ExactLength,
        /// Along the remaining axis.
        width: ExactLength,
        /// Standing off the mount face.
        thickness: ExactLength,
    },
    /// `cylinder(diameter:, length:)`; its axis stands off the face.
    Cylinder {
        /// Diameter.
        diameter: ExactLength,
        /// Length, standing off the face.
        length: ExactLength,
    },
}

/// A spacing for `spaced`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Spacing {
    /// `evenly`
    Evenly,
    /// `ends`, `ends(inset: d)`
    Ends(ExactLength),
    /// `pitch(d)`
    Pitch(ExactLength),
    /// `gap(d)`
    Gap(ExactLength),
}

/// `model(source, align:, sha256:)`
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModelRef {
    /// The source as written.
    pub source: String,
    /// `align: node("name")`: the node put on the kind's origin; `None`
    /// puts the model's own origin there.
    pub align: Option<String>,
    /// `sha256: "..."`: the one file the model is pinned to.
    pub sha256: Option<Sha256>,
}

/// A SHA-256 digest: the pin of `model(..., sha256: "...")`. Written as 64
/// hexadecimal digits; parsing accepts either case and shows lower case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Sha256([u8; 32]);

impl Sha256 {
    /// The digest with these bytes.
    pub const fn from_bytes(bytes: [u8; 32]) -> Sha256 {
        Sha256(bytes)
    }

    /// Its bytes.
    pub const fn bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Why a text is not a SHA-256 digest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Sha256Error {
    /// Not 64 characters long.
    #[error("it has {0} characters, not 64")]
    Length(usize),
    /// A character that is not a hexadecimal digit.
    #[error("'{0}' is not a hexadecimal digit")]
    Digit(char),
}

impl std::str::FromStr for Sha256 {
    type Err = Sha256Error;

    fn from_str(text: &str) -> Result<Sha256, Sha256Error> {
        let count = text.chars().count();
        if count != 64 {
            return Err(Sha256Error::Length(count));
        }
        let mut bytes = [0u8; 32];
        let mut digits = text.chars();
        for byte in &mut bytes {
            let mut value = 0u8;
            for _ in 0..2 {
                let digit = digits.next().ok_or(Sha256Error::Length(count))?;
                let nibble = digit
                    .to_digit(16)
                    .and_then(|n| u8::try_from(n).ok())
                    .ok_or(Sha256Error::Digit(digit))?;
                value = (value << 4) | nibble;
            }
            *byte = value;
        }
        Ok(Sha256(bytes))
    }
}

impl fmt::Display for Sha256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in &self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl serde::Serialize for Sha256 {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

/// Where a region came from, as far as `cover`'s `meeting:` needs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RegionOrigin {
    /// Not one piece of a split with nothing between its pieces.
    Plain,
    /// Piece `index` of `count` of a `split` along `axis` with nothing
    /// between the pieces.
    Split {
        /// The split axis.
        axis: Axis,
        /// Which piece, in the direction of the axis.
        index: usize,
        /// How many pieces.
        count: usize,
    },
}

/// A region's box with where it came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RegionValue {
    /// The box.
    pub bounds: ExactBox,
    /// Where it came from.
    pub origin: RegionOrigin,
}

/// A kind with settings filled in.
#[derive(Debug, Clone, PartialEq)]
pub struct KindValue {
    /// The kind.
    pub kind: KindId,
    /// Its name as written where it was named (`hw.brass_knob_26`).
    pub name: String,
    /// Settings by name, in the order given.
    pub settings: Vec<(String, Value)>,
}

impl KindValue {
    /// A setting by name.
    pub fn setting(&self, name: &str) -> Option<&Value> {
        self.settings
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v)
    }
}

/// How the boards of a part are joined: `joined(by: ...)` (grammar,
/// "Boards"). Every kind is a joint kind value; all of them are used.
#[derive(Debug, Clone, PartialEq)]
pub struct Joining {
    /// The joint kinds, in the order written.
    pub by: crate::nonempty::NonEmpty<KindValue>,
}

/// An evaluated value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// A length.
    Length(ExactLength),
    /// An angle.
    Angle(ExactAngle),
    /// A count (a whole number).
    Count(Rational),
    /// A ratio.
    Ratio(Rational),
    /// `true` or `false`.
    Bool(bool),
    /// Text.
    Text(String),
    /// One part.
    Part(PartId),
    /// Several parts.
    Parts(Vec<PartId>),
    /// A region.
    Region(RegionId),
    /// Regions not yet declared (what `opening`, `split` and `cover` make).
    Regions(Vec<RegionValue>),
    /// A stock.
    Stock(StockId),
    /// A profile.
    Profile(ProfileId),
    /// A face or mid-plane.
    Anchor(AnchorValue),
    /// A bare word.
    Word(Word),
    /// A mount.
    Mount(Mount),
    /// `turned(profile)`.
    Turned(ProfileId),
    /// `joined(by: ...)`.
    Boards(Joining),
    /// An envelope.
    Envelope(Envelope),
    /// A model.
    Model(ModelRef),
    /// A spacing.
    Spacing(Spacing),
    /// A kind value.
    Kind(KindValue),
    /// A tool kind.
    Tool(KindId),
    /// A recipe.
    Recipe(RecipeId),
    /// An assembly.
    Assembly(AssemblyId),
    /// Several assemblies (`drawer[*]`).
    Assemblies(Vec<AssemblyId>),
    /// A joint.
    Joint(JointId),
    /// A piece of hardware.
    Hardware(HardwareId),
    /// A list of other values.
    List(Vec<Value>),
}

/// Why arithmetic failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ValueError {
    /// The result is too large to represent exactly.
    #[error("the value is too large")]
    Overflow,
    /// A division by zero.
    #[error("division by zero")]
    DivisionByZero,
    /// The operands don't go together (already reported by the type
    /// checker when the types were known).
    #[error("can't {verb} {lhs} and {rhs}")]
    Mismatch {
        /// "add", "compare", ...
        verb: &'static str,
        /// The left value's description.
        lhs: &'static str,
        /// The right value's description.
        rhs: &'static str,
    },
}

impl Value {
    /// How the value is described in messages.
    pub fn description(&self) -> &'static str {
        match self {
            Value::Length(_) => "a length",
            Value::Angle(_) => "an angle",
            Value::Count(_) => "a count",
            Value::Ratio(_) => "a ratio",
            Value::Bool(_) => "true or false",
            Value::Text(_) => "a text",
            Value::Part(_) => "a part",
            Value::Parts(_) => "parts",
            Value::Region(_) | Value::Regions(_) => "a region",
            Value::Stock(_) => "a stock",
            Value::Profile(_) => "a profile",
            Value::Anchor(_) => "a face or mid-plane",
            Value::Word(_) => "a word",
            Value::Mount(_) => "a mount",
            Value::Turned(_) => "a shape",
            Value::Boards(_) => "a way of joining boards",
            Value::Envelope(_) => "an envelope",
            Value::Model(_) => "a model",
            Value::Spacing(_) => "a spacing",
            Value::Kind(_) => "a kind",
            Value::Tool(_) => "a tool",
            Value::Recipe(_) => "a recipe",
            Value::Assembly(_) | Value::Assemblies(_) => "an assembly",
            Value::Joint(_) => "a joint",
            Value::Hardware(_) => "a piece of hardware",
            Value::List(_) => "a list",
        }
    }

    /// The number of a count or ratio.
    pub fn number(&self) -> Option<Rational> {
        match self {
            Value::Count(n) | Value::Ratio(n) => Some(*n),
            _ => None,
        }
    }

    /// Every part in the value, flattening lists.
    pub fn parts(&self) -> Option<Vec<PartId>> {
        match self {
            Value::Part(part) => Some(vec![*part]),
            Value::Parts(parts) => Some(parts.clone()),
            Value::List(items) => {
                let mut all = Vec::new();
                for item in items {
                    all.extend(item.parts()?);
                }
                Some(all)
            }
            _ => None,
        }
    }
}

fn ratio(value: &Value) -> Option<Rational> {
    value.number()
}

fn mismatch(op: BinaryOp, lhs: &Value, rhs: &Value) -> ValueError {
    let verb = match op {
        BinaryOp::Add => "add",
        BinaryOp::Sub => "subtract",
        BinaryOp::Mul => "multiply",
        BinaryOp::Div | BinaryOp::In => "divide",
        BinaryOp::And | BinaryOp::Or => "combine",
        BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
            "compare"
        }
    };
    ValueError::Mismatch {
        verb,
        lhs: lhs.description(),
        rhs: rhs.description(),
    }
}

/// Counts stay counts under `+`, `-` and `*`; everything else with a ratio
/// is a ratio.
fn number(lhs: &Value, rhs: &Value, value: Rational) -> Value {
    match (lhs, rhs) {
        (Value::Count(_), Value::Count(_)) if value.is_integer() => Value::Count(value),
        _ => Value::Ratio(value),
    }
}

fn compare(op: BinaryOp, order: Ordering) -> bool {
    match op {
        BinaryOp::Eq => order == Ordering::Equal,
        BinaryOp::Ne => order != Ordering::Equal,
        BinaryOp::Lt => order == Ordering::Less,
        BinaryOp::Le => order != Ordering::Greater,
        BinaryOp::Gt => order == Ordering::Greater,
        BinaryOp::Ge => order != Ordering::Less,
        _ => false,
    }
}

/// `lhs op rhs`, exactly.
pub fn binary(op: BinaryOp, lhs: &Value, rhs: &Value) -> Result<Value, ValueError> {
    use Value::{Angle, Bool, Length};
    let overflow = ValueError::Overflow;
    match op {
        BinaryOp::And | BinaryOp::Or => match (lhs, rhs) {
            (Bool(a), Bool(b)) => Ok(Bool(if op == BinaryOp::And {
                *a && *b
            } else {
                *a || *b
            })),
            _ => Err(mismatch(op, lhs, rhs)),
        },
        BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
            let order = match (lhs, rhs) {
                (Length(a), Length(b)) => a.cmp(b),
                (Angle(a), Angle(b)) => a.cmp(b),
                (a, b) if ratio(a).is_some() && ratio(b).is_some() => ratio(a).cmp(&ratio(b)),
                (a, b) if matches!(op, BinaryOp::Eq | BinaryOp::Ne) => {
                    if a == b {
                        Ordering::Equal
                    } else {
                        Ordering::Less
                    }
                }
                _ => return Err(mismatch(op, lhs, rhs)),
            };
            Ok(Bool(compare(op, order)))
        }
        BinaryOp::Add | BinaryOp::Sub => {
            let sub = op == BinaryOp::Sub;
            match (lhs, rhs) {
                (Length(a), Length(b)) => {
                    let result = if sub {
                        a.checked_sub(*b)
                    } else {
                        a.checked_add(*b)
                    };
                    result.map(Length).ok_or(overflow)
                }
                (Angle(a), Angle(b)) => {
                    let result = if sub {
                        a.checked_sub(*b)
                    } else {
                        a.checked_add(*b)
                    };
                    result.map(Angle).ok_or(overflow)
                }
                (a, b) => match (ratio(a), ratio(b)) {
                    (Some(x), Some(y)) => {
                        let result = if sub {
                            x.checked_sub(y)
                        } else {
                            x.checked_add(y)
                        };
                        result.map(|r| number(a, b, r)).ok_or(overflow)
                    }
                    _ => Err(mismatch(op, lhs, rhs)),
                },
            }
        }
        BinaryOp::Mul => match (lhs, rhs) {
            (Length(a), b) | (b, Length(a)) if ratio(b).is_some() => a
                .checked_mul(ratio(b).unwrap_or(Rational::ONE))
                .map(Length)
                .ok_or(overflow),
            (Angle(a), b) | (b, Angle(a)) if ratio(b).is_some() => a
                .checked_mul(ratio(b).unwrap_or(Rational::ONE))
                .map(Angle)
                .ok_or(overflow),
            (a, b) => match (ratio(a), ratio(b)) {
                (Some(x), Some(y)) => x.checked_mul(y).map(|r| number(a, b, r)).ok_or(overflow),
                _ => Err(mismatch(op, lhs, rhs)),
            },
        },
        BinaryOp::In => match (lhs, rhs) {
            (Value::Count(_) | Value::Ratio(_), Value::Count(_) | Value::Ratio(_)) => {
                let (x, y) = (ratio(lhs), ratio(rhs));
                match (x, y) {
                    (Some(_), Some(y)) if y == Rational::ZERO => Err(ValueError::DivisionByZero),
                    (Some(x), Some(y)) => x.checked_div(y).map(Value::Ratio).ok_or(overflow),
                    _ => Err(mismatch(op, lhs, rhs)),
                }
            }
            _ => Err(mismatch(op, lhs, rhs)),
        },
        BinaryOp::Div => {
            let zero = |r: Rational| r == Rational::ZERO;
            match (lhs, rhs) {
                (Length(a), Length(b)) => {
                    if b.value() == Rational::ZERO {
                        return Err(ValueError::DivisionByZero);
                    }
                    a.checked_ratio(*b).map(Value::Ratio).ok_or(overflow)
                }
                (Angle(a), Angle(b)) => {
                    if b.value() == Rational::ZERO {
                        return Err(ValueError::DivisionByZero);
                    }
                    a.checked_ratio(*b).map(Value::Ratio).ok_or(overflow)
                }
                (Length(a), b) if ratio(b).is_some() => {
                    let divisor = ratio(b).unwrap_or(Rational::ONE);
                    if zero(divisor) {
                        return Err(ValueError::DivisionByZero);
                    }
                    a.checked_div(divisor).map(Length).ok_or(overflow)
                }
                (Angle(a), b) if ratio(b).is_some() => {
                    let divisor = ratio(b).unwrap_or(Rational::ONE);
                    if zero(divisor) {
                        return Err(ValueError::DivisionByZero);
                    }
                    a.checked_div(divisor).map(Angle).ok_or(overflow)
                }
                (a, b) => match (ratio(a), ratio(b)) {
                    (Some(x), Some(y)) => {
                        if zero(y) {
                            return Err(ValueError::DivisionByZero);
                        }
                        x.checked_div(y).map(Value::Ratio).ok_or(overflow)
                    }
                    _ => Err(mismatch(op, lhs, rhs)),
                },
            }
        }
    }
}

/// `op operand`, exactly.
pub fn unary(op: UnaryOp, operand: &Value) -> Result<Value, ValueError> {
    let overflow = ValueError::Overflow;
    match (op, operand) {
        (UnaryOp::Not, Value::Bool(b)) => Ok(Value::Bool(!b)),
        (UnaryOp::Neg, Value::Length(l)) => l.checked_neg().map(Value::Length).ok_or(overflow),
        (UnaryOp::Neg, Value::Angle(a)) => a.checked_neg().map(Value::Angle).ok_or(overflow),
        (UnaryOp::Neg, Value::Count(n)) => n.checked_neg().map(Value::Count).ok_or(overflow),
        (UnaryOp::Neg, Value::Ratio(n)) => n.checked_neg().map(Value::Ratio).ok_or(overflow),
        (_, other) => Err(ValueError::Mismatch {
            verb: if op == UnaryOp::Not {
                "invert"
            } else {
                "negate"
            },
            lhs: other.description(),
            rhs: "nothing",
        }),
    }
}

/// Formats an exact length in millimetres, trailing zeros trimmed; a value
/// that isn't a whole number of nanometres is shown as a fraction.
pub fn format_length(length: ExactLength) -> String {
    match length.to_length() {
        Some(whole) => whole.to_string(),
        None => format!("{}nm", length.value()),
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Length(length) => f.write_str(&format_length(*length)),
            Value::Angle(angle) => match angle.to_angle() {
                Some(angle) => write!(f, "{angle}"),
                None => write!(f, "{}µdeg", angle.value()),
            },
            Value::Count(n) | Value::Ratio(n) => write!(f, "{n}"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Text(text) => write!(f, "{text}"),
            other => f.write_str(other.description()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stave_lang::units::Length;

    fn mm(value: i64) -> Value {
        Value::Length(Length::from_mm(value).to_exact())
    }

    fn count(value: i64) -> Value {
        Value::Count(Rational::from(value))
    }

    #[test]
    fn lengths_add_scale_and_divide_exactly() {
        assert_eq!(binary(BinaryOp::Add, &mm(19), &mm(1)).unwrap(), mm(20));
        assert_eq!(binary(BinaryOp::Mul, &count(2), &mm(20)).unwrap(), mm(40));
        let third = binary(BinaryOp::Div, &mm(19), &count(3)).unwrap();
        let back = binary(BinaryOp::Mul, &third, &count(3)).unwrap();
        assert_eq!(back, mm(19));
        assert_eq!(
            binary(BinaryOp::Div, &mm(10), &mm(4)).unwrap(),
            Value::Ratio(Rational::new(5, 2).unwrap())
        );
        assert!(matches!(
            binary(BinaryOp::Add, &mm(1), &count(1)),
            Err(ValueError::Mismatch { .. })
        ));
        assert_eq!(
            binary(BinaryOp::Div, &mm(1), &count(0)),
            Err(ValueError::DivisionByZero)
        );
    }

    #[test]
    fn counts_stay_counts_and_compare_with_ratios() {
        assert_eq!(
            binary(BinaryOp::Mul, &count(2), &count(3)).unwrap(),
            count(6)
        );
        assert_eq!(
            binary(BinaryOp::Div, &count(3), &count(2)).unwrap(),
            Value::Ratio(Rational::new(3, 2).unwrap())
        );
        assert_eq!(
            binary(
                BinaryOp::Le,
                &count(1),
                &Value::Ratio(Rational::new(3, 2).unwrap())
            )
            .unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            unary(UnaryOp::Neg, &mm(3)).unwrap(),
            Value::Length(Length::from_mm(-3).to_exact())
        );
    }

    #[test]
    fn boxes_meet_when_they_touch_over_an_area() {
        let at = |a: i64, b: i64| (Length::from_mm(a).to_exact(), Length::from_mm(b).to_exact());
        let base = ExactBox::new(at(0, 900), at(0, 19), at(0, 560));
        let side = ExactBox::new(at(0, 19), at(19, 1681), at(0, 560));
        assert!(base.meets(&side));
        assert_eq!(base.touching_axis(&side), Some(Axis::Up));
        let edge_only = ExactBox::new(at(900, 919), at(19, 100), at(0, 560));
        assert!(!base.meets(&edge_only));
        let crossing = ExactBox::new(at(10, 20), at(10, 20), at(10, 20));
        assert!(base.meets(&crossing));
        assert_eq!(base.touching_axis(&crossing), None);
    }

    #[test]
    fn a_sha256_is_64_hexadecimal_digits() {
        let text = "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD";
        let digest: Sha256 = text.parse().unwrap();
        assert_eq!(digest.to_string(), text.to_ascii_lowercase());
        assert_eq!(digest.bytes()[0], 0xba);
        assert_eq!("abc".parse::<Sha256>(), Err(Sha256Error::Length(3)));
        let wrong = format!("{}g", &text[..63]);
        assert_eq!(wrong.parse::<Sha256>(), Err(Sha256Error::Digit('g')));
    }
}
