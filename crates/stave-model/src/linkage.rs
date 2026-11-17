//! Four-bar linkages: how a cup hinge moves its door.
//!
//! A concealed cup hinge is not a pin. Its **arm** is screwed (through the
//! mounting plate) to the cabinet side; its **boss** is pressed into the cup
//! in the door; two **links** join them, each turning on one pin in the arm
//! and one in the boss. As the door opens, the boss (and the door with it)
//! turns and moves at once: forward, away from the cabinet, and sideways,
//! so the door clears the side it overlays.
//!
//! A [`Linkage`] is that mechanism in its own plane, across the hinge axis:
//! the four pins at rest (door closed), exact, in a [`LinkageFrame`]. Its
//! one input is the door's **opening angle**: how far the boss has turned
//! from rest. Turning the boss by an angle fixes where it must be for both
//! links to keep their lengths (two circles meet in two points; the linkage
//! stays on the one it starts on), so every angle has one pose, and the
//! pose at angle zero is rest, exactly.
//!
//! [`Linkage::new`] makes a linkage that can't jam unrepresentable: both
//! links have length, and over the whole range from closed to
//! `opening_angle` the two circles meet at a clear angle, never touching
//! (a link in line with the other's reach is a dead point, where the
//! mechanism locks or flips to its other branch). That is checked exactly,
//! in integers: the distance between the circles' centres moves on a
//! sinusoid of the angle, whose least and greatest values over the range
//! are found in closed form. The cosine and sine of the opening angle are
//! integer fixed-point values with a known, tiny error, which the margin
//! (0.1mm of reach) covers many times over.
//!
//! Solving the pose at an angle needs square roots and trigonometry: that
//! is done at the geometry boundary (`stave-geom`), from these exact pins.

use serde::Serialize;
use stave_lang::placement::{Axis, Direction};
use stave_lang::units::{Angle, Length};

use crate::solved::Point;

/// How far from a dead point the linkage must stay over its whole range:
/// the two circles' centres stay at least this much inside the reach of
/// the links together and outside the difference of their lengths
/// (0.1mm).
pub const DEAD_POINT_MARGIN: Length = Length::from_nm(100_000);

/// A point in a linkage's plane, relative to its frame's origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize)]
pub struct PlanePoint {
    /// Toward the frame's free side (from the hinge edge toward the middle
    /// of the door).
    pub x: Length,
    /// Toward the side the door swings to.
    pub y: Length,
}

impl PlanePoint {
    /// A point from its two coordinates.
    pub const fn new(x: Length, y: Length) -> PlanePoint {
        PlanePoint { x, y }
    }
}

/// The plane a linkage moves in: across a world axis, with `x` toward the
/// free side and `y` toward the side it swings to. Turning by a positive
/// angle carries `x` toward `y`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct LinkageFrame {
    #[serde(serialize_with = "serialize_axis")]
    axis: Axis,
    origin: Point,
    #[serde(serialize_with = "serialize_direction")]
    free_side: Direction,
    #[serde(serialize_with = "serialize_direction")]
    swing_toward: Direction,
}

impl LinkageFrame {
    /// A frame; `free_side` and `swing_toward` must be across `axis` and
    /// across each other.
    pub fn new(
        axis: Axis,
        origin: Point,
        free_side: Direction,
        swing_toward: Direction,
    ) -> Result<LinkageFrame, LinkageError> {
        let (a, b) = (free_side.axis(), swing_toward.axis());
        if a == axis || b == axis || a == b {
            return Err(LinkageError::Frame);
        }
        Ok(LinkageFrame {
            axis,
            origin,
            free_side,
            swing_toward,
        })
    }

    /// The world axis the pins run along.
    pub fn axis(&self) -> Axis {
        self.axis
    }

    /// The world point of the frame's origin. Its coordinate along
    /// [`LinkageFrame::axis`] is where the mechanism's middle is.
    pub fn origin(&self) -> Point {
        self.origin
    }

    /// The world direction of `x`.
    pub fn free_side(&self) -> Direction {
        self.free_side
    }

    /// The world direction of `y`.
    pub fn swing_toward(&self) -> Direction {
        self.swing_toward
    }

