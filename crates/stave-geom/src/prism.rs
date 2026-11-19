//! Prisms: a flat shape marked on a face and cut straight down from it.
//!
//! Every angled cut (`dovetail_socket`, `dovetail_tail`, `outline` and a
//! `slot` turned by an angle) is a [`Polygon`] in the plane of a face,
//! extruded along the face's axis between two depths ([`DepthRange`]). A
//! [`Prism`] keeps the polygon as marked and its split into convex pieces
//! ([`ConvexPolygon`]), which is what collision tests and meshes use.
//!
//! Everything here is exact: points are whole nanometres ([`Point2`]),
//! orientation and crossing tests are `i128` products of `i64` differences
//! (a 4m part is about 2^42 nm, so a product is about 2^86, well inside
//! `i128`), and the convex split adds no points. It is deterministic:
//! ear clipping takes the first ear in vertex order, and the merge
//! (Hertel–Mehlhorn) tries pieces in a fixed order.

use stave_lang::placement::PartAxis;
use stave_lang::units::Length;

/// A point in the plane of a face, in nanometres, on the two part axes
/// across the face's axis in [`across`](crate::solid::across) order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Point2 {
    /// Along the first axis across the face's.
    pub first: Length,
    /// Along the second.
    pub second: Length,
}

impl Point2 {
    /// A point from its two coordinates.
    pub const fn new(first: Length, second: Length) -> Point2 {
        Point2 { first, second }
    }

    fn x(self) -> i128 {
        i128::from(self.first.nm())
    }

    fn y(self) -> i128 {
        i128::from(self.second.nm())
    }
}

/// Twice the signed area of the triangle `a b c`: positive when it turns
/// counter-clockwise, zero when the points lie on one line.
fn orientation(a: Point2, b: Point2, c: Point2) -> i128 {
    (b.x() - a.x()) * (c.y() - a.y()) - (b.y() - a.y()) * (c.x() - a.x())
}

/// The dot product of `b - a` and `c - b`.
fn onward(a: Point2, b: Point2, c: Point2) -> i128 {
    (b.x() - a.x()) * (c.x() - b.x()) + (b.y() - a.y()) * (c.y() - b.y())
}

/// Is `p` on the closed segment `a b` (knowing the three are collinear)?
fn within(a: Point2, b: Point2, p: Point2) -> bool {
    p.x() >= a.x().min(b.x())
        && p.x() <= a.x().max(b.x())
        && p.y() >= a.y().min(b.y())
        && p.y() <= a.y().max(b.y())
}

/// Do the closed segments `a b` and `c d` share a point?
fn segments_meet(a: Point2, b: Point2, c: Point2, d: Point2) -> bool {
    let (o1, o2) = (orientation(a, b, c), orientation(a, b, d));
    let (o3, o4) = (orientation(c, d, a), orientation(c, d, b));
    if o1.signum() * o2.signum() < 0 && o3.signum() * o4.signum() < 0 {
        return true;
    }
    (o1 == 0 && within(a, b, c))
        || (o2 == 0 && within(a, b, d))
        || (o3 == 0 && within(c, d, a))
        || (o4 == 0 && within(c, d, b))
}

/// Twice the signed area of a ring of points (positive counter-clockwise).
fn twice_area(points: &[Point2]) -> i128 {
    let mut sum = 0i128;
    for (index, p) in points.iter().enumerate() {
        if let Some(q) = points.get((index + 1) % points.len()) {
            sum += p.x() * q.y() - q.x() * p.y();
        }
    }
    sum
}

/// Why a list of points is not a polygon. Marks are counted from 1, the
/// way a beginner counts them in the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
pub enum PolygonError {
    /// Fewer than three points.
    #[error("an outline needs at least three marks, found {count}")]
    TooFewPoints {
        /// How many there are.
        count: usize,
    },
    /// The same point twice.
    #[error("mark {first} and mark {second} are the same point")]
    RepeatedPoint {
        /// The first of the two, from 1.
        first: usize,
        /// The second, from 1.
        second: usize,
    },
    /// Two of its sides cross or touch.
    #[error("the outline crosses itself between mark {from} and mark {to}")]
    CrossesItself {
        /// The side that crosses another starts at this mark, from 1.
        from: usize,
        /// And ends at this one.
        to: usize,
        /// The side it crosses starts at this mark.
        other_from: usize,
        /// And ends at this one.
        other_to: usize,
    },
    /// All its points lie on one line.
    #[error("the outline's marks all lie on one line, so it cuts nothing")]
    NoArea,
    /// A collision piece that turns the wrong way somewhere.
    #[error("the shape is not convex at point {at}")]
    NotConvex {
        /// The point where it turns inward, from 1.
        at: usize,
    },
}

