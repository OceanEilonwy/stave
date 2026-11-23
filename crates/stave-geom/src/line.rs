//! A straight line along a world axis, and where it runs in a solid's
//! wood: what a clamp's stack is read from (grammar, "Clamps").
//!
//! Everything here is exact integer arithmetic on nanometres. The line's
//! point is kept doubled, so the middle of a box is exact; where a line
//! crosses a round or slanted outline, the ends are irrational and are
//! taken to the nanometre, far inside the 0.05mm contact tolerance. A
//! line seen by a body a pose has turned about a line along its axis
//! ([`Line::seen_by`]) is worked out in floats and taken to the
//! nanometre the same way.
//!
//! A solid's wood is its boxes, rods and prisms; its drilled holes and
//! angled cuts are not taken out (a hole is never a gap, and an angled
//! cut counts as wood), and a slanted cylinder counts as the box around
//! it.

use stave_lang::placement::Axis;
use stave_lang::units::Length;
use stave_model::solved::{Point, WorldBox};

use crate::collide::{Placement, WorldPrism, WorldRod, WorldSolid, point_vector};
use crate::render;

/// A line along a world axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Line {
    axis: Axis,
    /// Twice its coordinates on the two other axes, in [`Axis::others`]
    /// order.
    doubled: [i128; 2],
}

/// A stretch of a line, between two coordinates along its axis; `low` is
/// never above `high`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stretch {
    /// The low end.
    pub low: Length,
    /// The high end.
    pub high: Length,
}

impl Stretch {
    fn new(a: Length, b: Length) -> Stretch {
        Stretch {
            low: a.min(b),
            high: a.max(b),
        }
    }
}

impl Line {
    /// The line along `axis` through the middle of `region`, seen along
    /// the axis.
    pub fn through_middle(axis: Axis, region: &WorldBox) -> Line {
        let (min, max) = (region.min(), region.max());
        let doubled = axis
            .others()
            .map(|other| i128::from(min.get(other).nm()) + i128::from(max.get(other).nm()));
        Line { axis, doubled }
    }

    /// The axis it runs along.
    pub fn axis(&self) -> Axis {
        self.axis
    }

    /// Twice its coordinate on `other`, one of the two other axes.
    fn doubled_on(&self, other: Axis) -> Option<i128> {
        let [first, second] = self.axis.others();
        if other == first {
            Some(self.doubled[0])
        } else if other == second {
            Some(self.doubled[1])
        } else {
            None
        }
    }

    /// The stretch of the line inside a box, its surface included.
    pub fn in_box(&self, region: &WorldBox) -> Option<Stretch> {
        let (min, max) = (region.min(), region.max());
        let inside = self.axis.others().into_iter().all(|other| {
            self.doubled_on(other).is_some_and(|point| {
                2 * i128::from(min.get(other).nm()) <= point
                    && point <= 2 * i128::from(max.get(other).nm())
            })
        });
        inside.then(|| Stretch::new(min.get(self.axis), max.get(self.axis)))
    }

    /// Every stretch of the line inside a solid's wood, its surface
    /// included, in no particular order.
    pub fn in_solid(&self, solid: &WorldSolid) -> Vec<Stretch> {
        let boxes = solid
            .boxes()
            .iter()
            .filter_map(|region| self.in_box(region));
        let rods = solid.rods().iter().filter_map(|rod| self.in_rod(rod));
        let prisms = solid
            .prisms()
            .iter()
            .filter_map(|prism| self.in_prism(prism));
        let slants = solid
            .slant_bounds()
            .filter_map(|region| self.in_box(&region));
        boxes.chain(rods).chain(prisms).chain(slants).collect()
    }