    /// The world point of a point in the plane, at the origin's place
    /// along the axis. Exact (saturating at the ends of [`Length`]).
    pub fn world(&self, point: PlanePoint) -> Point {
        let step = |base: Point, direction: Direction, amount: Length| {
            let axis = direction.axis();
            let now = base.get(axis).nm();
            let value = if direction.is_positive() {
                now.saturating_add(amount.nm())
            } else {
                now.saturating_sub(amount.nm())
            };
            base.with(axis, Length::from_nm(value))
        };
        let moved = step(self.origin, self.free_side, point.x);
        step(moved, self.swing_toward, point.y)
    }

    /// The frame reflected through a plane across `axis`, its origin
    /// moved along `axis` to `origin` (where the reflection puts it). A
    /// mirrored linkage has the same points in its own frame.
    pub fn reflected(&self, axis: Axis, origin: Length) -> LinkageFrame {
        let flip = |d: Direction| if d.axis() == axis { d.opposite() } else { d };
        LinkageFrame {
            axis: self.axis,
            origin: self.origin.with(axis, origin),
            free_side: flip(self.free_side),
            swing_toward: flip(self.swing_toward),
        }
    }
}

/// One link at rest: its pin in the fixed member (the hinge arm) and its
/// pin in the moving member (the boss in the cup).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct Link {
    /// The pin in the arm, which stays put.
    pub ground: PlanePoint,
    /// The pin in the boss, which moves with the door.
    pub coupler: PlanePoint,
}

/// Which of the two places a turned boss could be is the linkage's: the
/// side of the line between the two circles' centres it lies on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Branch {
    /// To the left of the line from the first circle's centre to the
    /// second's (counter-clockwise).
    Left,
    /// To the right.
    Right,
}

/// Why a linkage is not one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum LinkageError {
    /// The frame's directions aren't across its axis and each other.
    #[error("a linkage's plane must be across its axis")]
    Frame,
    /// A link's two pins are in one place.
    #[error("link {0} has no length")]
    NoLength(usize),
    /// The two links share a pin.
    #[error("the two links share a pin")]
    SharedPin,
    /// The opening angle is not in `(0, 180deg]`.
    #[error("a linkage opens by more than 0deg and at most 180deg")]
    Angle,
    /// Somewhere between closed and the opening angle the links reach a
    /// dead point (or don't reach at all).
    #[error("the links reach a dead point before the door opens {0}")]
    DeadPoint(Angle),
    /// A number too big for the arithmetic (never a design's).
    #[error("the linkage is too large to check")]
    Overflow,
}

/// A four-bar linkage that moves without jamming from rest to its opening
/// angle. See the module docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct Linkage {
    frame: LinkageFrame,
    links: [Link; 2],
    opening_angle: Angle,
    branch: Branch,
}

impl Linkage {
    /// Checks a linkage: both links have length, they share no pin, the
    /// angle is in `(0, 180deg]`, and the links stay at least
    /// [`DEAD_POINT_MARGIN`] from a dead point from rest to
    /// `opening_angle`.
    pub fn new(
        frame: LinkageFrame,
        links: [Link; 2],
        opening_angle: Angle,
    ) -> Result<Linkage, LinkageError> {
        if opening_angle.microdeg() <= 0 || opening_angle > Angle::from_deg(180) {
            return Err(LinkageError::Angle);
        }
        let [first, second] = links;
        for (index, link) in links.iter().enumerate() {
            if link.ground == link.coupler {
                return Err(LinkageError::NoLength(index));
            }
        }
        if first.ground == second.ground || first.coupler == second.coupler {
            return Err(LinkageError::SharedPin);
        }
        let reach = Reach::of(&links).ok_or(LinkageError::Overflow)?;
        if !reach
            .clear_over(opening_angle)
            .ok_or(LinkageError::Overflow)?
        {
            return Err(LinkageError::DeadPoint(opening_angle));
        }
        // The boss is at rest at angle zero: which side of the centre line
        // that is decides the branch for every angle.
        let side = reach.rest_side().ok_or(LinkageError::Overflow)?;
        let branch = if side > 0 {
            Branch::Left
        } else if side < 0 {
            Branch::Right
        } else {
            return Err(LinkageError::DeadPoint(Angle::ZERO));
        };
        Ok(Linkage {
            frame,
            links,
            opening_angle,
            branch,
        })
    }

    /// The plane it moves in.
    pub fn frame(&self) -> &LinkageFrame {
        &self.frame
    }

    /// The two links at rest.
    pub fn links(&self) -> &[Link; 2] {
        &self.links
    }

    /// How far the door opens.
    pub fn opening_angle(&self) -> Angle {
        self.opening_angle
    }

