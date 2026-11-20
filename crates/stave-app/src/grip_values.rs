//! The values a grip may take (`docs/editing.md`, "Grips in the session"):
//! where a drag snaps, how far it may go, what an arrow key steps by, and
//! whether a typed value would be written as typed. The window turns the
//! pointer's position into a wanted value and these into words; the
//! arithmetic, exact, is here.
//!
//! ```text
//! pointer ──scrubbed──▶ wanted ──drag_target──▶ (target, held)   ──DragTo(target)──▶ session
//! arrow key ──stepped──▶ wanted ──drag_target──▶ ...
//! typed ──typed_check──▶ Ok, or why the session would write something else
//! ```

use stave_lang::units::Rational;
use stave_model::trace::Amount;

use crate::gripping::{GripStops, ListedGrip};
use crate::grips::GripAction;
use crate::stops::SearchSteps;

/// Which end of its range a value was held at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Held {
    /// The low end.
    Below,
    /// The high end.
    Above,
}

/// The value a grip has now.
pub fn grip_value(listed: &ListedGrip) -> Amount {
    match &listed.grip.action {
        GripAction::Moves(movable) => movable.value,
        GripAction::Follows(follows) => follows.value,
    }
}

/// How far a movable grip may go, low and high: its two last good values
/// once its stops are found; before, its `within` (or how far it may be
/// searched). `None` for a size that follows params: the session finds
/// its limits as it is dragged.
pub fn grip_bounds(listed: &ListedGrip) -> Option<(Amount, Amount)> {
    let GripAction::Moves(movable) = &listed.grip.action else {
        return None;
    };
    Some(match &listed.stops {
        GripStops::Found { below, above } => (below.last_good, above.last_good),
        GripStops::NotSearched | GripStops::NotSearchable | GripStops::Failed(_) => {
            (movable.low, movable.high)
        }
    })
}

/// `value` kept between `low` and `high`; the end it was held at, when it
/// was.
pub fn clamped(value: Amount, low: Amount, high: Amount) -> (Amount, Option<Held>) {
    if value.raw() < low.raw() {
        (low, Some(Held::Below))
    } else if value.raw() > high.raw() {
        (high, Some(Held::Above))
    } else {
        (value, None)
    }
}

/// The round numbers a drag snaps to while the modifier is held: 10mm,
/// 5°, whole numbers, twentieths.
fn round_step(like: Amount) -> Rational {
    match like {
        Amount::Length(_) => Rational::from_int(10_000_000),
        Amount::Angle(_) => Rational::from_int(5_000_000),
        Amount::Count(_) => Rational::ONE,
        Amount::Ratio(_) => Rational::new(1, 20).unwrap_or(Rational::ONE),
    }
}

/// Where a value the pointer reached snaps: to the grip's steps (the
/// project's resolution counted from the value now, or the nearest value
/// of a `within` list), or to round numbers when `round`.
pub fn snapped(now: Amount, wanted: Amount, steps: Option<&SearchSteps>, round: bool) -> Amount {
    let multiple = |from: Rational, step: Rational| -> Option<Rational> {
        let count = wanted.raw().checked_sub(from)?.checked_div(step)?.round();
        from.checked_add(step.checked_mul(Rational::from_int(count))?)
    };
    if round {
        let step = round_step(now);
        return multiple(Rational::ZERO, step).map_or(wanted, |raw| wanted.with_raw(raw));
    }
    match steps {
        Some(SearchSteps::Step(step)) => {
            multiple(now.raw(), step.raw()).map_or(wanted, |raw| wanted.with_raw(raw))
        }
        Some(SearchSteps::List(values)) => values
            .iter()
            .copied()
            .min_by_key(|value| value.raw().checked_sub(wanted.raw()).map(abs))
            .unwrap_or(wanted),
        None => wanted,
    }
}

fn abs(value: Rational) -> Rational {
    if value.is_negative() {
        value.checked_neg().unwrap_or(value)
    } else {
        value
    }
}

/// Where a drag toward `wanted` goes: a movable grip's value snapped to
/// its steps (or to round numbers) and held inside its range, with the
/// end it was held at; a size that follows params, snapped only when
/// `round`.
pub fn drag_target(listed: &ListedGrip, wanted: Amount, round: bool) -> (Amount, Option<Held>) {
    let now = grip_value(listed);
    match &listed.grip.action {
        GripAction::Moves(movable) => {
            let snapped = snapped(now, wanted, Some(&movable.steps), round);
            match grip_bounds(listed) {
                Some((low, high)) => clamped(snapped, low, high),
                None => (snapped, None),
            }
        }
        GripAction::Follows(_) => (snapped(now, wanted, None, round), None),
    }
}

