//! Exact lengths, angles and fractions.
//!
//! - **Evaluation is exact.** A literal such as `3/4in`, `1/3in` or `0.1mm`
//!   becomes an [`ExactLength`]: an exact [`Rational`] number of nanometres.
//!   Expressions are computed with exact fractions, so `1in / 3` or an even
//!   spacing never loses precision.
//! - **Stored values are whole.** A [`Length`] is a whole number of
//!   nanometres and an [`Angle`] a whole number of micro-degrees. That
//!   represents every metric value down to 1nm and every inch fraction down
//!   to 1/64in exactly (1in = 25,400,000nm = 2^6 * 5^5 * 127).
//! - **Rounding happens once**, when the cut list is made, with
//!   [`ExactLength::round_to`], which also reports how far the value moved.
//!
//! All arithmetic is checked: overflow and division by zero are `None`, never
//! a wrap or a panic.

use std::cmp::Ordering;
use std::fmt;

use crate::ast::{Dimension, Magnitude, Number, Quantity, Unit};

/// Nanometres in a millimetre.
pub const NM_PER_MM: i64 = 1_000_000;
/// Nanometres in an inch.
pub const NM_PER_INCH: i64 = 25_400_000;
/// Micro-degrees in a degree.
pub const MICRODEG_PER_DEG: i64 = 1_000_000;

// ---------------------------------------------------------------------------
// Rational
// ---------------------------------------------------------------------------

/// An exact fraction `numerator / denominator` with a positive denominator,
/// always in lowest terms.
///
/// Used for unitless ratios and counts, and inside [`ExactLength`] and
/// [`ExactAngle`] during evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rational {
    numerator: i128,
    denominator: i128,
}