    /// Which of the two solutions the boss is on at every angle.
    pub fn branch(&self) -> Branch {
        self.branch
    }

    /// The same linkage in another frame (a mirrored door's): whether it
    /// jams depends only on its points in its own frame, which don't
    /// change.
    pub fn in_frame(&self, frame: LinkageFrame) -> Linkage {
        Linkage { frame, ..*self }
    }
}

/// The squared distance between the two circles' centres as the boss turns
/// by `θ`: `p + q·cos θ + s·sin θ` (square nanometres), against the
/// squared lengths of the links.
struct Reach {
    p: i128,
    q: i128,
    s: i128,
    first: i128,
    second: i128,
    /// The two circles' centres at rest (each pin in the arm less its pin in
    /// the boss), for the branch.
    centres: [(i128, i128); 2],
}

/// Fixed-point scale of the cosine and sine: 2^60.
const SCALE_BITS: u32 = 60;
const SCALE: i128 = 1 << SCALE_BITS;
/// π · 2^60, rounded to the nearest integer.
const PI_FIXED: i128 = 3_622_009_729_038_561_421;
/// The error allowed for in a fixed-point product, in square nanometres:
/// far more than the cosine and sine errors (a few parts in 2^58) times
/// any design's squared sizes, and far less than the margin's square.
const SLACK: i128 = 1_000_000;

impl Reach {
    fn of(links: &[Link; 2]) -> Option<Reach> {
        let vector = |from: PlanePoint, to: PlanePoint| -> Option<(i128, i128)> {
            Some((
                i128::from(to.x.nm()).checked_sub(i128::from(from.x.nm()))?,
                i128::from(to.y.nm()).checked_sub(i128::from(from.y.nm()))?,
            ))
        };
        let [a, b] = links;
        let square = |(x, y): (i128, i128)| x.checked_mul(x)?.checked_add(y.checked_mul(y)?);
        let first = square(vector(a.ground, a.coupler)?)?;
        let second = square(vector(b.ground, b.coupler)?)?;
        // u: between the arm pins; v: between the boss pins.
        let u = vector(b.ground, a.ground)?;
        let v = vector(b.coupler, a.coupler)?;
        let dot = u.0.checked_mul(v.0)?.checked_add(u.1.checked_mul(v.1)?)?;
        let cross = u.0.checked_mul(v.1)?.checked_sub(u.1.checked_mul(v.0)?)?;
        let centre = |link: &Link| vector(link.coupler, link.ground);
        Some(Reach {
            p: square(u)?.checked_add(square(v)?)?,
            q: dot.checked_mul(-2)?,
            s: cross.checked_mul(2)?,
            first,
            second,
            centres: [centre(a)?, centre(b)?],
        })
    }

    /// The sign of the side the boss's rest position (the origin of the
    /// translation, zero) lies on, seen along the line between the
    /// circles' centres.
    fn rest_side(&self) -> Option<i128> {
        let [(x1, y1), (x2, y2)] = self.centres;
        let (dx, dy) = (x2.checked_sub(x1)?, y2.checked_sub(y1)?);
        // cross(c2 - c1, 0 - c1)
        let side = dx
            .checked_mul(y1.checked_neg()?)?
            .checked_sub(dy.checked_mul(x1.checked_neg()?)?)?;
        Some(side.signum())
    }

    /// `true` when, for every angle from zero to `angle`, the centres'
    /// distance stays at least the margin inside `first + second` and
    /// outside `|first - second|` (the links' lengths).
    fn clear_over(&self, angle: Angle) -> Option<bool> {
        let (cosine, sine) = cos_sin(angle)?;
        // The squared distance at both ends.
        let at_rest = self.p.checked_add(self.q)?;
        let at_end = self
            .p
            .checked_add(fixed_mul(self.q, cosine)?.checked_add(fixed_mul(self.s, sine)?)?)?;
        let mut low = at_rest.min(at_end).checked_sub(SLACK)?;
        let mut high = at_rest.max(at_end).checked_add(SLACK)?;
        // Inside the range, the sinusoid peaks where (cos, sin) points
        // along (q, s) and dips where it points the other way.
        let amplitude = isqrt(
            self.q
                .checked_mul(self.q)?
                .checked_add(self.s.checked_mul(self.s)?)?,
        )?;
        let (q, s) = (self.q, self.s);
        if within_arc(q, s, cosine, sine)? {
            high = high.max(self.p.checked_add(amplitude)?.checked_add(1)?);
        }
        if within_arc(q.checked_neg()?, s.checked_neg()?, cosine, sine)? {
            low = low.min(self.p.checked_sub(amplitude)?.checked_sub(1)?);
        }
        // The links' lengths, each between its integer square roots.
        let (first_low, first_high) = sqrt_bounds(self.first)?;
        let (second_low, second_high) = sqrt_bounds(self.second)?;
        let margin = i128::from(DEAD_POINT_MARGIN.nm());
        // Outer limit: (L1 + L2 - margin)^2 at its smallest.
        let outer = first_low.checked_add(second_low)?.checked_sub(margin)?;
        // Inner limit: (|L1 - L2| + margin)^2 at its largest.
        let difference = first_high
            .checked_sub(second_low)?
            .max(second_high.checked_sub(first_low)?)
            .max(0);
        let inner = difference.checked_add(margin)?;
        if outer <= 0 {
            return Some(false);
        }
        Some(low > inner.checked_mul(inner)? && high < outer.checked_mul(outer)?)
    }
}