/// The value `steps` presses of an arrow key move a grip to from `now`:
/// a step of its resolution each (a millimetre through a `within` list,
/// half a millimetre for a size that follows params), or 10mm each when
/// `round`. `None` when it can't be worked out.
pub fn stepped(listed: &ListedGrip, now: Amount, steps: i32, round: bool) -> Option<Amount> {
    let step = match (&listed.grip.action, round) {
        (GripAction::Moves(movable), false) => match &movable.steps {
            SearchSteps::Step(step) => step.raw(),
            SearchSteps::List(_) => Rational::from_int(1_000_000),
        },
        (GripAction::Moves(_) | GripAction::Follows(_), true) => Rational::from_int(10_000_000),
        (GripAction::Follows(_), false) => Rational::from_int(500_000),
    };
    let raw = step
        .checked_mul(Rational::from_int(i128::from(steps)))
        .and_then(|by| now.raw().checked_add(by))?;
    Some(now.with_raw(raw))
}

/// How far one pixel of a scrub moves a value of `like`'s kind: a
/// millimetre, half a degree, an eighth of a count (eight pixels a
/// piece) or a two-hundredth; a fifth of that when `fine`.
fn scrub_rate(like: Amount, fine: bool) -> Option<Rational> {
    let rate = match like {
        Amount::Length(_) => Rational::from_int(1_000_000),
        Amount::Angle(_) => Rational::from_int(500_000),
        Amount::Count(_) => Rational::new(1, 8)?,
        Amount::Ratio(_) => Rational::new(1, 200)?,
    };
    if fine {
        rate.checked_div(Rational::from_int(5))
    } else {
        Some(rate)
    }
}

/// The value a number field dragged sideways by `pixels` (right is
/// larger) from `from` asks for, before [`drag_target`] snaps it and
/// holds it in range: a millimetre a pixel for a length, half a degree
/// for an angle, a piece every eight pixels for a count, a fifth as fast
/// when `fine`. `None` when it can't be worked out.
pub fn scrubbed(from: Amount, pixels: i32, fine: bool) -> Option<Amount> {
    let by = scrub_rate(from, fine)?.checked_mul(Rational::from_int(i128::from(pixels)))?;
    Some(from.with_raw(from.raw().checked_add(by)?))
}

/// Why a value typed for a movable grip would not be written as typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypedProblem {
    /// It is past an end of the grip's range: the session would hold it
    /// there.
    OutOfRange {
        /// The end it is past.
        held: Held,
        /// The range.
        low: Amount,
        /// The range.
        high: Amount,
    },
    /// It is off the project's resolution: the session would write the
    /// nearest step.
    OffStep {
        /// The step.
        step: Amount,
        /// The nearest value on a step.
        nearest: Amount,
    },
    /// It is not one of the values of the param's `within` list.
    NotListed(Vec<Amount>),
    /// The nearest step can't be worked out.
    Unworkable,
}

