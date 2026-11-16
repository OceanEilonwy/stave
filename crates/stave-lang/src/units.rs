//! Exact lengths, angles and fractions.
//!
//! - Stored values: lengths are `i64` nanometers, which represents every
//!   metric value down to 1nm and every inch fraction down to 1/64in exactly
//!   (1in = 25,400,000nm = 2^6 * 5^5 * 127). Angles are `i64` micro-degrees.
//! - Evaluation: expressions are computed with [`Rational`], so `1in / 3` or
//!   an even spacing never loses precision. A value is rounded once, when the
//!   cut list is made, with [`round_to`], which also reports how far it moved.
//! - Literals that can't be stored exactly are rejected with the nearest
//!   storable value, instead of being silently rounded.
//!
//! All arithmetic is checked: overflow is an error, never a wrap or a panic.

use std::cmp::Ordering;
use std::fmt;

use crate::ast::{Dimension, Magnitude, Number, Quantity, Unit};

pub const NM_PER_MM: i64 = 1_000_000;
pub const NM_PER_INCH: i64 = 25_400_000;
pub const MICRODEG_PER_DEG: i64 = 1_000_000;

// ---------------------------------------------------------------------------
// Rational
// ---------------------------------------------------------------------------

/// An exact fraction `num / den` with `den > 0`, always in lowest terms.
///
/// Used for unitless ratios and counts, and for lengths during evaluation
/// (as a number of nanometers).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rational {
    num: i128,
    den: i128,
}

fn gcd(mut a: i128, mut b: i128) -> i128 {
    a = a.abs();
    b = b.abs();
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

impl Rational {
    pub const ZERO: Rational = Rational { num: 0, den: 1 };
    pub const ONE: Rational = Rational { num: 1, den: 1 };

    /// `None` if `den` is zero.
    pub fn new(num: i128, den: i128) -> Option<Rational> {
        if den == 0 {
            return None;
        }
        let sign = if den < 0 { -1 } else { 1 };
        let g = gcd(num, den).max(1);
        Some(Rational { num: sign * num / g, den: sign * den / g })
    }

    pub const fn from_int(n: i128) -> Rational {
        Rational { num: n, den: 1 }
    }

    pub fn numerator(self) -> i128 {
        self.num
    }

    pub fn denominator(self) -> i128 {
        self.den
    }

    pub fn is_integer(self) -> bool {
        self.den == 1
    }

    pub fn checked_add(self, rhs: Rational) -> Option<Rational> {
        let num = self.num.checked_mul(rhs.den)?.checked_add(rhs.num.checked_mul(self.den)?)?;
        Rational::new(num, self.den.checked_mul(rhs.den)?)
    }

    pub fn checked_sub(self, rhs: Rational) -> Option<Rational> {
        self.checked_add(rhs.checked_neg()?)
    }

    pub fn checked_mul(self, rhs: Rational) -> Option<Rational> {
        Rational::new(self.num.checked_mul(rhs.num)?, self.den.checked_mul(rhs.den)?)
    }

    /// `None` on division by zero or overflow.
    pub fn checked_div(self, rhs: Rational) -> Option<Rational> {
        Rational::new(self.num.checked_mul(rhs.den)?, self.den.checked_mul(rhs.num)?)
    }

    pub fn checked_neg(self) -> Option<Rational> {
        Some(Rational { num: self.num.checked_neg()?, den: self.den })
    }

    /// Nearest integer, halves rounded away from zero.
    pub fn round(self) -> i128 {
        let twice = 2 * self.num.abs() + self.den;
        let magnitude = twice / (2 * self.den);
        if self.num < 0 { -magnitude } else { magnitude }
    }
}

impl PartialOrd for Rational {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Rational {
    /// Compares by cross-multiplying. Values in a design (nanometers with small
    /// denominators) stay far below where this could overflow.
    fn cmp(&self, other: &Self) -> Ordering {
        (self.num * other.den).cmp(&(other.num * self.den))
    }
}

impl fmt::Display for Rational {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.den == 1 { write!(f, "{}", self.num) } else { write!(f, "{}/{}", self.num, self.den) }
    }
}

// ---------------------------------------------------------------------------
// Length and Angle
// ---------------------------------------------------------------------------

/// An exact length in nanometers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Length(i64);

/// An exact angle in micro-degrees.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Angle(i64);

impl Length {
    pub const ZERO: Length = Length(0);

    pub const fn from_nm(nm: i64) -> Self {
        Length(nm)
    }

    pub const fn nm(self) -> i64 {
        self.0
    }

    pub const fn from_mm(mm: i64) -> Self {
        Length(mm * NM_PER_MM)
    }

    /// `Length::inches(3, 4)` is 3/4in. Panics if not exact; for constants in
    /// code only. User input goes through [`quantity_to_length`].
    pub const fn inches(numerator: i64, denominator: i64) -> Self {
        let nm = numerator * NM_PER_INCH;
        assert!(nm % denominator == 0, "inch fraction not exactly representable");
        Length(nm / denominator)
    }

    /// The length in nanometers, for exact evaluation.
    pub fn to_rational(self) -> Rational {
        Rational::from_int(self.0 as i128)
    }

    /// For rendering only. Never feed this back into the model.
    pub fn to_mm_f64(self) -> f64 {
        self.0 as f64 / NM_PER_MM as f64
    }

    pub fn checked_add(self, rhs: Length) -> Option<Length> {
        self.0.checked_add(rhs.0).map(Length)
    }

    pub fn checked_sub(self, rhs: Length) -> Option<Length> {
        self.0.checked_sub(rhs.0).map(Length)
    }

    pub fn checked_mul(self, count: i64) -> Option<Length> {
        self.0.checked_mul(count).map(Length)
    }

    pub fn checked_neg(self) -> Option<Length> {
        self.0.checked_neg().map(Length)
    }
}

impl fmt::Display for Length {
    /// Millimeters with trailing zeros trimmed, e.g. `19.05mm`. The app formats
    /// for the project's units separately; this is for logs and tests.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sign = if self.0 < 0 { "-" } else { "" };
        let abs = self.0.unsigned_abs();
        let whole = abs / NM_PER_MM as u64;
        let frac = abs % NM_PER_MM as u64;
        if frac == 0 {
            write!(f, "{sign}{whole}mm")
        } else {
            let digits = format!("{frac:06}");
            write!(f, "{sign}{whole}.{}mm", digits.trim_end_matches('0'))
        }
    }
}