    /// The stretch inside a cylinder along a world axis.
    fn in_rod(&self, rod: &WorldRod) -> Option<Stretch> {
        let diameter = i128::from(rod.diameter.nm());
        let [first, second] = rod.axis.others();
        let doubled_centre = |axis: Axis| -> Option<i128> {
            if axis == first {
                Some(2 * i128::from(rod.centre[0].nm()))
            } else if axis == second {
                Some(2 * i128::from(rod.centre[1].nm()))
            } else {
                None
            }
        };
        if rod.axis == self.axis {
            // Along the rod: inside its circle, for its whole length.
            let off_first = doubled_centre(first)? - self.doubled_on(first)?;
            let off_second = doubled_centre(second)? - self.doubled_on(second)?;
            let inside = off_first
                .checked_mul(off_first)?
                .checked_add(off_second.checked_mul(off_second)?)?
                <= diameter.checked_mul(diameter)?;
            return inside.then(|| Stretch::new(rod.start, rod.end));
        }
        // Across the rod: the line must lie between its ends, and then
        // crosses its circle as a chord.
        let along_rod = self.doubled_on(rod.axis)?;
        if along_rod < 2 * i128::from(rod.start.nm()) || along_rod > 2 * i128::from(rod.end.nm()) {
            return None;
        }
        let third = if first == self.axis { second } else { first };
        let off = doubled_centre(third)? - self.doubled_on(third)?;
        let room = diameter.checked_mul(diameter)? - off.checked_mul(off)?;
        if room < 0 {
            return None;
        }
        // Twice the half chord, to the nanometre.
        let chord = u128::try_from(room).ok()?.isqrt();
        let half = i64::try_from(chord / 2).ok()?;
        let centre = doubled_centre(self.axis)? / 2;
        let centre = i64::try_from(centre).ok()?;
        Some(Stretch::new(
            Length::from_nm(centre.checked_sub(half)?),
            Length::from_nm(centre.checked_add(half)?),
        ))
    }

    /// The stretch inside a convex prism.
    fn in_prism(&self, prism: &WorldPrism) -> Option<Stretch> {
        let [first, second] = prism.axis.others();
        let corners: Vec<[i128; 2]> = prism
            .corners
            .iter()
            .map(|[a, b]| [2 * i128::from(a.nm()), 2 * i128::from(b.nm())])
            .collect();
        if corners.len() < 3 {
            return None;
        }
        let edges = || {
            corners
                .iter()
                .zip(corners.iter().cycle().skip(1))
                .take(corners.len())
        };
        if prism.axis == self.axis {
            // Along the prism: inside its outline (on every edge's same
            // side), for its whole length.
            let point = [self.doubled_on(first)?, self.doubled_on(second)?];
            let mut sides = edges().map(|(a, b)| {
                let cross = (b[0] - a[0]) * (point[1] - a[1]) - (b[1] - a[1]) * (point[0] - a[0]);
                cross.signum()
            });
            let inside = sides.clone().all(|side| side >= 0) || sides.all(|side| side <= 0);
            return inside.then(|| Stretch::new(prism.start, prism.end));
        }
        // Across the prism: between its ends, and then through its outline
        // in the plane across its axis.
        let along_prism = self.doubled_on(prism.axis)?;
        if along_prism < 2 * i128::from(prism.start.nm())
            || along_prism > 2 * i128::from(prism.end.nm())
        {
            return None;
        }
        // Which corner coordinate runs along the line, and which is fixed.
        let (run, fixed) = if first == self.axis { (0, 1) } else { (1, 0) };
        let third = if first == self.axis { second } else { first };
        let level = self.doubled_on(third)?;
        let mut low: Option<i128> = None;
        let mut high: Option<i128> = None;
        let mut take = |value: i128| {
            low = Some(low.map_or(value, |low| low.min(value)));
            high = Some(high.map_or(value, |high| high.max(value)));
        };
        for (a, b) in edges() {
            let (a_level, b_level) = (a[fixed] - level, b[fixed] - level);
            if a_level == 0 {
                take(a[run]);
            }
            if (a_level < 0 && b_level > 0) || (a_level > 0 && b_level < 0) {
                // Where the edge crosses the level, by similar triangles.
                let crossing = a[run] + (b[run] - a[run]) * (-a_level) / (b_level - a_level);
                take(crossing);
            }
        }
        let (low, high) = (low?, high?);
        Some(Stretch::new(
            Length::from_nm(i64::try_from(low / 2).ok()?),
            Length::from_nm(i64::try_from(high / 2).ok()?),
        ))
    }

