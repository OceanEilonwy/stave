//! Where a grip sits in the scene (docs/editing.md, "Grips"): the faces a
//! length is measured between, the handle on a face of the envelope, the
//! boundary between two neighbours of a split.
//!
//! This is geometry only. It takes the description of a grip (which
//! quantity, which item, which axis) and returns places at rest; what is
//! draggable, and what a drag writes, is the editor's business. Faces are
//! found and compared exactly; only the handle's point and direction are
//! in the render frame.

use glam::Vec3;
use stave_geom::render;
use stave_lang::placement::{Axis, Face};
use stave_lang::units::Length;
use stave_model::solved::{AssemblyId, PartId, Point, RegionId, WorldBox};

use crate::graph::{Item, SceneGraph};

/// The description of a grip: which quantity it moves, on which item,
/// along which axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GripOf {
    /// A param given in an assembly that is a length along an axis, with
    /// its value now (`divider.height`, up, 500mm).
    Length {
        /// The assembly.
        assembly: AssemblyId,
        /// The world axis the length runs along.
        axis: Axis,
        /// Its value in the design as solved.
        value: Length,
    },
    /// A size of the envelope: the box region and the face its handle is
    /// on (`overall_width` on the box's right face).
    Envelope {
        /// The box.
        region: RegionId,
        /// The face that moves when the size changes.
        face: Face,
    },
    /// A share of a split: the boundary between two neighbouring regions
    /// along the split's axis.
    Share {
        /// The neighbour on the low side (left, bottom, front).
        low: RegionId,
        /// The neighbour on the high side (right, top, back).
        high: RegionId,
        /// The split's axis.
        axis: Axis,
    },
}

/// What a face belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FaceOf {
    /// A part's drawn box.
    Part(PartId),
    /// A region's box.
    Region(RegionId),
}

/// One face of a part's drawn box or of a region, and where its plane
/// lies along the face's axis, exact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GripFace {
    /// Whose face.
    pub of: FaceOf,
    /// Which face.
    pub face: Face,
    /// Where its plane is along the face's axis.
    pub at: Length,
}

/// A grip's handle in the render frame: where it is drawn and which way
/// dragging makes the quantity larger.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GripHandle {
    /// Where it is drawn, millimetres.
    pub point: Vec3,
    /// Unit vector: dragging along it makes the quantity larger.
    pub direction: Vec3,
}

/// Where a grip sits.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GripPlace {
    /// A length measured between two faces: from `from` to `to`, the
    /// handle on `to`.
    Length {
        /// The face it is measured from: outside the assembly (the base's
        /// top), or the near face of the part whose size it is.
        from: GripFace,
        /// The face that moves: a face of the assembly's part (the
        /// divider's underside).
        to: GripFace,
        /// The point on `from` opposite the handle, for a dimension line.
        start: Vec3,
        /// The handle, on `to`.
        handle: GripHandle,
    },
    /// An envelope size: the handle in the middle of the box's face.
    Envelope {
        /// The face.
        face: GripFace,
        /// The handle, pointing out of the box.
        handle: GripHandle,
    },
    /// A share: the handle in the middle of the boundary between the two
    /// neighbours, pointing toward the high one (dragging that way makes
    /// the low one's share larger).
    Share {
        /// Where the boundary lies along the split's axis.
        at: Length,
        /// The handle.
        handle: GripHandle,
    },
}

/// The low and high faces along an axis.
fn faces_along(axis: Axis) -> (Face, Face) {
    match axis {
        Axis::Across => (Face::Left, Face::Right),
        Axis::Up => (Face::Bottom, Face::Top),
        // Deep runs from the front to the back.
        Axis::Deep => (Face::Front, Face::Back),
    }
}

/// The middle of two lengths, exact to the nanometre.
fn middle(low: Length, high: Length) -> Length {
    let (low, high) = (low.nm(), high.nm());
    Length::from_nm(low.saturating_add(high.saturating_sub(low) / 2))
}

/// Where two boxes overlap on the axes other than `axis`, as a range per
/// axis with room in it; `None` when they don't overlap with room.
fn overlap(a: &WorldBox, b: &WorldBox, axis: Axis) -> Option<[(Axis, Length, Length); 2]> {
    let range = |other: Axis| {
        let low = a.min().get(other).max(b.min().get(other));
        let high = a.max().get(other).min(b.max().get(other));
        (low < high).then_some((other, low, high))
    };
    let [first, second] = axis.others();
    Some([range(first)?, range(second)?])
}