/// `value · fraction / 2^60`, rounded toward zero.
fn fixed_mul(value: i128, fraction: i128) -> Option<i128> {
    value.checked_mul(fraction).map(|product| product / SCALE)
}

/// Is the direction `(x, y)` within the counter-clockwise arc from `(1, 0)`
/// to `(cosine, sine)` (an arc of at most half a turn)? Directions on its
/// edges count as inside.
fn within_arc(x: i128, y: i128, cosine: i128, sine: i128) -> Option<bool> {
    if x == 0 && y == 0 {
        return Some(false);
    }
    // Scale the direction down so the cross product can't overflow.
    let biggest = x.unsigned_abs().max(y.unsigned_abs());
    let shift = (128 - biggest.leading_zeros()).saturating_sub(60);
    let (x, y) = (x >> shift, y >> shift);
    // From the start: y ≥ 0 (with sine ≥ 0, the arc is in the upper half;
    // a half turn's end is (−1, 0), which y ≥ 0 also covers).
    let after_start = y >= 0;
    let before_end = x.checked_mul(sine)?.checked_sub(y.checked_mul(cosine)?)? >= 0;
    Some(after_start && before_end)
}

/// The floor and ceiling of the square root of a non-negative integer.
fn sqrt_bounds(value: i128) -> Option<(i128, i128)> {
    let root = isqrt(value)?;
    let exact = root.checked_mul(root)? == value;
    Some((root, if exact { root } else { root.checked_add(1)? }))
}

/// The floor of the square root of a non-negative integer.
fn isqrt(value: i128) -> Option<i128> {
    if value < 0 {
        return None;
    }
    if value < 2 {
        return Some(value);
    }
    // Newton's method from above.
    let bits = 128 - value.leading_zeros();
    let mut x: i128 = 1i128.checked_shl(bits.div_ceil(2))?;
    loop {
        let next = (x + value / x) / 2;
        if next >= x {
            return Some(x);
        }
        x = next;
    }
}

/// The cosine and sine of an angle in `[0, 180deg]`, as fixed-point
/// integers scaled by 2^60, each within a few units of the true value.
fn cos_sin(angle: Angle) -> Option<(i128, i128)> {
    // x = angle in radians, fixed point: microdeg · π / 180e6.
    let microdeg = i128::from(angle.microdeg());
    let x = microdeg.checked_mul(PI_FIXED)? / 180_000_000;
    // Taylor series about zero; for |x| ≤ π the terms past x^40 are far
    // below one unit.
    let mut cosine = SCALE;
    let mut sine = x;
    let mut term = x; // x^n / n!, starting at n = 1
    for n in 2..=40i128 {
        term = fixed_mul(term, x)? / n;
        match n % 4 {
            0 => cosine = cosine.checked_add(term)?,
            1 => sine = sine.checked_add(term)?,
            2 => cosine = cosine.checked_sub(term)?,
            _ => sine = sine.checked_sub(term)?,
        }
    }
    Some((cosine, sine))
}

fn serialize_axis<S: serde::Serializer>(value: &Axis, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(value.word())
}