/// Greatest common divisor of the magnitudes; zero only when both are zero.
fn gcd(a: i128, b: i128) -> u128 {
    let (mut a, mut b) = (a.unsigned_abs(), b.unsigned_abs());
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

impl Rational {
    /// Zero.
    pub const ZERO: Rational = Rational {
        numerator: 0,
        denominator: 1,
    };
    /// One.
    pub const ONE: Rational = Rational {
        numerator: 1,
        denominator: 1,
    };

    /// `numerator / denominator` in lowest terms. `None` if the denominator
    /// is zero or the reduced fraction doesn't fit.
    pub fn new(numerator: i128, denominator: i128) -> Option<Rational> {
        if denominator == 0 {
            return None;
        }
        // Reduce on magnitudes, so that i128::MIN on either side can't overflow.
        let divisor = gcd(numerator, denominator);
        let negative = (numerator < 0) != (denominator < 0);
        let magnitude = numerator.unsigned_abs() / divisor;
        let denominator = i128::try_from(denominator.unsigned_abs() / divisor).ok()?;
        let numerator = if negative {
            0i128.checked_sub_unsigned(magnitude)?
        } else {
            i128::try_from(magnitude).ok()?
        };
        Some(Rational {
            numerator,
            denominator,
        })
    }

    /// A whole number.
    pub const fn from_int(n: i128) -> Rational {
        Rational {
            numerator: n,
            denominator: 1,
        }
    }

    /// The part above the line; carries the sign.
    pub const fn numerator(self) -> i128 {
        self.numerator
    }

    /// The part below the line; always positive.
    pub const fn denominator(self) -> i128 {
        self.denominator
    }

    /// `true` for whole numbers.
    pub const fn is_integer(self) -> bool {
        self.denominator == 1
    }

    /// `true` for values below zero.
    pub const fn is_negative(self) -> bool {
        self.numerator < 0
    }

    /// `self + rhs`; `None` on overflow.
    pub fn checked_add(self, rhs: Rational) -> Option<Rational> {
        let numerator = self
            .numerator
            .checked_mul(rhs.denominator)?
            .checked_add(rhs.numerator.checked_mul(self.denominator)?)?;
        Rational::new(numerator, self.denominator.checked_mul(rhs.denominator)?)
    }

    /// `self - rhs`; `None` on overflow.
    pub fn checked_sub(self, rhs: Rational) -> Option<Rational> {
        self.checked_add(rhs.checked_neg()?)
    }

    /// `self * rhs`; `None` on overflow.
    pub fn checked_mul(self, rhs: Rational) -> Option<Rational> {
        Rational::new(
            self.numerator.checked_mul(rhs.numerator)?,
            self.denominator.checked_mul(rhs.denominator)?,
        )
    }

    /// `self / rhs`; `None` on division by zero or overflow.
    pub fn checked_div(self, rhs: Rational) -> Option<Rational> {
        Rational::new(
            self.numerator.checked_mul(rhs.denominator)?,
            self.denominator.checked_mul(rhs.numerator)?,
        )
    }

    /// `-self`; `None` on overflow.
    pub fn checked_neg(self) -> Option<Rational> {
        Some(Rational {
            numerator: self.numerator.checked_neg()?,
            denominator: self.denominator,
        })
    }

    /// Largest whole number that is not above `self`.
    pub fn floor(self) -> i128 {
        self.numerator.div_euclid(self.denominator)
    }

    /// Nearest whole number, halves rounded away from zero.
    pub fn round(self) -> i128 {
        let floor = self.floor();
        // 0 <= remainder < denominator, so neither subtraction can overflow.
        let remainder = self.numerator.rem_euclid(self.denominator);
        let rest = self.denominator - remainder;
        let up = match remainder.cmp(&rest) {
            Ordering::Greater => true,
            Ordering::Less => false,
            Ordering::Equal => !self.is_negative(),
        };
        // `floor + 1` can't overflow: a remainder above zero means the
        // denominator is at least 2, so `floor` is far from i128::MAX.
        if up { floor.saturating_add(1) } else { floor }
    }
}

impl From<i64> for Rational {
    fn from(n: i64) -> Self {
        Rational::from_int(i128::from(n))
    }
}

impl From<u64> for Rational {
    fn from(n: u64) -> Self {
        Rational::from_int(i128::from(n))
    }
}

impl PartialOrd for Rational {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Rational {
    /// Compares exactly and never overflows: cross-multiplies when that
    /// fits, and otherwise compares whole parts, then the remaining
    /// fractions as a continued fraction.
    fn cmp(&self, other: &Self) -> Ordering {
        if let (Some(left), Some(right)) = (
            self.numerator.checked_mul(other.denominator),
            other.numerator.checked_mul(self.denominator),
        ) {
            return left.cmp(&right);
        }
        let (mut a, mut b) = (*self, *other);
        let mut flipped = false;
        loop {
            let (whole_a, whole_b) = (a.floor(), b.floor());
            let order = whole_a.cmp(&whole_b);
            if order != Ordering::Equal {
                return if flipped { order.reverse() } else { order };
            }
            // Same whole part: compare the fractional parts r_a/d_a and
            // r_b/d_b (both in [0, 1)) by comparing their reciprocals the
            // other way round.
            let rest_a = a.numerator.rem_euclid(a.denominator);
            let rest_b = b.numerator.rem_euclid(b.denominator);
            match (rest_a == 0, rest_b == 0) {
                (true, true) => return Ordering::Equal,
                (true, false) => {
                    return if flipped {
                        Ordering::Greater
                    } else {
                        Ordering::Less
                    };
                }
                (false, true) => {
                    return if flipped {
                        Ordering::Less
                    } else {
                        Ordering::Greater
                    };
                }
                (false, false) => {
                    a = Rational {
                        numerator: a.denominator,
                        denominator: rest_a,
                    };
                    b = Rational {
                        numerator: b.denominator,
                        denominator: rest_b,
                    };
                    flipped = !flipped;
                }
            }
        }
    }
}

impl fmt::Display for Rational {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.denominator == 1 {
            write!(f, "{}", self.numerator)
        } else {
            write!(f, "{}/{}", self.numerator, self.denominator)
        }
    }
}

// ---------------------------------------------------------------------------
// Stored values: Length and Angle
// ---------------------------------------------------------------------------

/// A length as a whole number of nanometres: what the solved design stores.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Length(i64);

/// An angle as a whole number of micro-degrees: what the solved design stores.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Angle(i64);

impl Length {
    /// No length.
    pub const ZERO: Length = Length(0);

    /// A length from nanometres.
    pub const fn from_nm(nm: i64) -> Self {
        Length(nm)
    }

    /// The length in nanometres.
    pub const fn nm(self) -> i64 {
        self.0
    }

    /// A length from whole millimetres. Saturates at the largest length
    /// (about 9 million kilometres) instead of overflowing.
    pub const fn from_mm(mm: i64) -> Self {
        Length(mm.saturating_mul(NM_PER_MM))
    }

    /// `Length::inches(3, 4)` is 3/4in. `None` if the fraction is not a
    /// whole number of nanometres, the denominator is zero or it overflows.
    pub const fn inches(numerator: i64, denominator: i64) -> Option<Self> {
        let Some(nm) = numerator.checked_mul(NM_PER_INCH) else {
            return None;
        };
        match (nm.checked_rem(denominator), nm.checked_div(denominator)) {
            (Some(0), Some(quotient)) => Some(Length(quotient)),
            _ => None,
        }
    }

    /// The same length for exact evaluation.
    pub fn to_exact(self) -> ExactLength {
        ExactLength(Rational::from(self.0))
    }

    /// Millimetres as a float, for geometry and rendering only. This is the
    /// one place a length becomes a float; never feed the result back into
    /// the model.
    pub fn to_mm_f64(self) -> f64 {
        // Exact below 2^53 nm (about 9000 km); precision loss beyond that is
        // acceptable at the rendering boundary.
        #[allow(clippy::cast_precision_loss)]
        let nm = self.0 as f64;
        #[allow(clippy::cast_precision_loss)]
        let per_mm = NM_PER_MM as f64;
        nm / per_mm
    }

    /// `self + rhs`; `None` on overflow.
    pub fn checked_add(self, rhs: Length) -> Option<Length> {
        self.0.checked_add(rhs.0).map(Length)
    }

    /// `self - rhs`; `None` on overflow.
    pub fn checked_sub(self, rhs: Length) -> Option<Length> {
        self.0.checked_sub(rhs.0).map(Length)
    }

    /// `self * count`; `None` on overflow.
    pub fn checked_mul(self, count: i64) -> Option<Length> {
        self.0.checked_mul(count).map(Length)
    }

    /// `-self`; `None` on overflow.
    pub fn checked_neg(self) -> Option<Length> {
        self.0.checked_neg().map(Length)
    }
}

impl fmt::Display for Length {
    /// Millimetres with trailing zeros trimmed, e.g. `19.05mm`. The app
    /// formats for the project's units separately; this is for logs and tests.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let per_mm = NM_PER_MM.unsigned_abs();
        let sign = if self.0 < 0 { "-" } else { "" };
        let magnitude = self.0.unsigned_abs();
        let (whole, fraction) = (magnitude / per_mm, magnitude % per_mm);
        if fraction == 0 {
            write!(f, "{sign}{whole}mm")
        } else {
            let digits = format!("{fraction:06}");
            write!(f, "{sign}{whole}.{}mm", digits.trim_end_matches('0'))
        }
    }
}

