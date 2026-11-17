//! The old hardware contract, kept filled until its readers move to the new
//! one (work pack "generalise kinds", hand-offs H1 to H3).
//!
//! `SolvedDesign::hardware` still carries an `envelope`, a `fixed` member
//! and a `HardwareData` per base, which `stave-cli`, `stave-app` and the
//! checks read. They are filled here, from what a kind now says in general
//! (its roles, members, shapes, entries and cuts). This is the **only**
//! module that reads a kind's members, roles or entries by name; it is
//! deleted with the old fields.

use std::collections::BTreeMap;

use stave_lang::Code;
use stave_lang::ast::{BaseKind, Span};
use stave_lang::placement::{Axis, Direction, Face, PartAxis};
use stave_lang::units::{ExactAngle, ExactLength};
use stave_model::defs::{FrameAxis, KindDef};
use stave_model::ids::{PartId, RegionId};
use stave_model::solved::Orientation;
use stave_model::value::{Envelope, ExactBox, Value};

use crate::hardware::{
    Frame, FrameBox, HardwareCut, MotionExact, PlacedBody, PlacedShape, RoleBound, world_index,
};
use crate::{Reported, Solved, Solver};

/// Catalog data of a placed piece of hardware, exact, as the old contract
/// has it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Data {
    Slide {
        length: ExactLength,
        clearance: ExactLength,
        travel: ExactLength,
        opening: RegionId,
    },
    ButtHinge {
        opening_angle: ExactAngle,
        on: PartId,
    },
    Hinge {
        opening_angle: ExactAngle,
        cup_diameter: ExactLength,
        cup_depth: ExactLength,
        cup_inset: ExactLength,
        on: PartId,
        /// From the piece's edge nearer `on` to the face of `on` the plate
        /// is on.
        overlay: ExactLength,
        overlay_range: Option<OverlayRangeExact>,
    },
    Pull {
        centres: Option<ExactLength>,
        diameter: Option<ExactLength>,
        screw_diameter: ExactLength,
    },
}

/// A cup hinge's catalogue overlay data.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct OverlayRangeExact {
    pub base: ExactLength,
    pub plate_height: ExactLength,
    pub adjustment: ExactLength,
}

/// One solid of the old contract: an envelope, its world box and its frame.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Member {
    pub envelope: Envelope,
    pub bounds: ExactBox,
    pub orientation: Orientation,
}

/// The old contract of one placed piece of hardware.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Legacy {
    /// The member on the piece: a hinge's cup or leaf, a slide's drawer
    /// member, a pull's knob or bar.
    pub moving: Member,
    /// The member fixed to the carcass: a hinge's plate or fixed leaf, a
    /// slide's cabinet member.
    pub fixed: Option<Member>,
    pub data: Data,
}

/// Which way an old envelope's thickness runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Thickness {
    /// Into the piece (a cup, a leaf).
    Into,
    /// Out of the mount face (a slide's members, a pull, a fixed leaf).
    Out,
    /// Across the mount face (a plate, an arm).
    Across,
}

/// What the old contract is built from.
pub(crate) struct Input<'a> {
    pub kind: &'a KindDef,
    pub kind_name: &'a str,
    pub base: BaseKind,
    pub piece_orientation: Orientation,
    pub mount_face: Face,
    pub frame: &'a Frame,
    pub roles: &'a [(String, RoleBound)],
    pub entries: &'a BTreeMap<String, Value>,
    pub bodies: &'a [PlacedBody],
    pub cuts: &'a [HardwareCut],
    pub file: stave_lang::FileId,
    pub span: Span,
}