impl PolygonError {
    /// A concrete help line for the diagnostic.
    pub fn help(&self) -> String {
        match self {
            PolygonError::TooFewPoints { .. } => {
                "mark at least three corners of the shape, in order around it".to_owned()
            }
            PolygonError::RepeatedPoint { second, .. } => {
                format!("remove mark {second}, or move it to the corner it was meant for")
            }
            PolygonError::CrossesItself {
                from,
                to,
                other_from,
                other_to,
            } => format!(
                "the marks go round the shape in order; the side from mark {from} to mark {to} meets the side from mark {other_from} to mark {other_to}, so reorder the marks or move one"
            ),
            PolygonError::NoArea => {
                "move a mark off the line so the marks go round a shape".to_owned()
            }
            PolygonError::NotConvex { at } => {
                format!("move point {at} outward, or split the shape in two")
            }
        }
    }
}

/// A simple polygon: at least three points, none repeated, sides that
/// never cross or touch except where neighbours meet, a positive area.
/// Its points go counter-clockwise, and no point lies on the straight line
/// between its neighbours.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Polygon {
    points: Vec<Point2>,
}

impl Polygon {
    /// Checks a ring of points (the last joins the first) and normalises
    /// it: turned counter-clockwise, points in the middle of a straight
    /// side dropped.
    pub fn new(points: Vec<Point2>) -> Result<Polygon, PolygonError> {
        let count = points.len();
        if count < 3 {
            return Err(PolygonError::TooFewPoints { count });
        }
        for (i, p) in points.iter().enumerate() {
            if let Some(j) = points.iter().skip(i + 1).position(|q| q == p) {
                return Err(PolygonError::RepeatedPoint {
                    first: i + 1,
                    second: i + j + 2,
                });
            }
        }
        let point = |i: usize| points.get(i % count).copied();
        // Sides i (from point i to i + 1) and j > i.
        for j in 0..count {
            for i in 0..j {
                let (Some(a), Some(b), Some(c), Some(d)) =
                    (point(i), point(i + 1), point(j), point(j + 1))
                else {
                    continue;
                };
                let neighbours = j == i + 1 || (i == 0 && j == count - 1);
                let crosses = if neighbours {
                    // Neighbours share a point; they cross only by folding
                    // back over each other.
                    let (first, shared, last) = if j == i + 1 { (a, b, d) } else { (c, a, b) };
                    orientation(first, shared, last) == 0 && onward(first, shared, last) < 0
                } else {
                    segments_meet(a, b, c, d)
                };
                if crosses {
                    return Err(PolygonError::CrossesItself {
                        from: j + 1,
                        to: (j + 1) % count + 1,
                        other_from: i + 1,
                        other_to: (i + 1) % count + 1,
                    });
                }
            }
        }
        let mut ring = points;
        let area = twice_area(&ring);
        if area == 0 {
            return Err(PolygonError::NoArea);
        }
        if area < 0 {
            ring.reverse();
        }
        drop_straight(&mut ring);
        if ring.len() < 3 {
            return Err(PolygonError::NoArea);
        }
        Ok(Polygon { points: ring })
    }

    /// The points, counter-clockwise.
    pub fn points(&self) -> &[Point2] {
        &self.points
    }

    /// Twice the area, in square nanometres (always positive).
    pub fn twice_area(&self) -> i128 {
        twice_area(&self.points)
    }

    /// Is every turn a left turn?
    pub fn is_convex(&self) -> bool {
        convex_at(&self.points).is_none()
    }