impl Angle {
    pub const ZERO: Angle = Angle(0);

    pub const fn from_microdeg(microdeg: i64) -> Self {
        Angle(microdeg)
    }

    pub const fn microdeg(self) -> i64 {
        self.0
    }

    pub const fn from_deg(deg: i64) -> Self {
        Angle(deg * MICRODEG_PER_DEG)
    }

    pub fn checked_add(self, rhs: Angle) -> Option<Angle> {
        self.0.checked_add(rhs.0).map(Angle)
    }

    pub fn checked_sub(self, rhs: Angle) -> Option<Angle> {
        self.0.checked_sub(rhs.0).map(Angle)
    }

    /// For rendering only.
    pub fn to_radians_f64(self) -> f64 {
        (self.0 as f64 / MICRODEG_PER_DEG as f64).to_radians()
    }
}

// ---------------------------------------------------------------------------
// Rounding for the cut list
// ---------------------------------------------------------------------------

/// A value rounded to the shop's resolution, with how far it moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rounded {
    pub length: Length,
    /// `length - exact`, in nanometers. Zero when nothing was rounded.
    pub moved_by: Rational,
}

/// Round an exact length (in nanometers) to the nearest multiple of
/// `resolution`, halves away from zero. `None` if `resolution` is not
/// positive or the result overflows.
pub fn round_to(exact_nm: Rational, resolution: Length) -> Option<Rounded> {
    if resolution.0 <= 0 {
        return None;
    }
    let steps = exact_nm.checked_div(resolution.to_rational())?.round();
    let nm = i64::try_from(steps.checked_mul(resolution.0 as i128)?).ok()?;
    let length = Length(nm);
    Some(Rounded { length, moved_by: length.to_rational().checked_sub(exact_nm)? })
}