impl Angle {
    /// No angle.
    pub const ZERO: Angle = Angle(0);

    /// An angle from micro-degrees.
    pub const fn from_microdeg(microdeg: i64) -> Self {
        Angle(microdeg)
    }

    /// The angle in micro-degrees.
    pub const fn microdeg(self) -> i64 {
        self.0
    }

    /// An angle from whole degrees. Saturates instead of overflowing.
    pub const fn from_deg(deg: i64) -> Self {
        Angle(deg.saturating_mul(MICRODEG_PER_DEG))
    }

    /// The same angle for exact evaluation.
    pub fn to_exact(self) -> ExactAngle {
        ExactAngle(Rational::from(self.0))
    }

    /// `self + rhs`; `None` on overflow.
    pub fn checked_add(self, rhs: Angle) -> Option<Angle> {
        self.0.checked_add(rhs.0).map(Angle)
    }

    /// `self - rhs`; `None` on overflow.
    pub fn checked_sub(self, rhs: Angle) -> Option<Angle> {
        self.0.checked_sub(rhs.0).map(Angle)
    }

    /// Radians as a float, for geometry and rendering only.
    pub fn to_radians_f64(self) -> f64 {
        #[allow(clippy::cast_precision_loss)]
        let microdeg = self.0 as f64;
        #[allow(clippy::cast_precision_loss)]
        let per_deg = MICRODEG_PER_DEG as f64;
        (microdeg / per_deg).to_radians()
    }
}

impl fmt::Display for Angle {
    /// Degrees with trailing zeros trimmed, e.g. `12.5deg`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let per_deg = MICRODEG_PER_DEG.unsigned_abs();
        let sign = if self.0 < 0 { "-" } else { "" };
        let magnitude = self.0.unsigned_abs();
        let (whole, fraction) = (magnitude / per_deg, magnitude % per_deg);
        if fraction == 0 {
            write!(f, "{sign}{whole}deg")
        } else {
            let digits = format!("{fraction:06}");
            write!(f, "{sign}{whole}.{}deg", digits.trim_end_matches('0'))
        }
    }
}