    /// Splits the polygon into convex pieces that cover it exactly and
    /// share only sides: ear clipping, then neighbouring pieces merged
    /// while the result stays convex. No point is added.
    pub fn convex_pieces(&self) -> Vec<ConvexPolygon> {
        let mut pieces = triangulate(&self.points);
        merge_convex(&mut pieces);
        pieces
            .into_iter()
            .filter_map(|points| ConvexPolygon::new(points).ok())
            .collect()
    }

    /// Is the polygon an axis-aligned rectangle?
    pub fn is_rectangle(&self) -> bool {
        is_rectangle(&self.points)
    }
}

/// Drops points in the middle of a straight side (the ring is simple).
fn drop_straight(ring: &mut Vec<Point2>) {
    loop {
        let count = ring.len();
        if count < 3 {
            return;
        }
        let straight = (0..count).find(|&i| {
            let previous = ring.get((i + count - 1) % count);
            let here = ring.get(i);
            let next = ring.get((i + 1) % count);
            matches!((previous, here, next), (Some(a), Some(b), Some(c)) if orientation(*a, *b, *c) == 0)
        });
        match straight {
            Some(i) => {
                ring.remove(i);
            }
            None => return,
        }
    }
}

/// The first point (from 0) where a counter-clockwise ring turns right or
/// goes straight on, if any.
fn convex_at(points: &[Point2]) -> Option<usize> {
    let count = points.len();
    (0..count).find(|&i| {
        let previous = points.get((i + count - 1) % count);
        let here = points.get(i);
        let next = points.get((i + 1) % count);
        match (previous, here, next) {
            (Some(a), Some(b), Some(c)) => orientation(*a, *b, *c) <= 0,
            _ => true,
        }
    })
}

fn is_rectangle(points: &[Point2]) -> bool {
    if points.len() != 4 {
        return false;
    }
    let count = points.len();
    (0..count).all(|i| match (points.get(i), points.get((i + 1) % count)) {
        (Some(a), Some(b)) => a.first == b.first || a.second == b.second,
        _ => false,
    })
}

/// Ear clipping of a counter-clockwise simple ring with no straight
/// points: each step cuts off the first ear in ring order.
fn triangulate(points: &[Point2]) -> Vec<Vec<Point2>> {
    let mut ring: Vec<Point2> = points.to_vec();
    let mut triangles = Vec::new();
    while ring.len() > 3 {
        let count = ring.len();
        let ear = (0..count).find(|&i| {
            let (Some(&a), Some(&b), Some(&c)) = (
                ring.get((i + count - 1) % count),
                ring.get(i),
                ring.get((i + 1) % count),
            ) else {
                return false;
            };
            if orientation(a, b, c) <= 0 {
                return false;
            }
            // No other point inside the triangle or on its sides.
            !ring
                .iter()
                .any(|&p| p != a && p != b && p != c && in_closed_triangle(a, b, c, p))
        });
        let Some(i) = ear else {
            // Never for a simple polygon; keep what is left as one piece
            // rather than lose it.
            break;
        };
        if let (Some(&a), Some(&b), Some(&c)) = (
            ring.get((i + count - 1) % count),
            ring.get(i),
            ring.get((i + 1) % count),
        ) {
            triangles.push(vec![a, b, c]);
        }
        ring.remove(i);
    }
    triangles.push(ring);
    triangles
}

fn in_closed_triangle(a: Point2, b: Point2, c: Point2, p: Point2) -> bool {
    orientation(a, b, p) >= 0 && orientation(b, c, p) >= 0 && orientation(c, a, p) >= 0
}

/// Hertel–Mehlhorn: merges pieces that share a side while the union stays
/// convex, trying pairs in a fixed order until none merges.
fn merge_convex(pieces: &mut Vec<Vec<Point2>>) {
    loop {
        let mut merged = None;
        'search: for i in 0..pieces.len() {
            for j in i + 1..pieces.len() {
                if let (Some(p), Some(q)) = (pieces.get(i), pieces.get(j))
                    && let Some(union) = join(p, q)
                    && convex_at(&union).is_none()
                {
                    merged = Some((i, j, union));
                    break 'search;
                }
            }
        }
        let Some((i, j, union)) = merged else {
            return;
        };
        pieces.remove(j);
        if let Some(slot) = pieces.get_mut(i) {
            *slot = union;
        }
    }
}