// ---------------------------------------------------------------------------
// Literal conversion
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnitError {
    /// A length was expected but the literal is an angle, or vice versa.
    WrongDimension { expected: Dimension, found: Unit },
    /// The literal can't be stored exactly. `nearest` is the closest value
    /// that can (nanometers for lengths, micro-degrees for angles).
    Inexact { dimension: Dimension, nearest: i64 },
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
            UnitError::Inexact { dimension: Dimension::Length, nearest } => write!(
                f,
                "this value can't be stored exactly; the nearest storable value is {}",
                Length(*nearest)
            ),
            UnitError::Inexact { dimension: Dimension::Angle, nearest } => write!(
                f,
                "this angle can't be stored exactly; the nearest storable value is {}deg",
                Rational::new(*nearest as i128, MICRODEG_PER_DEG as i128).unwrap_or(Rational::ZERO)
            ),
            UnitError::Overflow => write!(f, "value is too large"),
        }
    }
}

impl std::error::Error for UnitError {}

/// Convert a quantity literal to an exact [`Length`].
pub fn quantity_to_length(q: Quantity) -> Result<Length, UnitError> {
    let per_unit: i64 = match q.unit {
        Unit::Millimeter => NM_PER_MM,
        Unit::Centimeter => 10 * NM_PER_MM,
        Unit::Meter => 1_000 * NM_PER_MM,
        Unit::Inch => NM_PER_INCH,
        Unit::Foot => 12 * NM_PER_INCH,
        Unit::Degree => {
            return Err(UnitError::WrongDimension { expected: Dimension::Length, found: q.unit });
        }
    };
    scale_exact(magnitude_to_rational(q.magnitude)?, per_unit, Dimension::Length).map(Length)
}

/// Convert a quantity literal to an exact [`Angle`].
pub fn quantity_to_angle(q: Quantity) -> Result<Angle, UnitError> {
    match q.unit {
        Unit::Degree => scale_exact(magnitude_to_rational(q.magnitude)?, MICRODEG_PER_DEG, Dimension::Angle).map(Angle),
        other => Err(UnitError::WrongDimension { expected: Dimension::Angle, found: other }),
    }
}

/// Convert a unitless literal to an exact [`Rational`].
pub fn number_to_rational(n: Number) -> Result<Rational, UnitError> {
    match n {
        Number::Integer(i) => Ok(Rational::from_int(i as i128)),
        Number::Decimal(d) => decimal(d.digits, d.scale),
    }
}

fn decimal(digits: u64, scale: u8) -> Result<Rational, UnitError> {
    let den = 10i128.checked_pow(scale as u32).ok_or(UnitError::Overflow)?;
    Rational::new(digits as i128, den).ok_or(UnitError::Overflow)
}

fn magnitude_to_rational(m: Magnitude) -> Result<Rational, UnitError> {
    match m {
        Magnitude::Integer(n) => Ok(Rational::from_int(n as i128)),
        Magnitude::Decimal(d) => decimal(d.digits, d.scale),
        Magnitude::Fraction(f) => {
            let den = f.denominator.get() as i128;
            let num = (f.whole as i128)
                .checked_mul(den)
                .and_then(|w| w.checked_add(f.numerator as i128))
                .ok_or(UnitError::Overflow)?;
            Rational::new(num, den).ok_or(UnitError::Overflow)
        }
    }
}