// ---------------------------------------------------------------------------
// Values during evaluation: ExactLength and ExactAngle
// ---------------------------------------------------------------------------

/// A length during evaluation: an exact fraction of nanometres.
///
/// The methods are the grammar's arithmetic table: lengths add and subtract,
/// scale by a ratio, and divide to give a ratio. There is no way to add a
/// length to an angle or to a bare number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExactLength(Rational);

/// An angle during evaluation: an exact fraction of micro-degrees.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExactAngle(Rational);

macro_rules! exact_arithmetic {
    ($exact:ident, $unit:literal) => {
        impl $exact {
            /// Zero.
            pub const ZERO: $exact = $exact(Rational::ZERO);

            #[doc = concat!("A value from an exact number of ", $unit, ".")]
            pub const fn new(value: Rational) -> Self {
                $exact(value)
            }

            #[doc = concat!("The exact number of ", $unit, ".")]
            pub const fn value(self) -> Rational {
                self.0
            }

            /// `self + rhs`; `None` on overflow.
            pub fn checked_add(self, rhs: $exact) -> Option<$exact> {
                self.0.checked_add(rhs.0).map($exact)
            }

            /// `self - rhs`; `None` on overflow.
            pub fn checked_sub(self, rhs: $exact) -> Option<$exact> {
                self.0.checked_sub(rhs.0).map($exact)
            }

            /// `-self`; `None` on overflow.
            pub fn checked_neg(self) -> Option<$exact> {
                self.0.checked_neg().map($exact)
            }

            /// `self * ratio`; `None` on overflow.
            pub fn checked_mul(self, ratio: Rational) -> Option<$exact> {
                self.0.checked_mul(ratio).map($exact)
            }

            /// `self / ratio`; `None` on division by zero or overflow.
            pub fn checked_div(self, ratio: Rational) -> Option<$exact> {
                self.0.checked_div(ratio).map($exact)
            }

            /// `self / rhs` as a ratio; `None` on division by zero or overflow.
            pub fn checked_ratio(self, rhs: $exact) -> Option<Rational> {
                self.0.checked_div(rhs.0)
            }
        }
    };
}

exact_arithmetic!(ExactLength, "nanometres");
exact_arithmetic!(ExactAngle, "micro-degrees");

impl ExactLength {
    /// The stored form, if this is a whole number of nanometres that fits.
    pub fn to_length(self) -> Option<Length> {
        if self.0.is_integer() {
            i64::try_from(self.0.numerator()).ok().map(Length)
        } else {
            None
        }
    }

    /// Round to the nearest multiple of `resolution`, halves away from zero.
    /// `None` if the result overflows.
    pub fn round_to(self, resolution: Resolution) -> Option<Rounded> {
        let step = resolution.step();
        let steps = self.0.checked_div(Rational::from(step.nm()))?.round();
        let nm = i64::try_from(steps.checked_mul(i128::from(step.nm()))?).ok()?;
        let length = Length(nm);
        Some(Rounded {
            length,
            moved_by: length.to_exact().checked_sub(self)?,
        })
    }

    /// Round to the nearest whole nanometre, for storing a solved value.
    pub fn to_nearest_length(self) -> Option<Length> {
        i64::try_from(self.0.round()).ok().map(Length)
    }
}

impl From<Length> for ExactLength {
    fn from(length: Length) -> Self {
        length.to_exact()
    }
}

impl ExactAngle {
    /// The stored form, if this is a whole number of micro-degrees that fits.
    pub fn to_angle(self) -> Option<Angle> {
        if self.0.is_integer() {
            i64::try_from(self.0.numerator()).ok().map(Angle)
        } else {
            None
        }
    }

    /// Round to the nearest whole micro-degree, for storing a solved value.
    pub fn to_nearest_angle(self) -> Option<Angle> {
        i64::try_from(self.0.round()).ok().map(Angle)
    }
}

impl From<Angle> for ExactAngle {
    fn from(angle: Angle) -> Self {
        angle.to_exact()
    }
}

// ---------------------------------------------------------------------------
// Rounding for the cut list
// ---------------------------------------------------------------------------

/// The step the cut list rounds to: a project's `resolution:`. Always
/// positive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Resolution(Length);

impl Resolution {
    /// The default `resolution:` of a project: 0.5mm.
    pub const DEFAULT: Resolution = Resolution(Length(NM_PER_MM / 2));