    /// The line as a body moved by `placement` sees it where it is at
    /// rest, and how a coordinate along it there maps back onto this line
    /// (grammar, "Clamps", at a pose): `Some` when the move leaves the line
    /// along the same world axis (a slide; a turn about a line along the
    /// axis, which takes a line on it to itself), exact for a slide and to
    /// the nanometre for a turn; `None` when the move slants it.
    pub fn seen_by(&self, placement: &Placement) -> Option<(Line, AlongMap)> {
        let [first, second] = self.axis.others();
        match placement {
            Placement::Exact(offset) => {
                let back = |on: Axis, doubled: i128| {
                    doubled.checked_sub(2 * i128::from(offset.get(on).nm()))
                };
                Some((
                    Line {
                        axis: self.axis,
                        doubled: [
                            back(first, self.doubled[0])?,
                            back(second, self.doubled[1])?,
                        ],
                    },
                    AlongMap {
                        reversed: false,
                        shift: offset.get(self.axis),
                    },
                ))
            }
            Placement::Turned(matrix) => {
                let inverse = matrix.inverse();
                let unit = point_vector(Point::default().with(self.axis, Length::from_nm(1)));
                let unit_on =
                    |axis: Axis| point_vector(Point::default().with(axis, Length::from_nm(1)));
                // Half a doubled coordinate, in nanometres.
                let half = |doubled: i128| -> Option<f64> {
                    let whole = i64::try_from(doubled.div_euclid(2)).ok()?;
                    let rest = if doubled.rem_euclid(2) == 0 { 0.0 } else { 0.5 };
                    Some(render::nm(Length::from_nm(whole)) + rest)
                };
                let on_line = unit_on(first) * half(self.doubled[0])?
                    + unit_on(second) * half(self.doubled[1])?;
                let direction = inverse.transform_vector3(unit);
                if direction.cross(unit).length() > 1e-9 {
                    return None;
                }
                let reversed = direction.dot(unit) < 0.0;
                let seen = inverse.transform_point3(on_line);
                let coordinate = |axis: Axis| seen.dot(unit_on(axis));
                let doubled = |axis: Axis| -> Option<i128> {
                    let value = (2.0 * coordinate(axis)).round();
                    render::length_from_nm(value).map(|length| i128::from(length.nm()))
                };
                // A coordinate `u'` along the seen line is `±u' + shift`
                // along this one.
                let along = coordinate(self.axis);
                let shift = if reversed { along } else { -along };
                Some((
                    Line {
                        axis: self.axis,
                        doubled: [doubled(first)?, doubled(second)?],
                    },
                    AlongMap {
                        reversed,
                        shift: render::length_from_nm(shift.round())?,
                    },
                ))
            }
        }
    }
}

/// How a coordinate along a line, as a moved body sees it at rest, maps
/// back onto the line where the body is ([`Line::seen_by`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlongMap {
    reversed: bool,
    shift: Length,
}