/// `value * per_unit`, requiring an exact integer result.
fn scale_exact(value: Rational, per_unit: i64, dimension: Dimension) -> Result<i64, UnitError> {
    let scaled = value.checked_mul(Rational::from_int(per_unit as i128)).ok_or(UnitError::Overflow)?;
    let to_i64 = |v: i128| i64::try_from(v).map_err(|_| UnitError::Overflow);
    if scaled.is_integer() {
        to_i64(scaled.numerator())
    } else {
        Err(UnitError::Inexact { dimension, nearest: to_i64(scaled.round())? })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Decimal, Fraction};
    use std::num::NonZeroU64;

    fn frac(whole: u64, n: u64, d: u64, unit: Unit) -> Quantity {
        Quantity {
            magnitude: Magnitude::Fraction(Fraction {
                whole,
                numerator: n,
                denominator: NonZeroU64::new(d).unwrap(),
            }),
            unit,
        }
    }

    fn dec(digits: u64, scale: u8, unit: Unit) -> Quantity {
        Quantity { magnitude: Magnitude::Decimal(Decimal { digits, scale }), unit }
    }

    fn r(n: i128, d: i128) -> Rational {
        Rational::new(n, d).unwrap()
    }

    #[test]
    fn inch_fractions_are_exact() {
        assert_eq!(quantity_to_length(frac(0, 3, 4, Unit::Inch)), Ok(Length::from_nm(19_050_000)));
        assert_eq!(quantity_to_length(frac(1, 1, 2, Unit::Inch)), Ok(Length::from_nm(38_100_000)));
        assert_eq!(quantity_to_length(frac(0, 1, 64, Unit::Inch)), Ok(Length::from_nm(396_875)));
        assert_eq!(quantity_to_length(frac(0, 1, 5, Unit::Inch)), Ok(Length::from_nm(5_080_000)));
        assert_eq!(Length::inches(3, 4), Length::from_nm(19_050_000));
    }

    #[test]
    fn inexact_literals_report_the_nearest_value() {
        let err = quantity_to_length(frac(0, 1, 128, Unit::Inch)).unwrap_err();
        assert_eq!(err, UnitError::Inexact { dimension: Dimension::Length, nearest: 198_438 });
        assert_eq!(
            err.to_string(),
            "this value can't be stored exactly; the nearest storable value is 0.198438mm"
        );
        assert!(matches!(quantity_to_length(frac(0, 1, 3, Unit::Inch)), Err(UnitError::Inexact { .. })));
    }

    #[test]
    fn metric_decimals() {
        assert_eq!(quantity_to_length(dec(185, 1, Unit::Millimeter)), Ok(Length::from_nm(18_500_000)));
        assert_eq!(
            quantity_to_length(dec(1_234_567, 7, Unit::Millimeter)),
            Err(UnitError::Inexact { dimension: Dimension::Length, nearest: 123_457 })
        );
    }

    #[test]
    fn mixed_units_agree() {
        let inch = quantity_to_length(Quantity { magnitude: Magnitude::Integer(1), unit: Unit::Inch });
        let mm = quantity_to_length(dec(254, 1, Unit::Millimeter));
        assert_eq!(inch, mm);
    }

    #[test]
    fn dimension_mismatch() {
        let deg = Quantity { magnitude: Magnitude::Integer(90), unit: Unit::Degree };
        assert!(matches!(quantity_to_length(deg), Err(UnitError::WrongDimension { .. })));
        assert_eq!(quantity_to_angle(deg), Ok(Angle::from_deg(90)));
    }

    #[test]
    fn display_trims_zeros() {
        assert_eq!(Length::inches(3, 4).to_string(), "19.05mm");
        assert_eq!(Length::from_mm(18).to_string(), "18mm");
        assert_eq!(Length::inches(1, 64).checked_neg().unwrap().to_string(), "-0.396875mm");
    }

    #[test]
    fn rationals_stay_exact() {
        assert_eq!(r(2, 4), r(1, 2));
        assert_eq!(r(1, -2), r(-1, 2));
        assert_eq!(r(1, 3).checked_add(r(1, 6)), Some(r(1, 2)));
        assert_eq!(r(1, 2).checked_div(Rational::ZERO), None);
        assert!(r(1, 3) < r(1, 2));
        assert_eq!(r(5, 2).round(), 3);
        assert_eq!(r(-5, 2).round(), -3);
        assert_eq!(number_to_rational(Number::Decimal(Decimal { digits: 75, scale: 2 })), Ok(r(3, 4)));
    }

    #[test]
    fn one_inch_divided_by_three_rounds_once() {
        let third = Length::inches(1, 1).to_rational().checked_div(Rational::from_int(3)).unwrap();
        assert!(!third.is_integer());
        let rounded = round_to(third, Length::from_nm(500_000)).unwrap();
        assert_eq!(rounded.length, Length::from_nm(8_500_000));
        assert_eq!(rounded.moved_by, r(100_000, 3));
    }

    #[test]
    fn checked_arithmetic_reports_overflow() {
        assert_eq!(Length::from_nm(i64::MAX).checked_add(Length::from_nm(1)), None);
        assert_eq!(Length::from_nm(i64::MIN).checked_neg(), None);
    }
}