impl Input<'_> {
    fn length(&self, name: &str) -> Option<ExactLength> {
        match self.entries.get(name) {
            Some(Value::Length(length)) => Some(*length),
            _ => None,
        }
    }

    fn angle(&self, name: &str) -> Option<ExactAngle> {
        match self.entries.get(name) {
            Some(Value::Angle(angle)) => Some(*angle),
            _ => None,
        }
    }

    fn body(&self, name: &str) -> Option<&PlacedBody> {
        self.bodies.iter().find(|placed| {
            self.kind
                .bodies
                .get(placed.body.index())
                .is_some_and(|body| body.name.as_str() == name)
        })
    }

    fn part_role(&self, name: &str) -> Option<PartId> {
        let by_name = self.roles.iter().find_map(|(role, bound)| match bound {
            RoleBound::Part(part) if role == name => Some(*part),
            RoleBound::Part(_) | RoleBound::Region(_) => None,
        });
        // A kind that calls its other part something else: its second
        // part role.
        by_name.or_else(|| {
            self.roles
                .iter()
                .skip(1)
                .find_map(|(_, bound)| match bound {
                    RoleBound::Part(part) => Some(*part),
                    RoleBound::Region(_) => None,
                })
        })
    }

    fn region_role(&self) -> Option<RegionId> {
        self.roles.iter().find_map(|(_, bound)| match bound {
            RoleBound::Region(region) => Some(*region),
            RoleBound::Part(_) => None,
        })
    }

    /// The old frame's length direction: the piece's length, or its width
    /// when the mount face is an end.
    fn length_direction(&self) -> Direction {
        let length = self.piece_orientation.direction(PartAxis::Length);
        if length.axis() == self.mount_face.axis() {
            self.piece_orientation.direction(PartAxis::Width)
        } else {
            length
        }
    }

    fn orientation(&self, thickness: Thickness) -> Solved<Orientation> {
        let length = self.length_direction();
        let across = self.frame.across;
        let out = self.mount_face.direction();
        let made = match thickness {
            Thickness::Into => Orientation::new(length, across, out.opposite()),
            Thickness::Out => Orientation::new(length, across, out),
            Thickness::Across => Orientation::new(length, out, across),
        };
        made.map_err(|_| Reported)
    }

    /// The old member for a body, from its first shape.
    fn member(&self, placed: &PlacedBody, thickness: Thickness) -> Solved<Member> {
        let Some(shape) = placed.shapes.first() else {
            return Err(Reported);
        };
        let size = |(low, high): (ExactLength, ExactLength)| high.checked_sub(low).ok_or(Reported);
        let frame_box = shape.frame;
        let envelope = match shape.cylinder {
            Some(FrameAxis::Along) => Envelope::Cylinder {
                diameter: size(frame_box.across)?,
                length: size(frame_box.along)?,
            },
            Some(FrameAxis::Across) => Envelope::Cylinder {
                diameter: size(frame_box.along)?,
                length: size(frame_box.across)?,
            },
            Some(FrameAxis::Depth) => Envelope::Cylinder {
                diameter: size(frame_box.along)?,
                length: size(frame_box.depth)?,
            },
            None => {
                let (width, thick) = match thickness {
                    Thickness::Into | Thickness::Out => {
                        (size(frame_box.across)?, size(frame_box.depth)?)
                    }
                    Thickness::Across => (size(frame_box.depth)?, size(frame_box.across)?),
                };
                Envelope::Box {
                    length: size(frame_box.along)?,
                    width,
                    thickness: thick,
                }
            }
        };
        Ok(Member {
            envelope,
            bounds: shape.world,
            orientation: self.orientation(thickness)?,
        })
    }

    /// The old member for a box in the kind's frame whose thickness runs
    /// across (a cup hinge's arm and knuckle).
    pub(crate) fn boxed(&self, bounds: ExactBox, frame_box: &FrameBox) -> Solved<Member> {
        self.member(
            &PlacedBody {
                body: stave_model::defs::BodyId::new(0).ok_or(Reported)?,
                shapes: vec![PlacedShape {
                    frame: *frame_box,
                    world: bounds,
                    cylinder: None,
                    outline: None,
                }],
            },
            Thickness::Across,
        )
    }
}