/// The area of an overlap, in square nanometres, to rank two pairs.
fn area(ranges: &[(Axis, Length, Length); 2]) -> i128 {
    ranges
        .iter()
        .map(|(_, low, high)| i128::from(high.nm().saturating_sub(low.nm())))
        .product()
}

/// The point at `at` along `axis`, in the middle of `ranges` across.
fn point_at(axis: Axis, at: Length, ranges: &[(Axis, Length, Length); 2]) -> Vec3 {
    let point = ranges.iter().fold(
        Point::default().with(axis, at),
        |point, (other, low, high)| point.with(*other, middle(*low, *high)),
    );
    render::point(point)
}

/// The render-frame direction along an axis toward its high side, or the
/// low side when `toward_high` is false.
fn along(axis: Axis, toward_high: bool) -> Vec3 {
    let (low, high) = faces_along(axis);
    render::direction(if toward_high { high } else { low }.direction())
}

/// A face that may be measured from, with the box it bounds.
struct Reference {
    of: FaceOf,
    bounds: WorldBox,
}

/// One way a length could be measured: from a reference face to a face
/// of the assembly's part.
struct Match {
    from: GripFace,
    to: GripFace,
    ranges: [(Axis, Length, Length); 2],
    /// Lower is better: a part's face over a region's, then the larger
    /// overlap, then the lower ids.
    rank: (u8, std::cmp::Reverse<i128>, FaceOf, PartId),
}

impl SceneGraph {
    /// Where a grip sits, at rest; `None` when the description names
    /// nothing in this scene, or (for a length) no two faces lie that far
    /// apart.
    ///
    /// - A length in an assembly is measured between a face of one of the
    ///   assembly's parts and a face outside the assembly that faces it
    ///   across the gap, exactly `value` apart, overlapping it across
    ///   (the base's top and the divider's underside for the divider's
    ///   height). A part's face is preferred to a region's in the same
    ///   plane, then the pair that overlaps most. When no gap is that
    ///   wide, a part of the assembly that is `value` long along the axis
    ///   is measured from its low face to its high face.
    /// - An envelope size has its handle in the middle of the box's face.
    /// - A share's handle is in the middle of the boundary between the two
    ///   neighbours, which must touch along the axis.
    pub fn grip_place(&self, grip: GripOf) -> Option<GripPlace> {
        match grip {
            GripOf::Length {
                assembly,
                axis,
                value,
            } => self.length_place(assembly, axis, value),
            GripOf::Envelope { region, face } => {
                let bounds = self.region(region)?.bounds;
                let axis = face.axis();
                let ranges = [
                    (axis.others()[0], bounds.min(), bounds.max()),
                    (axis.others()[1], bounds.min(), bounds.max()),
                ]
                .map(|(other, min, max)| (other, min.get(other), max.get(other)));
                let at = bounds.face(face);
                Some(GripPlace::Envelope {
                    face: GripFace {
                        of: FaceOf::Region(region),
                        face,
                        at,
                    },
                    handle: GripHandle {
                        point: point_at(axis, at, &ranges),
                        direction: render::direction(face.direction()),
                    },
                })
            }
            GripOf::Share { low, high, axis } => {
                let (low, high) = (self.region(low)?.bounds, self.region(high)?.bounds);
                let at = low.max().get(axis);
                if high.min().get(axis) != at {
                    return None;
                }
                let ranges = overlap(&low, &high, axis)?;
                Some(GripPlace::Share {
                    at,
                    handle: GripHandle {
                        point: point_at(axis, at, &ranges),
                        direction: along(axis, true),
                    },
                })
            }
        }
    }