    /// `None` unless `step` is longer than zero.
    pub fn new(step: Length) -> Option<Self> {
        (step > Length::ZERO).then_some(Resolution(step))
    }

    /// The rounding step.
    pub fn step(self) -> Length {
        self.0
    }
}

/// A value rounded to the shop's resolution, with how far it moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rounded {
    /// The rounded value.
    pub length: Length,
    /// `length - exact`. Zero when nothing was rounded.
    pub moved_by: ExactLength,
}

// ---------------------------------------------------------------------------
// Literal conversion
// ---------------------------------------------------------------------------

/// Why a literal can't become a value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnitError {
    /// A length was expected but the literal is an angle, or the other way
    /// round.
    WrongDimension {
        /// What was expected.
        expected: Dimension,
        /// The unit that was written.
        found: Unit,
    },
    /// Too large to represent.
    Overflow,
}

impl fmt::Display for UnitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UnitError::WrongDimension { expected, found } => write!(
                f,
                "expected {}, found a value in '{}'",
                match expected {
                    Dimension::Length => "a length",
                    Dimension::Angle => "an angle",
                },
                found.suffix()
            ),
            UnitError::Overflow => write!(f, "value is too large"),
        }
    }
}

impl std::error::Error for UnitError {}

/// Nanometres in one of a length unit; `None` for `deg`.
fn nm_per_unit(unit: Unit) -> Option<i64> {
    match unit {
        Unit::Millimeter => Some(NM_PER_MM),
        Unit::Centimeter => Some(10 * NM_PER_MM),
        Unit::Meter => Some(1_000 * NM_PER_MM),
        Unit::Inch => Some(NM_PER_INCH),
        Unit::Foot => Some(12 * NM_PER_INCH),
        Unit::Degree => None,
    }
}

/// Convert a length literal to its exact value. Every literal has one:
/// `1/3in` is exactly a third of an inch.
pub fn quantity_to_length(quantity: Quantity) -> Result<ExactLength, UnitError> {
    let per_unit = nm_per_unit(quantity.unit).ok_or(UnitError::WrongDimension {
        expected: Dimension::Length,
        found: quantity.unit,
    })?;
    magnitude_to_rational(quantity.magnitude)?
        .checked_mul(Rational::from(per_unit))
        .map(ExactLength)
        .ok_or(UnitError::Overflow)
}

/// Convert an angle literal to its exact value.
pub fn quantity_to_angle(quantity: Quantity) -> Result<ExactAngle, UnitError> {
    match quantity.unit {
        Unit::Degree => magnitude_to_rational(quantity.magnitude)?
            .checked_mul(Rational::from(MICRODEG_PER_DEG))
            .map(ExactAngle)
            .ok_or(UnitError::Overflow),
        Unit::Millimeter | Unit::Centimeter | Unit::Meter | Unit::Inch | Unit::Foot => {
            Err(UnitError::WrongDimension {
                expected: Dimension::Angle,
                found: quantity.unit,
            })
        }
    }
}

/// Convert a unitless literal to an exact [`Rational`].
pub fn number_to_rational(number: Number) -> Result<Rational, UnitError> {
    match number {
        Number::Integer(n) => Ok(Rational::from(n)),
        Number::Decimal(decimal) => decimal_to_rational(decimal.digits, decimal.scale),
    }
}

fn decimal_to_rational(digits: u64, scale: u8) -> Result<Rational, UnitError> {
    let denominator = 10i128
        .checked_pow(u32::from(scale))
        .ok_or(UnitError::Overflow)?;
    Rational::new(i128::from(digits), denominator).ok_or(UnitError::Overflow)
}

