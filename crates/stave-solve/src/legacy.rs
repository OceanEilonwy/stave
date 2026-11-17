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
use stave_lang::units::{ExactAngle, ExactLength, Length};
use stave_model::defs::{BodyId, FrameAxis, KindDef, RExpr, RangeDef, ShapeDef};
use stave_model::ids::{PartId, RegionId};
use stave_model::solved::{
    Link, LinkBar, Linkage, LinkageError, LinkageFrame, Motion, Orientation, Pivot,
    PlanePoint as LinkagePoint, Point,
};
use stave_model::value::{Envelope, ExactBox, Value};

use stave_model::mechanism::DriveValue;

use crate::eval::Env;
use crate::hardware::{
    Frame, FrameBox, HardwareCut, PlacedBody, PlacedShape, RoleBound, world_index,
};
use crate::joints::Binding;
use crate::mechanism::{Drive, Entry, HardwareMechanism, MemberWith, PlanePoint};
use crate::{Reported, Solved, Solver};

/// The motion a piece of hardware gives its assembly, as the old contract
/// has it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum LegacyMotion {
    Turn {
        axis: Axis,
        through: [ExactLength; 3],
        free_side: Direction,
        swing_toward: Direction,
        angle: ExactAngle,
    },
    /// A four-bar linkage, checked and in whole nanometres.
    Linkage(Linkage),
    Travel {
        toward: Direction,
        travel: ExactLength,
    },
}

impl LegacyMotion {
    /// The motion of the mirror image, across the world plane at `plane` on
    /// `axis`.
    pub(crate) fn reflected(self, axis: Axis, plane: ExactLength) -> Option<LegacyMotion> {
        let flip = |direction: Direction| {
            if direction.axis() == axis {
                direction.opposite()
            } else {
                direction
            }
        };
        let twice = plane.checked_add(plane)?;
        Some(match self {
            LegacyMotion::Turn {
                axis: pivot_axis,
                through,
                free_side,
                swing_toward,
                angle,
            } => {
                let mut through = through;
                if let Some(slot) = through.get_mut(world_index(axis)) {
                    *slot = twice.checked_sub(*slot)?;
                }
                LegacyMotion::Turn {
                    axis: pivot_axis,
                    through,
                    free_side: flip(free_side),
                    swing_toward: flip(swing_toward),
                    angle,
                }
            }
            LegacyMotion::Linkage(linkage) => {
                // The linkage's points in its own frame stay; its frame is
                // reflected, the origin exactly, then to the nearest
                // nanometre.
                let at = linkage.frame().origin().get(axis).to_exact();
                let origin = twice.checked_sub(at)?.to_nearest_length()?;
                LegacyMotion::Linkage(linkage.in_frame(linkage.frame().reflected(axis, origin)))
            }
            LegacyMotion::Travel { toward, travel } => LegacyMotion::Travel {
                toward: flip(toward),
                travel,
            },
        })
    }

    /// The four-bar linkage, for a motion that is one.
    pub(crate) fn linkage(&self) -> Option<Linkage> {
        match self {
            LegacyMotion::Linkage(linkage) => Some(*linkage),
            LegacyMotion::Turn { .. } | LegacyMotion::Travel { .. } => None,
        }
    }
}

/// A four-bar linkage's arm and links, for the old contract.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CupParts {
    /// The two links' members, in the linkage's order.
    pub link_bodies: [BodyId; 2],
    pub arm: Member,
    pub knuckle: Member,
    pub links: [LinkBar; 2],
    pub pin_diameter: Length,
    pub pin_length: Length,
}

/// One link of a four-bar: pinned once to a held member and once to the
/// driven member.
struct HungLink {
    link: BodyId,
    held: BodyId,
    held_pin: PlanePoint,
    driven_pin: PlanePoint,
}

/// A driven member hung on two links from held members, turned from 0deg:
/// exactly four pivots, nothing else.
struct FourBar {
    links: [HungLink; 2],
    opening: ExactAngle,
}