impl Solver<'_> {
    /// The old contract for a placed piece of hardware.
    pub(crate) fn legacy(&mut self, input: &Input<'_>) -> Solved<Legacy> {
        let missing = |solver: &mut Self, name: &str| {
            solver.error(
                Code::MissingField,
                format!("kind '{}' is missing '{name}'", input.kind_name),
                input.file,
                input.span,
                format!("give the kind {name}: ..."),
            )
        };
        match input.base {
            BaseKind::Slide => {
                let (Some(drawer), Some(cabinet)) =
                    (input.body("drawer_member"), input.body("cabinet_member"))
                else {
                    return Err(
                        self.unsupported(input, "a slide's drawer_member and cabinet_member")
                    );
                };
                let mut length = || -> Solved<[ExactLength; 3]> {
                    let mut values = [ExactLength::ZERO; 3];
                    for (slot, name) in values.iter_mut().zip(["length", "clearance", "travel"]) {
                        *slot = match input.length(name) {
                            Some(value) => value,
                            None => return Err(missing(self, name)),
                        };
                    }
                    Ok(values)
                };
                let [length, clearance, travel] = length()?;
                let Some(opening) = input.region_role() else {
                    return Err(self.unsupported(input, "a slide's region role"));
                };
                Ok(Legacy {
                    moving: input.member(drawer, Thickness::Out)?,
                    fixed: Some(input.member(cabinet, Thickness::Out)?),
                    data: Data::Slide {
                        length,
                        clearance,
                        travel,
                        opening,
                    },
                })
            }
            BaseKind::Hinge => {
                let Some(on) = input.part_role("on") else {
                    return Err(self.unsupported(input, "a hinge's second part"));
                };
                let Some(opening_angle) = input.angle("opening_angle") else {
                    return Err(missing(self, "opening_angle"));
                };
                if let (Some(leaf), Some(fixed)) = (input.body("leaf"), input.body("fixed_leaf")) {
                    return Ok(Legacy {
                        moving: input.member(leaf, Thickness::Into)?,
                        fixed: Some(input.member(fixed, Thickness::Out)?),
                        data: Data::ButtHinge { opening_angle, on },
                    });
                }
                let (Some(cup), Some(plate)) = (input.body("cup"), input.body("plate")) else {
                    return Err(
                        self.unsupported(input, "a hinge's cup and plate, or leaf and fixed_leaf")
                    );
                };
                let mut cup_entries = [ExactLength::ZERO; 3];
                for (slot, name) in
                    cup_entries
                        .iter_mut()
                        .zip(["cup_diameter", "cup_depth", "cup_inset"])
                {
                    *slot = match input.length(name) {
                        Some(value) => value,
                        None => return Err(missing(self, name)),
                    };
                }
                let [cup_diameter, cup_depth, cup_inset] = cup_entries;
                // The overlay: from the piece's edge nearer `on` to the face
                // of `on` that looks toward the middle of the piece.
                let on_box = self.drawn_box(on)?;
                let axis = input.frame.across.axis();
                let edge = input
                    .frame
                    .across_edge
                    .unwrap_or_else(|| input.frame.origin_on(axis));
                let positive = input.frame.across.is_positive();
                let inside = if positive {
                    on_box.max(axis)
                } else {
                    on_box.min(axis)
                };
                let overlay = if positive {
                    inside.checked_sub(edge)
                } else {
                    edge.checked_sub(inside)
                }
                .ok_or(Reported)?;
                let overlay_range = input.length("base_overlay").map(|base| OverlayRangeExact {
                    base,
                    plate_height: input.length("plate_height").unwrap_or(ExactLength::ZERO),
                    adjustment: input
                        .length("overlay_adjustment")
                        .unwrap_or(ExactLength::ZERO),
                });
                Ok(Legacy {
                    moving: input.member(cup, Thickness::Into)?,
                    fixed: Some(input.member(plate, Thickness::Across)?),
                    data: Data::Hinge {
                        opening_angle,
                        cup_diameter,
                        cup_depth,
                        cup_inset,
                        on,
                        overlay,
                        overlay_range,
                    },
                })
            }
            BaseKind::Pull => {
                let Some(held) = input.body("knob").or_else(|| input.body("bar")) else {
                    return Err(self.unsupported(input, "a pull's knob or bar"));
                };
                let drilled = input.cuts.iter().find_map(|cut| match cut.kind {
                    stave_model::solved::CutKind::Drill { diameter, .. } => Some(diameter),
                    _ => None,
                });
                let screw_diameter = input
                    .length("screw_diameter")
                    .or_else(|| drilled.map(|d| d.to_exact()))
                    .unwrap_or(ExactLength::ZERO);
                Ok(Legacy {
                    moving: input.member(held, Thickness::Out)?,
                    fixed: None,
                    data: Data::Pull {
                        centres: input.length("centres"),
                        diameter: input.length("diameter"),
                        screw_diameter,
                    },
                })
            }
            BaseKind::Joint | BaseKind::Fastener | BaseKind::Tool => Err(Reported),
        }
    }

    /// The motion of a piece of hardware whose kind has no mechanism, as
    /// the old contract reads it from its base: a slide travels toward the
    /// front by its `travel`; a hinge turns about the `along` line through
    /// the edge its mount measures from, on its mount face, by its
    /// `opening_angle`. A pull doesn't move.
    pub(crate) fn legacy_motion(
        &mut self,
        input: &Input<'_>,
        piece_box: &ExactBox,
    ) -> Solved<Option<MotionExact>> {
        match input.base {
            BaseKind::Slide => Ok(input.length("travel").map(|travel| MotionExact::Travel {
                toward: Direction::Front,
                travel,
            })),
            BaseKind::Hinge => {
                let Some(angle) = input.angle("opening_angle") else {
                    return Ok(None);
                };
                let along = input.frame.along.axis();
                let across = input.frame.across.axis();
                let face_axis = input.mount_face.axis();
                let mut through = [ExactLength::ZERO; 3];
                for axis in Axis::ALL {
                    if let Some(slot) = through.get_mut(world_index(axis)) {
                        *slot = if axis == face_axis {
                            input.frame.origin_on(face_axis)
                        } else if axis == across {
                            input
                                .frame
                                .across_edge
                                .unwrap_or_else(|| input.frame.origin_on(across))
                        } else {
                            piece_box.min(axis)
                        };
                    }
                }
                Ok(Some(MotionExact::Turn {
                    axis: along,
                    through,
                    free_side: input.frame.across,
                    swing_toward: input.mount_face.opposite().direction(),
                    angle,
                }))
            }
            BaseKind::Pull | BaseKind::Joint | BaseKind::Fastener | BaseKind::Tool => Ok(None),
        }
    }

    fn unsupported(&mut self, input: &Input<'_>, what: &str) -> Reported {
        self.error(
            Code::NotSupported,
            format!(
                "kind '{}' without {what} is not supported yet",
                input.kind_name
            ),
            input.file,
            input.span,
            "give the kind the members the standard kinds in std:hardware have",
        )
    }

    /// The old contract's references to parts and regions, re-bound after
    /// the roles of a mirrored piece of hardware are.
    pub(crate) fn legacy_rebound(
        data: &Data,
        old: &[(String, RoleBound)],
        new: &[(String, RoleBound)],
    ) -> Data {
        let part = |part: PartId| {
            old.iter()
                .zip(new)
                .find_map(|((_, before), (_, after))| match (before, after) {
                    (RoleBound::Part(b), RoleBound::Part(a)) if *b == part => Some(*a),
                    _ => None,
                })
                .unwrap_or(part)
        };
        let region = |region: RegionId| {
            old.iter()
                .zip(new)
                .find_map(|((_, before), (_, after))| match (before, after) {
                    (RoleBound::Region(b), RoleBound::Region(a)) if *b == region => Some(*a),
                    _ => None,
                })
                .unwrap_or(region)
        };
        match data.clone() {
            Data::Slide {
                length,
                clearance,
                travel,
                opening,
            } => Data::Slide {
                length,
                clearance,
                travel,
                opening: region(opening),
            },
            Data::ButtHinge { opening_angle, on } => Data::ButtHinge {
                opening_angle,
                on: part(on),
            },
            Data::Hinge {
                opening_angle,
                cup_diameter,
                cup_depth,
                cup_inset,
                on,
                overlay,
                overlay_range,
            } => Data::Hinge {
                opening_angle,
                cup_diameter,
                cup_depth,
                cup_inset,
                on: part(on),
                overlay,
                overlay_range,
            },
            pull @ Data::Pull { .. } => pull,
        }
    }
}
