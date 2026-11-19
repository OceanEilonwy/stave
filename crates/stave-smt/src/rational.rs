//! Exact rational numbers: the only kind of number the API takes or gives.

use std::cmp::Ordering;
use std::fmt;

use crate::SmtError;

/// An exact fraction `numerator / denominator`, always in lowest terms with a
/// positive denominator.
///
/// Both parts are `i128`, wide enough for any length in nanometres and for
/// the products of a coefficient and such a length. Two values that are equal
/// as fractions are equal as Rust values (`2/4 == 1/2`).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rational {
    numerator: i128,
    denominator: i128,
}

impl Rational {
    /// The fraction zero.
    pub const ZERO: Rational = Rational {
        numerator: 0,
        denominator: 1,
    };

    /// The fraction one.
    pub const ONE: Rational = Rational {
        numerator: 1,
        denominator: 1,
    };

    /// Makes `numerator / denominator`, reduced to lowest terms.
    ///
    /// # Errors
    ///
    /// [`SmtError::ZeroDenominator`] when `denominator` is zero, and
    /// [`SmtError::Overflow`] when the sign cannot be moved to the numerator
    /// (a part equal to `i128::MIN`).
    pub fn new(numerator: i128, denominator: i128) -> Result<Rational, SmtError> {
        if denominator == 0 {
            return Err(SmtError::ZeroDenominator);
        }
        let divisor = greatest_common_divisor(numerator, denominator);
        // `divisor` is at least 1 because the denominator is not zero.
        let mut numerator = numerator.checked_div(divisor).ok_or(SmtError::Overflow)?;
        let mut denominator = denominator.checked_div(divisor).ok_or(SmtError::Overflow)?;
        if denominator < 0 {
            numerator = numerator.checked_neg().ok_or(SmtError::Overflow)?;
            denominator = denominator.checked_neg().ok_or(SmtError::Overflow)?;
        }
        Ok(Rational {
            numerator,
            denominator,
        })
    }

    /// Makes the whole number `value`.
    #[must_use]
    pub const fn from_integer(value: i128) -> Rational {
        Rational {
            numerator: value,
            denominator: 1,
        }
    }

    /// The numerator, carrying the sign.
    #[must_use]
    pub const fn numerator(self) -> i128 {
        self.numerator
    }

    /// The denominator, always positive.
    #[must_use]
    pub const fn denominator(self) -> i128 {
        self.denominator
    }

    /// Whether this is zero.
    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.numerator == 0
    }

    /// `self - other`, or [`SmtError::Overflow`] when a part leaves `i128`.
    ///
    /// # Errors
    ///
    /// [`SmtError::Overflow`] when the exact result does not fit.
    pub fn checked_sub(self, other: Rational) -> Result<Rational, SmtError> {
        let left = self
            .numerator
            .checked_mul(other.denominator)
            .ok_or(SmtError::Overflow)?;
        let right = other
            .numerator
            .checked_mul(self.denominator)
            .ok_or(SmtError::Overflow)?;
        let numerator = left.checked_sub(right).ok_or(SmtError::Overflow)?;
        let denominator = self
            .denominator
            .checked_mul(other.denominator)
            .ok_or(SmtError::Overflow)?;
        Rational::new(numerator, denominator)
    }

    /// `1 / self`, or `None` for zero.
    #[must_use]
    pub fn reciprocal(self) -> Option<Rational> {
        Rational::new(self.denominator, self.numerator).ok()
    }
}

impl Ord for Rational {
    fn cmp(&self, other: &Rational) -> Ordering {
        compare_fractions(
            self.numerator,
            self.denominator,
            other.numerator,
            other.denominator,
        )
    }
}

impl PartialOrd for Rational {
    fn partial_cmp(&self, other: &Rational) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl From<i64> for Rational {
    fn from(value: i64) -> Rational {
        Rational::from_integer(i128::from(value))
    }
}

impl From<i32> for Rational {
    fn from(value: i32) -> Rational {
        Rational::from_integer(i128::from(value))
    }
}

impl fmt::Display for Rational {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.denominator == 1 {
            write!(formatter, "{}", self.numerator)
        } else {
            write!(formatter, "{}/{}", self.numerator, self.denominator)
        }
    }
}

impl fmt::Debug for Rational {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

/// The greatest common divisor of the magnitudes, computed in `u128` so that
/// `i128::MIN` does not overflow; at least 1 when `b` is not zero. A divisor
/// that does not fit back in `i128` (only possible for `i128::MIN` with
/// itself) is reported as 1, leaving the fraction unreduced but correct.
fn greatest_common_divisor(a: i128, b: i128) -> i128 {
    let mut x = a.unsigned_abs();
    let mut y = b.unsigned_abs();
    while y != 0 {
        let remainder = x % y;
        x = y;
        y = remainder;
    }
    i128::try_from(x).unwrap_or(1).max(1)
}

/// Compares `a/b` with `c/d` (`b`, `d` positive) without multiplying, by
/// comparing whole parts and then the reciprocals of the remainders (the
/// continued-fraction expansion), so no input can overflow.
fn compare_fractions(a: i128, b: i128, c: i128, d: i128) -> Ordering {
    let (mut a, mut b, mut c, mut d) = (a, b, c, d);
    let mut flipped = false;
    loop {
        // `b` and `d` stay positive, so neither division can fail or overflow.
        let whole_left = a.div_euclid(b);
        let whole_right = c.div_euclid(d);
        let remainder_left = a.rem_euclid(b);
        let remainder_right = c.rem_euclid(d);
        let ordering = match whole_left.cmp(&whole_right) {
            Ordering::Equal => match (remainder_left == 0, remainder_right == 0) {
                (true, true) => Ordering::Equal,
                (true, false) => Ordering::Less,
                (false, true) => Ordering::Greater,
                (false, false) => {
                    // remainder_left / b against remainder_right / d, both in
                    // (0, 1), orders the opposite way to b / remainder_left
                    // against d / remainder_right.
                    (a, b, c, d) = (b, remainder_left, d, remainder_right);
                    flipped = !flipped;
                    continue;
                }
            },
            unequal @ (Ordering::Less | Ordering::Greater) => unequal,
        };
        return if flipped {
            ordering.reverse()
        } else {
            ordering
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fraction(numerator: i128, denominator: i128) -> Rational {
        match Rational::new(numerator, denominator) {
            Ok(value) => value,
            Err(error) => panic!("{error}"),
        }
    }

    #[test]
    fn reduces_and_moves_the_sign_up() {
        assert_eq!(fraction(2, -4), fraction(-1, 2));
        assert_eq!(fraction(-2, -4).denominator(), 2);
        assert_eq!(fraction(0, -7), Rational::ZERO);
        assert!(Rational::new(1, 0).is_err());
    }

    #[test]
    fn orders_without_overflow() {
        assert!(fraction(1, 3) < fraction(1, 2));
        assert!(fraction(-1, 2) < fraction(-1, 3));
        assert!(fraction(7, 3) > fraction(9, 4));
        let huge = fraction(i128::MAX, i128::MAX - 1);
        let other = fraction(i128::MAX - 1, i128::MAX - 2);
        assert!(huge < other);
        assert_eq!(fraction(6, 4).cmp(&fraction(3, 2)), Ordering::Equal);
    }

    #[test]
    fn subtracts_and_inverts() {
        assert_eq!(
            fraction(1, 2).checked_sub(fraction(1, 3)),
            Ok(fraction(1, 6))
        );
        assert_eq!(fraction(-2, 3).reciprocal(), Some(fraction(-3, 2)));
        assert_eq!(Rational::ZERO.reciprocal(), None);
    }
}