fn magnitude_to_rational(magnitude: Magnitude) -> Result<Rational, UnitError> {
    match magnitude {
        Magnitude::Integer(n) => Ok(Rational::from(n)),
        Magnitude::Decimal(decimal) => decimal_to_rational(decimal.digits, decimal.scale),
        Magnitude::Fraction(fraction) => {
            let denominator = i128::from(fraction.denominator.get());
            let numerator = i128::from(fraction.whole)
                .checked_mul(denominator)
                .and_then(|whole| whole.checked_add(i128::from(fraction.numerator)))
                .ok_or(UnitError::Overflow)?;
            Rational::new(numerator, denominator).ok_or(UnitError::Overflow)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Decimal, Fraction};
    use std::num::NonZeroU64;

    fn fraction(whole: u64, numerator: u64, denominator: u64, unit: Unit) -> Quantity {
        Quantity {
            magnitude: Magnitude::Fraction(Fraction {
                whole,
                numerator,
                denominator: NonZeroU64::new(denominator).unwrap(),
            }),
            unit,
        }
    }

    fn decimal(digits: u64, scale: u8, unit: Unit) -> Quantity {
        Quantity {
            magnitude: Magnitude::Decimal(Decimal { digits, scale }),
            unit,
        }
    }

    fn whole(n: u64, unit: Unit) -> Quantity {
        Quantity {
            magnitude: Magnitude::Integer(n),
            unit,
        }
    }

    fn r(numerator: i128, denominator: i128) -> Rational {
        Rational::new(numerator, denominator).unwrap()
    }

    fn nm(n: i64) -> ExactLength {
        Length::from_nm(n).to_exact()
    }

    #[test]
    fn inch_fractions_are_exact() {
        assert_eq!(
            quantity_to_length(fraction(0, 3, 4, Unit::Inch)),
            Ok(nm(19_050_000))
        );
        assert_eq!(
            quantity_to_length(fraction(1, 1, 2, Unit::Inch)),
            Ok(nm(38_100_000))
        );
        assert_eq!(
            quantity_to_length(fraction(0, 1, 64, Unit::Inch)),
            Ok(nm(396_875))
        );
        assert_eq!(
            quantity_to_length(fraction(0, 1, 5, Unit::Inch)),
            Ok(nm(5_080_000))
        );
        assert_eq!(Length::inches(3, 4), Some(Length::from_nm(19_050_000)));
        assert_eq!(Length::inches(1, 3), None);
        assert_eq!(Length::inches(1, 0), None);
        assert_eq!(Length::inches(i64::MAX, 1), None);
    }

    #[test]
    fn a_third_of_an_inch_is_a_valid_literal() {
        let third = quantity_to_length(fraction(0, 1, 3, Unit::Inch)).unwrap();
        assert_eq!(third.value(), r(25_400_000, 3));
        assert_eq!(third.to_length(), None);
        assert_eq!(third.to_nearest_length(), Some(Length::from_nm(8_466_667)));
        assert_eq!(
            third.checked_mul(Rational::from_int(3)),
            Some(nm(NM_PER_INCH))
        );
    }

    #[test]
    fn metric_decimals() {
        assert_eq!(
            quantity_to_length(decimal(185, 1, Unit::Millimeter)),
            Ok(nm(18_500_000))
        );
        assert_eq!(
            quantity_to_length(whole(2, Unit::Centimeter)),
            Ok(nm(20_000_000))
        );
        assert_eq!(
            quantity_to_length(whole(1, Unit::Meter)),
            Ok(nm(1_000_000_000))
        );
        assert_eq!(
            quantity_to_length(whole(1, Unit::Foot)),
            Ok(nm(304_800_000))
        );
        let fine = quantity_to_length(decimal(1_234_567, 7, Unit::Millimeter)).unwrap();
        assert_eq!(fine.value(), r(1_234_567, 10));
        assert_eq!(fine.to_nearest_length(), Some(Length::from_nm(123_457)));
    }

    #[test]
    fn mixed_units_agree() {
        assert_eq!(
            quantity_to_length(whole(1, Unit::Inch)),
            quantity_to_length(decimal(254, 1, Unit::Millimeter))
        );
    }

    #[test]
    fn dimension_mismatch() {
        let degrees = whole(90, Unit::Degree);
        let error = quantity_to_length(degrees).unwrap_err();
        assert_eq!(
            error,
            UnitError::WrongDimension {
                expected: Dimension::Length,
                found: Unit::Degree
            }
        );
        assert_eq!(
            error.to_string(),
            "expected a length, found a value in 'deg'"
        );
        assert_eq!(
            quantity_to_angle(degrees),
            Ok(Angle::from_deg(90).to_exact())
        );
        assert_eq!(
            quantity_to_angle(whole(3, Unit::Inch))
                .unwrap_err()
                .to_string(),
            "expected an angle, found a value in 'in'"
        );
        assert_eq!(
            quantity_to_angle(decimal(125, 1, Unit::Degree))
                .unwrap()
                .to_angle(),
            Some(Angle::from_microdeg(12_500_000))
        );
    }

    #[test]
    fn huge_literals_overflow_instead_of_wrapping() {
        let huge = Quantity {
            magnitude: Magnitude::Integer(u64::MAX),
            unit: Unit::Meter,
        };
        let exact = quantity_to_length(huge).unwrap();
        assert_eq!(exact.to_length(), None);
        assert_eq!(exact.to_nearest_length(), None);
        assert_eq!(decimal_to_rational(1, 200), Err(UnitError::Overflow));
        assert_eq!(UnitError::Overflow.to_string(), "value is too large");
    }

    #[test]
    fn display_trims_zeros() {
        assert_eq!(Length::inches(3, 4).unwrap().to_string(), "19.05mm");
        assert_eq!(Length::from_mm(18).to_string(), "18mm");
        assert_eq!(Length::from_nm(12_700_000).to_string(), "12.7mm");
        assert_eq!(
            Length::inches(1, 64)
                .unwrap()
                .checked_neg()
                .unwrap()
                .to_string(),
            "-0.396875mm"
        );
        assert_eq!(
            Length::from_nm(i64::MIN).to_string(),
            "-9223372036854.775808mm"
        );
        assert_eq!(Angle::from_deg(110).to_string(), "110deg");
        assert_eq!(Angle::from_microdeg(-12_500_000).to_string(), "-12.5deg");
    }

    #[test]
    fn rationals_stay_exact() {
        assert_eq!(r(2, 4), r(1, 2));
        assert_eq!(r(1, -2), r(-1, 2));
        assert_eq!(r(1, -2).denominator(), 2);
        assert_eq!(r(1, 3).checked_add(r(1, 6)), Some(r(1, 2)));
        assert_eq!(r(1, 2).checked_sub(r(1, 3)), Some(r(1, 6)));
        assert_eq!(r(2, 3).checked_mul(r(3, 4)), Some(r(1, 2)));
        assert_eq!(r(1, 2).checked_div(Rational::ZERO), None);
        assert_eq!(Rational::new(1, 0), None);
        assert_eq!(Rational::new(i128::MIN, i128::MIN), Some(Rational::ONE));
        assert_eq!(Rational::new(1, i128::MIN), None);
        assert!(r(1, 3) < r(1, 2));
        assert!(r(-1, 2) < r(-1, 3));
        assert_eq!(r(6, 3).to_string(), "2");
        assert_eq!(r(-3, 6).to_string(), "-1/2");
        assert_eq!(
            number_to_rational(Number::Decimal(Decimal {
                digits: 75,
                scale: 2
            })),
            Ok(r(3, 4))
        );
        assert_eq!(
            number_to_rational(Number::Integer(4)),
            Ok(Rational::from_int(4))
        );
    }

    #[test]
    fn rounding_is_half_away_from_zero() {
        assert_eq!(r(5, 2).round(), 3);
        assert_eq!(r(-5, 2).round(), -3);
        assert_eq!(r(7, 3).round(), 2);
        assert_eq!(r(-7, 3).round(), -2);
        assert_eq!(r(8, 3).round(), 3);
        assert_eq!(r(-8, 3).round(), -3);
        assert_eq!(Rational::from_int(i128::MAX).round(), i128::MAX);
        assert_eq!(Rational::from_int(i128::MIN).round(), i128::MIN);
        assert_eq!(r(-7, 2).floor(), -4);
        assert_eq!(r(7, 2).floor(), 3);
    }

    #[test]
    fn comparison_never_overflows() {
        let big = i128::MAX;
        // Cross-multiplying these overflows; the continued-fraction path must agree
        // with what exact arithmetic says.
        assert_eq!(r(big - 1, big).cmp(&r(big - 2, big - 1)), Ordering::Greater);
        assert_eq!(r(big - 2, big - 1).cmp(&r(big - 1, big)), Ordering::Less);
        assert_eq!(
            r(-(big - 1), big).cmp(&r(-(big - 2), big - 1)),
            Ordering::Less
        );
        assert_eq!(r(big, 3).cmp(&r(big, 3)), Ordering::Equal);
        assert_eq!(r(big, 2).cmp(&r(big - 1, 2)), Ordering::Greater);
        assert_eq!(r(big, 5).cmp(&r(big, 7)), Ordering::Greater);
        assert_eq!(
            Rational::from_int(i128::MIN).cmp(&Rational::from_int(i128::MAX)),
            Ordering::Less
        );
    }

    #[test]
    fn exact_lengths_follow_the_arithmetic_table() {
        let inch = quantity_to_length(whole(1, Unit::Inch)).unwrap();
        let third = inch.checked_div(Rational::from_int(3)).unwrap();
        assert_eq!(third.to_length(), None);
        assert_eq!(
            third
                .checked_add(third)
                .and_then(|sum| sum.checked_add(third)),
            Some(inch)
        );
        assert_eq!(inch.checked_sub(inch), Some(ExactLength::ZERO));
        assert_eq!(inch.checked_ratio(third), Some(Rational::from_int(3)));
        assert_eq!(inch.checked_ratio(ExactLength::ZERO), None);
        assert_eq!(inch.checked_div(Rational::ZERO), None);
        assert_eq!(
            inch.checked_neg().and_then(ExactLength::to_length),
            Length::inches(-1, 1)
        );
        assert!(third < inch);
        assert_eq!(
            ExactLength::from(Length::from_mm(19)).to_length(),
            Some(Length::from_mm(19))
        );
        assert_eq!(ExactLength::new(r(1, 2)).value(), r(1, 2));
        let right_angle = ExactAngle::from(Angle::from_deg(90));
        assert_eq!(
            right_angle
                .checked_div(Rational::from_int(2))
                .and_then(ExactAngle::to_angle),
            Some(Angle::from_deg(45))
        );
        assert_eq!(
            right_angle
                .checked_div(Rational::from_int(7))
                .and_then(ExactAngle::to_nearest_angle),
            Some(Angle::from_microdeg(12_857_143))
        );
    }

    #[test]
    fn one_inch_divided_by_three_rounds_once() {
        let third = Length::inches(1, 1)
            .unwrap()
            .to_exact()
            .checked_div(Rational::from_int(3))
            .unwrap();
        let rounded = third.round_to(Resolution::DEFAULT).unwrap();
        assert_eq!(rounded.length, Length::from_nm(8_500_000));
        assert_eq!(rounded.moved_by.value(), r(100_000, 3));

        let exact = Length::from_mm(19)
            .to_exact()
            .round_to(Resolution::DEFAULT)
            .unwrap();
        assert_eq!(exact.length, Length::from_mm(19));
        assert_eq!(exact.moved_by, ExactLength::ZERO);

        let negative = nm(-250_000).round_to(Resolution::DEFAULT).unwrap();
        assert_eq!(negative.length, Length::from_nm(-500_000));
    }

    #[test]
    fn a_resolution_is_positive() {
        assert_eq!(Resolution::DEFAULT.step(), Length::from_nm(500_000));
        assert_eq!(Resolution::new(Length::ZERO), None);
        assert_eq!(Resolution::new(Length::from_mm(-1)), None);
        assert_eq!(
            Resolution::new(Length::from_mm(1)).map(Resolution::step),
            Some(Length::from_mm(1))
        );
    }

    #[test]
    fn checked_arithmetic_reports_overflow() {
        assert_eq!(
            Length::from_nm(i64::MAX).checked_add(Length::from_nm(1)),
            None
        );
        assert_eq!(Length::from_nm(i64::MIN).checked_neg(), None);
        assert_eq!(Length::from_nm(i64::MAX).checked_mul(2), None);
        assert_eq!(Length::from_mm(2).checked_mul(3), Some(Length::from_mm(6)));
        assert_eq!(
            Length::from_mm(2).checked_sub(Length::from_mm(3)),
            Some(Length::from_mm(-1))
        );
        assert_eq!(Length::from_mm(i64::MAX), Length::from_nm(i64::MAX));
        assert_eq!(
            Angle::from_microdeg(i64::MAX).checked_add(Angle::from_microdeg(1)),
            None
        );
        assert_eq!(
            Angle::from_deg(90).checked_sub(Angle::from_deg(20)),
            Some(Angle::from_deg(70))
        );
        assert_eq!(
            Rational::from_int(i128::MAX).checked_add(Rational::ONE),
            None
        );
        assert_eq!(Rational::from_int(i128::MIN).checked_neg(), None);
    }

    #[test]
    fn floats_only_at_the_rendering_boundary() {
        assert!((Length::inches(3, 4).unwrap().to_mm_f64() - 19.05).abs() < 1e-12);
        assert!((Angle::from_deg(180).to_radians_f64() - std::f64::consts::PI).abs() < 1e-12);
        assert_eq!(Angle::from_deg(110).microdeg(), 110_000_000);
        assert_eq!(Length::from_mm(1).nm(), NM_PER_MM);
    }
}