/// A value typed for a grip, checked before it is tried: the session
/// would write the nearest step of the project's resolution, or the end
/// of the grip's range, in place of a value off a step or past an end, so
/// such a value is refused, and nothing different from what was typed is
/// ever written. A size that follows params is checked by the session
/// alone (`Ok`).
///
/// # Errors
///
/// What the session would change, as a [`TypedProblem`].
pub fn typed_check(listed: &ListedGrip, value: Amount) -> Result<(), TypedProblem> {
    let GripAction::Moves(movable) = &listed.grip.action else {
        return Ok(());
    };
    if let Some((low, high)) = grip_bounds(listed)
        && let (_, Some(held)) = clamped(value, low, high)
    {
        return Err(TypedProblem::OutOfRange { held, low, high });
    }
    match &movable.steps {
        // As the session snaps: to a whole number of steps.
        SearchSteps::Step(step) => {
            let nearest = value
                .raw()
                .checked_div(step.raw())
                .map(|times| times.round())
                .and_then(|times| step.raw().checked_mul(Rational::from_int(times)));
            match nearest {
                Some(nearest) if nearest == value.raw() => Ok(()),
                Some(nearest) => Err(TypedProblem::OffStep {
                    step: *step,
                    nearest: value.with_raw(nearest),
                }),
                None => Err(TypedProblem::Unworkable),
            }
        }
        SearchSteps::List(values) => {
            if values.iter().any(|listed| listed.raw() == value.raw()) {
                Ok(())
            } else {
                Err(TypedProblem::NotListed(values.clone()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use stave_lang::units::Length;
    use stave_model::ids::ParamId;
    use stave_model::trace::Freedom;

    use super::*;
    use crate::grips::{GripSize, MovableGrip, SelectionGrip};
    use crate::stops::{Side, SideEnd};

    fn mm(n: i64) -> Amount {
        Amount::Length(Length::from_mm(n).to_exact())
    }

    fn tenths(n: i64) -> Amount {
        Amount::Length(Length::from_nm(n * 100_000).to_exact())
    }

    fn movable(value: i64, stops: GripStops) -> ListedGrip {
        ListedGrip {
            grip: SelectionGrip {
                size: GripSize::Param {
                    param: ParamId::from_index(0).unwrap(),
                    path: Some("overall_width".to_owned()),
                },
                label: "overall_width".to_owned(),
                axis: None,
                action: GripAction::Moves(Box::new(MovableGrip {
                    value: mm(value),
                    low: mm(600),
                    high: mm(1200),
                    steps: SearchSteps::Step(tenths(5)),
                    freedom: Freedom::Plain,
                    stops: None,
                })),
                place: None,
            },
            stops,
        }
    }

    #[test]
    fn a_drag_snaps_to_the_resolution_or_to_round_numbers() {
        let grip = movable(900, GripStops::NotSearched);
        assert_eq!(
            drag_target(&grip, tenths(9123), false),
            (tenths(9125), None)
        );
        assert_eq!(drag_target(&grip, tenths(9123), true), (mm(910), None));
    }

    #[test]
    fn a_drag_past_an_end_is_held_there() {
        let grip = movable(
            5,
            GripStops::Found {
                below: Box::new(Side {
                    last_good: mm(0),
                    end: SideEnd::Domain,
                }),
                above: Box::new(Side {
                    last_good: mm(7),
                    end: SideEnd::Limit,
                }),
            },
        );
        assert_eq!(grip_bounds(&grip), Some((mm(0), mm(7))));
        assert_eq!(drag_target(&grip, mm(9), false), (mm(7), Some(Held::Above)));
        assert_eq!(drag_target(&grip, mm(6), false), (mm(6), None));
        // Before the stops are found, the range is the within.
        let grip = movable(900, GripStops::NotSearched);
        assert_eq!(
            drag_target(&grip, mm(100), false),
            (mm(600), Some(Held::Below))
        );
    }

    #[test]
    fn an_arrow_key_steps_by_the_resolution() {
        let grip = movable(900, GripStops::NotSearched);
        assert_eq!(stepped(&grip, mm(900), 2, false), Some(mm(901)));
        assert_eq!(stepped(&grip, mm(900), -1, true), Some(mm(890)));
    }

    #[test]
    fn a_scrub_moves_a_millimetre_a_pixel_then_snaps() {
        let grip = movable(900, GripStops::NotSearched);
        assert_eq!(scrubbed(mm(900), 37, false), Some(mm(937)));
        assert_eq!(scrubbed(mm(900), -3, true), Some(tenths(8994)));
        let wanted = scrubbed(mm(900), 3, true).unwrap();
        assert_eq!(drag_target(&grip, wanted, false), (tenths(9005), None));
        assert_eq!(
            scrubbed(Amount::Count(Rational::from_int(3)), 16, false),
            Some(Amount::Count(Rational::from_int(5)))
        );
    }

    #[test]
    fn a_typed_value_the_session_would_change_is_refused() {
        let grip = movable(900, GripStops::NotSearched);
        assert_eq!(
            typed_check(&grip, mm(9000)),
            Err(TypedProblem::OutOfRange {
                held: Held::Above,
                low: mm(600),
                high: mm(1200)
            })
        );
        assert_eq!(
            typed_check(&grip, tenths(9123)),
            Err(TypedProblem::OffStep {
                step: tenths(5),
                nearest: tenths(9125)
            })
        );
        assert_eq!(typed_check(&grip, tenths(9125)), Ok(()));
        assert_eq!(typed_check(&grip, mm(1200)), Ok(()));
    }
}