impl AlongMap {
    /// A stretch the body sees, where it is on the line; `None` past
    /// `i64` nanometres (never a design's).
    pub fn stretch(&self, seen: Stretch) -> Option<Stretch> {
        let map = |at: Length| -> Option<Length> {
            let own = if self.reversed {
                at.nm().checked_neg()?
            } else {
                at.nm()
            };
            own.checked_add(self.shift.nm()).map(Length::from_nm)
        };
        Some(Stretch::new(map(seen.low)?, map(seen.high)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stave_model::solved::Point;

    fn mm(value: i64) -> Length {
        Length::from_mm(value)
    }

    fn point(across: i64, up: i64, deep: i64) -> Point {
        Point::default()
            .with(Axis::Across, mm(across))
            .with(Axis::Up, mm(up))
            .with(Axis::Deep, mm(deep))
    }

    fn stretch(low: i64, high: i64) -> Stretch {
        Stretch {
            low: mm(low),
            high: mm(high),
        }
    }

    #[test]
    fn a_line_through_the_middle_of_a_box_runs_its_whole_length() {
        let region = WorldBox::new(point(0, 0, 0), point(45, 70, 1500));
        let line = Line::through_middle(Axis::Across, &region);
        assert_eq!(line.in_box(&region), Some(stretch(0, 45)));
        let beside = WorldBox::new(point(0, 71, 0), point(45, 80, 10));
        assert_eq!(line.in_box(&beside), None);
    }

    #[test]
    fn a_line_on_a_face_meets_the_box() {
        let region = WorldBox::new(point(0, 0, 0), point(10, 10, 10));
        let line = Line::through_middle(
            Axis::Across,
            &WorldBox::new(point(0, 0, 0), point(1, 20, 20)),
        );
        assert_eq!(line.in_box(&region), Some(stretch(0, 10)));
    }

    #[test]
    fn a_line_along_a_rod_is_inside_it_only_within_its_circle() {
        let rod = WorldRod {
            axis: Axis::Across,
            centre: [mm(0), mm(0)],
            diameter: mm(10),
            start: mm(-5),
            end: mm(5),
        };
        let at = |up: i64, deep: i64| {
            Line::through_middle(
                Axis::Across,
                &WorldBox::new(point(0, up, deep), point(0, up, deep)),
            )
        };
        assert_eq!(at(0, 0).in_rod(&rod), Some(stretch(-5, 5)));
        assert_eq!(at(3, 4).in_rod(&rod), Some(stretch(-5, 5)));
        assert_eq!(at(4, 4).in_rod(&rod), None);
    }

    #[test]
    fn a_line_across_a_rod_crosses_it_as_a_chord() {
        let rod = WorldRod {
            axis: Axis::Up,
            centre: [mm(0), mm(0)],
            diameter: mm(10),
            start: mm(0),
            end: mm(100),
        };
        let line = Line::through_middle(
            Axis::Across,
            &WorldBox::new(point(0, 50, 3), point(0, 50, 3)),
        );
        assert_eq!(line.in_rod(&rod), Some(stretch(-4, 4)));
        let above = Line::through_middle(
            Axis::Across,
            &WorldBox::new(point(0, 101, 0), point(0, 101, 0)),
        );
        assert_eq!(above.in_rod(&rod), None);
    }

    #[test]
    fn a_line_across_a_prism_runs_between_its_edges() {
        // A triangle across `up` and `deep`, extruded along `across`.
        let prism = WorldPrism {
            axis: Axis::Across,
            corners: vec![[mm(0), mm(0)], [mm(10), mm(0)], [mm(0), mm(10)]],
            start: mm(0),
            end: mm(5),
        };
        let along =
            Line::through_middle(Axis::Across, &WorldBox::new(point(0, 2, 2), point(0, 2, 2)));
        assert_eq!(along.in_prism(&prism), Some(stretch(0, 5)));
        let outside =
            Line::through_middle(Axis::Across, &WorldBox::new(point(0, 8, 8), point(0, 8, 8)));
        assert_eq!(outside.in_prism(&prism), None);
        let across = Line::through_middle(Axis::Up, &WorldBox::new(point(2, 0, 4), point(2, 0, 4)));
        assert_eq!(across.in_prism(&prism), Some(stretch(0, 6)));
    }

    /// A clamp at a pose (grammar, "Clamps"): a body moved straight sees
    /// the line moved back; one turned about a line along the axis
    /// through the clamp's line sees the line itself; one turned about
    /// another axis sees it slanted.
    #[test]
    fn a_line_seen_by_a_moved_body() {
        let region = WorldBox::new(point(0, 0, 0), point(45, 70, 1500));
        let at = |up: i64| {
            Line::through_middle(
                Axis::Across,
                &WorldBox::new(point(0, up, 750), point(0, up, 750)),
            )
        };
        let line = at(35);
        // Moved 100mm up and 10mm across: it sees a line 135mm up where it
        // was at rest 35mm up, and what it fills is 10mm further across.
        let up = Placement::Exact(point(10, 100, 0));
        let (seen, back) = at(135).seen_by(&up).unwrap();
        assert_eq!(seen, line);
        assert_eq!(
            seen.in_box(&region).and_then(|s| back.stretch(s)),
            Some(stretch(10, 55))
        );
        // The line it was on at rest now runs below it.
        let (below, _) = line.seen_by(&up).unwrap();
        assert_eq!(below.in_box(&region), None);
        // Turned a quarter about the line itself: the same stretch.
        let through = glam::DVec3::new(0.0, 35e6, 750e6);
        let turn = glam::DMat4::from_translation(through)
            * glam::DMat4::from_rotation_x(std::f64::consts::FRAC_PI_2)
            * glam::DMat4::from_translation(-through);
        let (seen, back) = line.seen_by(&Placement::Turned(turn)).unwrap();
        assert_eq!(seen, line);
        assert_eq!(
            seen.in_box(&region).and_then(|s| back.stretch(s)),
            Some(stretch(0, 45))
        );
        // Turned about `up`: slanted across the line.
        let slant = glam::DMat4::from_rotation_y(0.3);
        assert_eq!(line.seen_by(&Placement::Turned(slant)), None);
    }
}