/// Two counter-clockwise rings that share a side (`a → b` in `p`, `b → a`
/// in `q`) as one ring, straight points dropped.
fn join(p: &[Point2], q: &[Point2]) -> Option<Vec<Point2>> {
    let (np, nq) = (p.len(), q.len());
    for i in 0..np {
        let (Some(&a), Some(&b)) = (p.get(i), p.get((i + 1) % np)) else {
            continue;
        };
        let Some(k) = (0..nq).find(|&k| q.get(k) == Some(&b) && q.get((k + 1) % nq) == Some(&a))
        else {
            continue;
        };
        // p from b round to a, then q's points strictly between a and b.
        let mut ring: Vec<Point2> = (0..np)
            .filter_map(|s| p.get((i + 1 + s) % np).copied())
            .collect();
        ring.extend((0..nq.saturating_sub(2)).filter_map(|s| q.get((k + 2 + s) % nq).copied()));
        drop_straight(&mut ring);
        return Some(ring);
    }
    None
}

/// A convex polygon: a [`Polygon`] whose every turn is a left turn.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConvexPolygon {
    polygon: Polygon,
}

impl ConvexPolygon {
    /// Checks a ring of points like [`Polygon::new`], and that it is
    /// convex.
    pub fn new(points: Vec<Point2>) -> Result<ConvexPolygon, PolygonError> {
        let polygon = Polygon::new(points)?;
        match convex_at(&polygon.points) {
            Some(at) => Err(PolygonError::NotConvex { at: at + 1 }),
            None => Ok(ConvexPolygon { polygon }),
        }
    }

    /// The points, counter-clockwise.
    pub fn points(&self) -> &[Point2] {
        self.polygon.points()
    }

    /// Is it an axis-aligned rectangle (a cut a block can make)?
    pub fn is_rectangle(&self) -> bool {
        self.polygon.is_rectangle()
    }

    /// The box around it: (low, high) per axis.
    pub fn bounds(&self) -> (Point2, Point2) {
        let points = self.points();
        let mut low = points
            .first()
            .copied()
            .unwrap_or(Point2::new(Length::ZERO, Length::ZERO));
        let mut high = low;
        for p in points {
            low = Point2::new(low.first.min(p.first), low.second.min(p.second));
            high = Point2::new(high.first.max(p.first), high.second.max(p.second));
        }
        (low, high)
    }

    /// Twice the area, in square nanometres.
    pub fn twice_area(&self) -> i128 {
        self.polygon.twice_area()
    }

    /// Does the polygon hold `p` strictly inside (not on a side)?
    pub fn holds(&self, p: Point2) -> bool {
        let points = self.points();
        let count = points.len();
        (0..count).all(|i| match (points.get(i), points.get((i + 1) % count)) {
            (Some(a), Some(b)) => orientation(*a, *b, p) > 0,
            _ => false,
        })
    }
}

/// Where a prism runs along its face's axis: `from < to`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DepthRange {
    from: Length,
    to: Length,
}

impl DepthRange {
    /// The range between two local coordinates; `None` unless
    /// `from < to`.
    pub fn new(from: Length, to: Length) -> Option<DepthRange> {
        (from < to).then_some(DepthRange { from, to })
    }

    /// The low end.
    pub fn from(&self) -> Length {
        self.from
    }

    /// The high end.
    pub fn to(&self) -> Length {
        self.to
    }
}

/// A cut that is a polygon on a face cut straight down: the polygon as
/// marked, its convex pieces, and the range it covers along the face's
/// axis.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Prism {
    face_axis: PartAxis,
    outline: Polygon,
    pieces: Vec<ConvexPolygon>,
    depth: DepthRange,
}

impl Prism {
    /// The prism of a polygon over a depth range; its convex pieces are
    /// worked out once, here. A simple polygon always has some; an error
    /// says it had none.
    pub fn new(
        face_axis: PartAxis,
        outline: Polygon,
        depth: DepthRange,
    ) -> Result<Prism, PolygonError> {
        let pieces = outline.convex_pieces();
        if pieces.is_empty() {
            return Err(PolygonError::NoArea);
        }
        Ok(Prism {
            face_axis,
            outline,
            pieces,
            depth,
        })
    }