/// The four-bar shape of a mechanism, when it has it.
fn four_bar(mechanism: &HardwareMechanism) -> Option<FourBar> {
    let Some(Drive::Turn {
        body: driven,
        from,
        to,
    }) = mechanism.drive
    else {
        return None;
    };
    if from != ExactAngle::ZERO || mechanism.entries.len() != 4 {
        return None;
    }
    let with = |body: BodyId| {
        mechanism
            .members
            .iter()
            .find(|member| member.body == body)
            .map(|member| member.with)
    };
    let placed: Vec<BodyId> = mechanism
        .members
        .iter()
        .filter(|member| member.with == MemberWith::Placed)
        .map(|member| member.body)
        .collect();
    let [first, second] = placed.as_slice() else {
        return None;
    };
    let hung = |link: BodyId| -> Option<HungLink> {
        let mut held: Option<(BodyId, PlanePoint)> = None;
        let mut on_driven: Option<PlanePoint> = None;
        for entry in &mechanism.entries {
            let Entry::Pivot { bodies, at } = entry else {
                return None;
            };
            let other = match bodies {
                [a, b] if *a == link => *b,
                [a, b] if *b == link => *a,
                _ => continue,
            };
            if other == driven && on_driven.is_none() {
                on_driven = Some(*at);
            } else if matches!(with(other), Some(MemberWith::Role(_))) && held.is_none() {
                held = Some((other, *at));
            } else {
                return None;
            }
        }
        let ((held, held_pin), driven_pin) = (held?, on_driven?);
        Some(HungLink {
            link,
            held,
            held_pin,
            driven_pin,
        })
    };
    Some(FourBar {
        links: [hung(*first)?, hung(*second)?],
        opening: to,
    })
}

/// The old contract's motion of an assembly: its first driving piece of
/// hardware's (the rule for several items makes every driver move alike),
/// as far as the assembly's mechanism reaches (a follower's stop may end
/// it sooner).
pub(crate) fn legacy_assembly_motion(motion: &LegacyMotion, extent: DriveValue) -> Solved<Motion> {
    let near = |value: ExactLength| value.to_nearest_length().ok_or(Reported);
    Ok(match motion {
        LegacyMotion::Turn {
            axis,
            through,
            free_side,
            swing_toward,
            angle,
        } => {
            let [across, up, deep] = *through;
            Motion::Turn {
                pivot: Pivot {
                    axis: *axis,
                    through: Point {
                        across: near(across)?,
                        up: near(up)?,
                        deep: near(deep)?,
                    },
                    free_side: *free_side,
                    swing_toward: *swing_toward,
                },
                opening_angle: match extent {
                    DriveValue::Turn(reach) => reach,
                    DriveValue::Travel(_) => angle.to_nearest_angle().ok_or(Reported)?,
                },
            }
        }
        LegacyMotion::Linkage(linkage) => Motion::Linkage { linkage: *linkage },
        LegacyMotion::Travel { toward, travel } => Motion::Travel {
            toward: *toward,
            travel: match extent {
                DriveValue::Travel(reach) => reach,
                DriveValue::Turn(_) => near(*travel)?,
            },
        },
    })
}