    /// The faces a length in an assembly is measured between.
    fn length_place(&self, assembly: AssemblyId, axis: Axis, value: Length) -> Option<GripPlace> {
        let (low_face, high_face) = faces_along(axis);
        let mut inside: Vec<(PartId, WorldBox)> = Vec::new();
        let mut references: Vec<Reference> = Vec::new();
        for id in self.node_ids() {
            let Some(node) = self.node(id) else {
                continue;
            };
            let Item::Part(part) = node.item else {
                continue;
            };
            let Some(bounds) = self.drawn_box(id) else {
                continue;
            };
            if node.assemblies.contains(&assembly) {
                inside.push((part, bounds));
            } else {
                references.push(Reference {
                    of: FaceOf::Part(part),
                    bounds,
                });
            }
        }
        if inside.is_empty() {
            return None;
        }
        references.extend(
            self.regions()
                .iter()
                .filter(|region| !region.assemblies.contains(&assembly))
                .map(|region| Reference {
                    of: FaceOf::Region(region.id),
                    bounds: region.bounds,
                }),
        );
        let mut best: Option<Match> = None;
        for (part, bounds) in &inside {
            for reference in &references {
                let Some(ranges) = overlap(bounds, &reference.bounds, axis) else {
                    continue;
                };
                // The faces that look at each other across a gap: a part
                // looks out of its box, a region into it.
                let looks_up = |of: FaceOf| match of {
                    FaceOf::Part(_) => (high_face, reference.bounds.max().get(axis)),
                    FaceOf::Region(_) => (low_face, reference.bounds.min().get(axis)),
                };
                let looks_down = |of: FaceOf| match of {
                    FaceOf::Part(_) => (low_face, reference.bounds.min().get(axis)),
                    FaceOf::Region(_) => (high_face, reference.bounds.max().get(axis)),
                };
                let (up_face, up_at) = looks_up(reference.of);
                let (down_face, down_at) = looks_down(reference.of);
                // The assembly's part above the reference, its low face
                // looking down at it; or below, its high face looking up.
                let candidates = [
                    (up_face, up_at, low_face, bounds.min().get(axis)),
                    (down_face, down_at, high_face, bounds.max().get(axis)),
                ];
                for (from_face, from_at, to_face, to_at) in candidates {
                    let gap = if to_face == low_face {
                        to_at.checked_sub(from_at)
                    } else {
                        from_at.checked_sub(to_at)
                    };
                    if gap != Some(value) {
                        continue;
                    }
                    let rank = (
                        match reference.of {
                            FaceOf::Part(_) => 0,
                            FaceOf::Region(_) => 1,
                        },
                        std::cmp::Reverse(area(&ranges)),
                        reference.of,
                        *part,
                    );
                    if best.as_ref().is_some_and(|best| best.rank <= rank) {
                        continue;
                    }
                    best = Some(Match {
                        from: GripFace {
                            of: reference.of,
                            face: from_face,
                            at: from_at,
                        },
                        to: GripFace {
                            of: FaceOf::Part(*part),
                            face: to_face,
                            at: to_at,
                        },
                        ranges,
                        rank,
                    });
                }
            }
        }
        // No gap that wide: a part of the assembly that long.
        let best = best.or_else(|| {
            inside
                .iter()
                .find(|(_, bounds)| bounds.size(axis) == value)
                .and_then(|(part, bounds)| {
                    let ranges = overlap(bounds, bounds, axis)?;
                    Some(Match {
                        from: GripFace {
                            of: FaceOf::Part(*part),
                            face: low_face,
                            at: bounds.min().get(axis),
                        },
                        to: GripFace {
                            of: FaceOf::Part(*part),
                            face: high_face,
                            at: bounds.max().get(axis),
                        },
                        ranges,
                        rank: (0, std::cmp::Reverse(0), FaceOf::Part(*part), *part),
                    })
                })
        })?;
        Some(GripPlace::Length {
            start: point_at(axis, best.from.at, &best.ranges),
            handle: GripHandle {
                point: point_at(axis, best.to.at, &best.ranges),
                direction: along(axis, best.to.at >= best.from.at),
            },
            from: best.from,
            to: best.to,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_middle_of_two_lengths_is_exact() {
        assert_eq!(
            middle(Length::from_mm(10), Length::from_mm(20)),
            Length::from_mm(15)
        );
        assert_eq!(
            middle(Length::from_nm(-3), Length::from_nm(4)),
            Length::from_nm(0)
        );
    }

    #[test]
    fn two_boxes_overlap_across_an_axis_only_with_room() {
        let mm = |a, u, d| {
            Point::default()
                .with(Axis::Across, Length::from_mm(a))
                .with(Axis::Up, Length::from_mm(u))
                .with(Axis::Deep, Length::from_mm(d))
        };
        let a = WorldBox::new(mm(0, 0, 0), mm(10, 10, 10));
        let b = WorldBox::new(mm(5, 20, 5), mm(15, 30, 15));
        let ranges = overlap(&a, &b, Axis::Up).expect("they overlap across up");
        assert_eq!(area(&ranges), 5_000_000 * 5_000_000);
        let c = WorldBox::new(mm(10, 20, 0), mm(20, 30, 10));
        assert!(
            overlap(&a, &c, Axis::Up).is_none(),
            "only an edge is shared"
        );
    }
}