    /// The axis it is cut along.
    pub fn face_axis(&self) -> PartAxis {
        self.face_axis
    }

    /// The polygon as marked (counter-clockwise).
    pub fn outline(&self) -> &Polygon {
        &self.outline
    }

    /// Its convex pieces: never empty, together exactly the outline.
    pub fn pieces(&self) -> &[ConvexPolygon] {
        &self.pieces
    }

    /// The pieces that are not axis-aligned rectangles: what a box can't
    /// make.
    pub fn slanted_pieces(&self) -> impl Iterator<Item = &ConvexPolygon> {
        self.pieces.iter().filter(|piece| !piece.is_rectangle())
    }

    /// Does it have a piece a box can't make?
    pub fn is_slanted(&self) -> bool {
        self.slanted_pieces().next().is_some()
    }

    /// The range it covers along its axis.
    pub fn depth(&self) -> DepthRange {
        self.depth
    }

    /// The same prism moved `by` along a part axis (a board's cut moved
    /// into its part's frame). Moving keeps every shape as it is, so
    /// nothing is checked again; `None` if a coordinate would overflow.
    pub fn shifted(&self, axis: PartAxis, by: Length) -> Option<Prism> {
        let move_by = |value: Length| value.checked_add(by);
        if axis == self.face_axis {
            let depth = DepthRange::new(move_by(self.depth.from)?, move_by(self.depth.to)?)?;
            return Some(Prism {
                depth,
                ..self.clone()
            });
        }
        let [first, _] = crate::solid::across(self.face_axis);
        let move_point = |p: &Point2| -> Option<Point2> {
            Some(if axis == first {
                Point2::new(move_by(p.first)?, p.second)
            } else {
                Point2::new(p.first, move_by(p.second)?)
            })
        };
        let move_ring =
            |points: &[Point2]| -> Option<Vec<Point2>> { points.iter().map(move_point).collect() };
        Some(Prism {
            face_axis: self.face_axis,
            outline: Polygon {
                points: move_ring(&self.outline.points)?,
            },
            pieces: self
                .pieces
                .iter()
                .map(|piece| {
                    Some(ConvexPolygon {
                        polygon: Polygon {
                            points: move_ring(&piece.polygon.points)?,
                        },
                    })
                })
                .collect::<Option<_>>()?,
            depth: self.depth,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(first: i64, second: i64) -> Point2 {
        Point2::new(Length::from_mm(first), Length::from_mm(second))
    }

    #[test]
    fn a_shifted_prism_moves_its_outline_or_its_depth() {
        // A trapezoid on the plane across the thickness, 0..19 deep.
        let trapezoid = Polygon::new(vec![p(0, 0), p(10, 0), p(8, 5), p(2, 5)]).unwrap();
        let depth = DepthRange::new(Length::ZERO, Length::from_mm(19)).unwrap();
        let prism = Prism::new(PartAxis::Thickness, trapezoid, depth).unwrap();
        // Along the width (the second axis on the face): every point moves.
        let moved = prism
            .shifted(PartAxis::Width, Length::from_mm(140))
            .unwrap();
        assert_eq!(
            moved.outline().points(),
            Prism::new(
                PartAxis::Thickness,
                Polygon::new(vec![p(0, 140), p(10, 140), p(8, 145), p(2, 145)]).unwrap(),
                depth
            )
            .unwrap()
            .outline()
            .points()
        );
        assert_eq!(moved.pieces().len(), prism.pieces().len());
        assert_eq!(moved.depth(), depth);
        // Along its own axis: only the depth moves.
        let deeper = prism
            .shifted(PartAxis::Thickness, Length::from_mm(5))
            .unwrap();
        assert_eq!(deeper.outline(), prism.outline());
        assert_eq!(
            (deeper.depth().from(), deeper.depth().to()),
            (Length::from_mm(5), Length::from_mm(24))
        );
    }

    #[test]
    fn a_polygon_that_crosses_itself_is_rejected() {
        // A bow tie: the side from mark 3 to mark 4 crosses the side from
        // mark 1 to mark 2.
        let error = Polygon::new(vec![p(0, 0), p(10, 10), p(10, 0), p(0, 10)]).unwrap_err();
        assert_eq!(
            error,
            PolygonError::CrossesItself {
                from: 3,
                to: 4,
                other_from: 1,
                other_to: 2
            }
        );
        assert_eq!(
            error.to_string(),
            "the outline crosses itself between mark 3 and mark 4"
        );
        // A side that folds back over its neighbour.
        assert!(matches!(
            Polygon::new(vec![p(0, 0), p(10, 0), p(5, 0), p(5, 5)]),
            Err(PolygonError::CrossesItself { .. })
        ));
        // A point that touches another side.
        assert!(matches!(
            Polygon::new(vec![p(0, 0), p(10, 0), p(10, 10), p(5, 0)]),
            Err(PolygonError::CrossesItself { .. })
        ));
    }

    #[test]
    fn polygons_need_three_distinct_points_and_an_area() {
        assert_eq!(
            Polygon::new(vec![p(0, 0), p(1, 1)]),
            Err(PolygonError::TooFewPoints { count: 2 })
        );
        assert_eq!(
            Polygon::new(vec![p(0, 0), p(1, 1), p(2, 0), p(1, 1)]),
            Err(PolygonError::RepeatedPoint {
                first: 2,
                second: 4
            })
        );
        assert!(Polygon::new(vec![p(0, 0), p(1, 0), p(2, 0)]).is_err());
    }

    #[test]
    fn polygons_turn_counter_clockwise_and_lose_straight_points() {
        let square = Polygon::new(vec![p(0, 0), p(0, 10), p(5, 10), p(10, 10), p(10, 0)]).unwrap();
        assert_eq!(square.points(), &[p(10, 0), p(10, 10), p(0, 10), p(0, 0)]);
        assert!(square.is_convex());
        assert!(square.is_rectangle());
        assert_eq!(square.twice_area(), 2 * 100 * 1_000_000_000_000);
    }

    #[test]
    fn an_l_shaped_outline_splits_into_two_convex_pieces() {
        let l = Polygon::new(vec![
            p(0, 0),
            p(20, 0),
            p(20, 10),
            p(10, 10),
            p(10, 30),
            p(0, 30),
        ])
        .unwrap();
        let pieces = l.convex_pieces();
        assert_eq!(pieces.len(), 2);
        let total: i128 = pieces.iter().map(ConvexPolygon::twice_area).sum();
        assert_eq!(total, l.twice_area());
        assert!(
            pieces
                .iter()
                .all(|piece| Polygon::new(piece.points().to_vec()).is_ok_and(|p| p.is_convex()))
        );
    }

    #[test]
    fn convex_split_is_the_same_every_run() {
        let shape = vec![
            p(0, 0),
            p(40, 0),
            p(40, 15),
            p(30, 25),
            p(35, 40),
            p(10, 40),
            p(15, 20),
            p(0, 20),
        ];
        let first = Polygon::new(shape.clone()).unwrap().convex_pieces();
        for _ in 0..10 {
            assert_eq!(Polygon::new(shape.clone()).unwrap().convex_pieces(), first);
        }
        let total: i128 = first.iter().map(ConvexPolygon::twice_area).sum();
        assert_eq!(total, Polygon::new(shape).unwrap().twice_area());
    }

    #[test]
    fn a_convex_polygon_holds_only_its_inside() {
        let triangle = ConvexPolygon::new(vec![p(0, 0), p(10, 0), p(0, 10)]).unwrap();
        assert!(triangle.holds(p(2, 2)));
        assert!(!triangle.holds(p(5, 5)));
        assert!(!triangle.holds(p(0, 3)));
        assert_eq!(
            ConvexPolygon::new(vec![
                p(0, 0),
                p(20, 0),
                p(20, 10),
                p(10, 10),
                p(10, 30),
                p(0, 30)
            ]),
            Err(PolygonError::NotConvex { at: 4 })
        );
    }

    #[test]
    fn depth_ranges_run_forward() {
        assert!(DepthRange::new(Length::from_mm(1), Length::from_mm(1)).is_none());
        assert!(DepthRange::new(Length::from_mm(2), Length::from_mm(1)).is_none());
        assert!(DepthRange::new(Length::from_mm(1), Length::from_mm(2)).is_some());
    }
}