fn abs(value: ExactLength) -> Solved<ExactLength> {
    if value < ExactLength::ZERO {
        value.checked_neg().ok_or(Reported)
    } else {
        Ok(value)
    }
}

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

    /// The old contract's motion of a piece of hardware, read from the
    /// shape of its kind's mechanism (never from its kind or base): one
    /// pivot between the driven member and a held one is a turn about that
    /// pin; one rail is a travel along it; a driven member hung on two
    /// links from held members is a four-bar linkage, checked exactly, with
    /// its arm and links for the old contract. Any other mechanism, and a
    /// kind with none, gives the old contract no motion.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn legacy_motion(
        &mut self,
        input: &Input<'_>,
        piece_box: &ExactBox,
        mechanism: Option<&HardwareMechanism>,
        binding: &Binding,
        name: &str,
        env: &Env<'_>,
        span: Span,
    ) -> Solved<(Option<LegacyMotion>, Option<CupParts>)> {
        let Some(mechanism) = mechanism else {
            return Ok((None, None));
        };
        let plane = &mechanism.plane;
        match (mechanism.entries.as_slice(), mechanism.drive) {
            (
                [Entry::Pivot { at, .. }],
                Some(Drive::Turn {
                    to: angle, from, ..
                }),
            ) if from == ExactAngle::ZERO => {
                let mut through = plane.origin;
                for (direction, amount) in [(plane.across, at[0]), (plane.depth, at[1])] {
                    if let Some(slot) = through.get_mut(world_index(direction.axis())) {
                        let signed = if direction.is_positive() {
                            amount
                        } else {
                            amount.checked_neg().ok_or(Reported)?
                        };
                        *slot = slot.checked_add(signed).ok_or(Reported)?;
                    }
                }
                if let Some(slot) = through.get_mut(world_index(plane.axis)) {
                    *slot = piece_box.min(plane.axis);
                }
                Ok((
                    Some(LegacyMotion::Turn {
                        axis: plane.axis,
                        through,
                        free_side: plane.across,
                        swing_toward: plane.depth,
                        angle,
                    }),
                    None,
                ))
            }
            ([Entry::Rail { direction, .. }], Some(Drive::Travel { from, to, .. })) => {
                let along = if direction[0] != ExactLength::ZERO {
                    plane.across
                } else {
                    plane.depth
                };
                let travel = to.checked_sub(from).ok_or(Reported)?;
                let (toward, travel) = if travel < ExactLength::ZERO {
                    (along.opposite(), travel.checked_neg().ok_or(Reported)?)
                } else {
                    (along, travel)
                };
                Ok((Some(LegacyMotion::Travel { toward, travel }), None))
            }
            _ => match four_bar(mechanism) {
                Some(four_bar) => {
                    let (linkage, cup) =
                        self.cup_linkage(input, piece_box, &four_bar, binding, name, env, span)?;
                    Ok((Some(LegacyMotion::Linkage(linkage)), cup))
                }
                None => Ok((None, None)),
            },
        }
    }

    /// A cup hinge's linkage for the old contract: the four pins placed
    /// relative to the door's edge and its front face, checked exactly
    /// free of dead points over the drive's range; and its held member's
    /// two boxes as the arm and its knuckle, and its links' bars.
    #[allow(clippy::too_many_arguments)]
    fn cup_linkage(
        &mut self,
        input: &Input<'_>,
        piece_box: &ExactBox,
        four_bar: &FourBar,
        binding: &Binding,
        name: &str,
        env: &Env<'_>,
        span: Span,
    ) -> Solved<(Linkage, Option<CupParts>)> {
        let kind = input.kind;
        let frame = input.frame;
        let face_axis = input.mount_face.axis();
        let along = frame.along.axis();
        let across = frame.across.axis();
        let opposite = input.mount_face.opposite();
        let opposite_position = if opposite.is_positive() {
            piece_box.max(face_axis)
        } else {
            piece_box.min(face_axis)
        };
        let face_position = frame.origin_on(face_axis);
        let edge = frame.across_edge.unwrap_or_else(|| frame.origin_on(across));
        let mut through = [ExactLength::ZERO; 3];
        for axis in Axis::ALL {
            if let Some(slot) = through.get_mut(world_index(axis)) {
                *slot = if axis == face_axis {
                    opposite_position
                } else if axis == across {
                    edge
                } else {
                    frame.origin_on(along)
                };
            }
        }
        let cup_x = abs(frame.origin_on(across).checked_sub(edge).ok_or(Reported)?)?;
        let thickness = abs(opposite_position
            .checked_sub(face_position)
            .ok_or(Reported)?)?;
        let near = |value: ExactLength| value.to_nearest_length().ok_or(Reported);
        let coordinate = |axis: Axis| -> Solved<Length> {
            near(*through.get(world_index(axis)).ok_or(Reported)?)
        };
        let origin = Point {
            across: coordinate(Axis::Across)?,
            up: coordinate(Axis::Up)?,
            deep: coordinate(Axis::Deep)?,
        };
        let world = LinkageFrame::new(along, origin, frame.across, opposite.direction())
            .map_err(|_| Reported)?;
        // From the kind's frame (across from the cup's centre, depth into
        // the door from its back) to the linkage's plane (from the door's
        // edge, and from its front face).
        let place = |[across, depth]: PlanePoint| -> Solved<LinkagePoint> {
            let x = cup_x.checked_add(across).ok_or(Reported)?;
            let y = depth.checked_sub(thickness).ok_or(Reported)?;
            Ok(LinkagePoint::new(near(x)?, near(y)?))
        };
        let link = |hung: &HungLink| -> Solved<Link> {
            Ok(Link {
                ground: place(hung.held_pin)?,
                coupler: place(hung.driven_pin)?,
            })
        };
        let [first, second] = &four_bar.links;
        let opening = four_bar.opening.to_nearest_angle().ok_or(Reported)?;
        let linkage = match Linkage::new(world, [link(first)?, link(second)?], opening) {
            Ok(linkage) => linkage,
            Err(LinkageError::DeadPoint(_) | LinkageError::Angle) => {
                return Err(self.error(
                    Code::Motion,
                    format!("{name}: a cup hinge's links lock before it opens {opening}"),
                    env.file,
                    span,
                    "give the hinge a smaller opening_angle: its links can't turn this far",
                ));
            }
            Err(
                LinkageError::Frame
                | LinkageError::NoLength(_)
                | LinkageError::SharedPin
                | LinkageError::Overflow,
            ) => return Err(Reported),
        };
        // The old contract's arm and links, when the kind's shapes are the
        // ones it can describe; otherwise only the new members show them.
        let (Some(first_bar), Some(second_bar)) = (
            self.link_bar(binding, kind, first.link)?,
            self.link_bar(binding, kind, second.link)?,
        ) else {
            return Ok((linkage, None));
        };
        let link_bars = [first_bar, second_bar];
        let held = kind.bodies.get(first.held.index()).ok_or(Reported)?;
        let boxes: Vec<&ShapeDef> = held
            .shapes
            .iter()
            .map(|shape| &shape.shape)
            .filter(|shape| matches!(shape, ShapeDef::Box { .. }))
            .collect();
        let ([arm, knuckle], 2) = (boxes.as_slice(), held.shapes.len()) else {
            return Ok((linkage, None));
        };
        let arm = (*arm).clone();
        let knuckle = (*knuckle).clone();
        let (arm, knuckle) = self.in_kind(binding, kind, |solver, kind_env| {
            Ok((
                solver.frame_box(&arm, kind_env)?,
                solver.frame_box(&knuckle, kind_env)?,
            ))
        })?;
        let pin_diameter = match kind.mechanism.as_ref().and_then(|m| m.pin_diameter.clone()) {
            Some(diameter) => self.in_kind(binding, kind, |solver, kind_env| {
                solver.kind_length(&diameter, kind_env)
            })?,
            None => ExactLength::ZERO,
        };
        let widest = link_bars
            .iter()
            .map(|link| link.width)
            .max()
            .unwrap_or(Length::ZERO);
        Ok((
            linkage,
            Some(CupParts {
                link_bodies: [first.link, second.link],
                arm: input.boxed(frame.world_box(&arm)?, &arm)?,
                knuckle: input.boxed(frame.world_box(&knuckle)?, &knuckle)?,
                links: link_bars,
                pin_diameter: near(pin_diameter)?,
                pin_length: widest,
            }),
        ))
    }

    /// A link's bar, from its shapes: one bar centred along the pins, or
    /// two mirror-image bars (a strap whose sides pass either side of the
    /// other link); `None` for any other link, which the old contract
    /// can't describe.
    fn link_bar(
        &mut self,
        binding: &Binding,
        kind: &KindDef,
        link: BodyId,
    ) -> Solved<Option<LinkBar>> {
        let body = kind.bodies.get(link.index()).ok_or(Reported)?;
        let bars: Vec<(RExpr, RangeDef)> = body
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                ShapeDef::Bar {
                    thickness, along, ..
                } => Some((thickness.clone(), along.clone())),
                ShapeDef::Box { .. } | ShapeDef::Cylinder { .. } | ShapeDef::Prism { .. } => None,
            })
            .collect();
        let shapes = body.shapes.len();
        let evaluated = self.in_kind(binding, kind, |solver, kind_env| {
            bars.iter()
                .map(|(thickness, along)| {
                    Ok((
                        solver.kind_length(thickness, kind_env)?,
                        solver.frame_range(along, kind_env)?,
                    ))
                })
                .collect::<Solved<Vec<_>>>()
        })?;
        let near = |value: ExactLength| value.to_nearest_length().ok_or(Reported);
        let bar = match (evaluated.as_slice(), shapes) {
            ([(thickness, (low, high))], 1) if low.checked_neg() == Some(*high) => {
                Some((*thickness, high.checked_sub(*low), Some(ExactLength::ZERO)))
            }
            ([(thickness, (low, high)), (other, (far_low, far_high))], 2)
                if thickness == other
                    && far_low.checked_neg() == Some(*high)
                    && far_high.checked_neg() == Some(*low)
                    && *low >= ExactLength::ZERO =>
            {
                Some((*thickness, high.checked_add(*high), low.checked_add(*low)))
            }
            _ => None,
        };
        match bar {
            Some((thickness, Some(width), Some(slot))) => Ok(Some(LinkBar {
                width: near(width)?,
                thickness: near(thickness)?,
                slot: near(slot)?,
            })),
            Some(_) => Err(Reported),
            None => Ok(None),
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