fn serialize_direction<S: serde::Serializer>(
    value: &Direction,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(value.word())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mm(value: f64) -> Length {
        Length::from_nm((value * 1e6).round() as i64)
    }

    fn point(x: f64, y: f64) -> PlanePoint {
        PlanePoint::new(mm(x), mm(y))
    }

    fn frame() -> LinkageFrame {
        LinkageFrame::new(
            Axis::Up,
            Point::default(),
            Direction::Right,
            Direction::Front,
        )
        .unwrap()
    }

    #[test]
    fn fixed_point_trig_is_accurate() {
        for degrees in [0, 1, 30, 45, 60, 89, 90, 110, 135, 179, 180] {
            let (c, s) = cos_sin(Angle::from_deg(degrees)).unwrap();
            let radians = f64::from(degrees as i32).to_radians();
            let scale = SCALE as f64;
            assert!(
                (c as f64 / scale - radians.cos()).abs() < 1e-15,
                "{degrees}"
            );
            assert!(
                (s as f64 / scale - radians.sin()).abs() < 1e-15,
                "{degrees}"
            );
        }
        assert_eq!(isqrt(0), Some(0));
        assert_eq!(isqrt(15), Some(3));
        assert_eq!(isqrt(16), Some(4));
        assert_eq!(isqrt(i128::from(u64::MAX)), Some(4_294_967_295));
    }

    #[test]
    fn a_linkage_is_checked_over_its_whole_range() {
        // Two links of about 10.4mm from arm pins 20mm apart to boss pins
        // 14mm apart: at rest the circles' centres are 6mm apart (fine);
        // turned half a turn they are 34mm apart, past the 20.9mm the
        // links reach together.
        let links = [
            Link {
                ground: point(0.0, 0.0),
                coupler: point(3.0, 10.0),
            },
            Link {
                ground: point(20.0, 0.0),
                coupler: point(17.0, 10.0),
            },
        ];
        let small = Linkage::new(frame(), links, Angle::from_deg(30)).unwrap();
        assert_eq!(small.opening_angle(), Angle::from_deg(30));
        assert!(matches!(
            Linkage::new(frame(), links, Angle::from_deg(180)),
            Err(LinkageError::DeadPoint(_))
        ));
        // Bad shapes.
        let mut zero = links;
        zero[1].coupler = zero[1].ground;
        assert_eq!(
            Linkage::new(frame(), zero, Angle::from_deg(30)),
            Err(LinkageError::NoLength(1))
        );
        assert_eq!(
            Linkage::new(frame(), links, Angle::ZERO),
            Err(LinkageError::Angle)
        );
        assert!(
            LinkageFrame::new(Axis::Up, Point::default(), Direction::Up, Direction::Front).is_err()
        );
    }

    #[test]
    fn the_largest_clear_angle_matches_the_sinusoid() {
        // Find, by the exact check, the largest whole degree the linkage
        // above opens to, and compare with floats.
        let links = [
            Link {
                ground: point(0.0, 0.0),
                coupler: point(3.0, 10.0),
            },
            Link {
                ground: point(20.0, 0.0),
                coupler: point(17.0, 10.0),
            },
        ];
        let largest = (1..=180)
            .take_while(|d| Linkage::new(frame(), links, Angle::from_deg(*d)).is_ok())
            .last()
            .unwrap();
        // Centres c_i = a_i − R b_i; |c1 − c2|² = |u|² + |v|² − 2 u·Rv with
        // u = (−20, 0), v = (−14, 0): 400 + 196 − 560 cos θ, which must stay
        // below (L1 + L2 − 0.1)², each link √109mm long.
        let reach = 2.0 * 109f64.sqrt() - 0.1;
        let limit = ((400.0 + 196.0 - reach * reach) / 560.0)
            .acos()
            .to_degrees();
        assert_eq!(largest, limit.floor() as i64);
    }

    #[test]
    fn mirroring_keeps_the_plane_points() {
        let links = [
            Link {
                ground: point(0.0, 0.0),
                coupler: point(3.0, 10.0),
            },
            Link {
                ground: point(20.0, 0.0),
                coupler: point(17.0, 10.0),
            },
        ];
        let linkage = Linkage::new(frame(), links, Angle::from_deg(30)).unwrap();
        let image = linkage.in_frame(linkage.frame().reflected(Axis::Across, mm(900.0)));
        assert_eq!(image.links(), linkage.links());
        assert_eq!(image.frame().free_side(), Direction::Left);
        assert_eq!(image.frame().origin().across, mm(900.0));
        assert_eq!(
            image.frame().world(point(10.0, 5.0)),
            Point {
                across: mm(890.0),
                up: Length::ZERO,
                deep: mm(-5.0),
            }
        );
    }
}
