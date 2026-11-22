//! Hardware: mounting a slide, hinge or pull on a face of its piece where
//! its kind's `mount { }` and the design's `place { }` say, placing its
//! members' shapes, the cuts its kind makes and the motion it gives its
//! assembly. Nothing here knows a base or a member by name.

use std::collections::BTreeMap;

use stave_lang::ast::{BaseKind, Span};
use stave_lang::placement::{Axis, Direction, Face, PartAxis, PerAxis};
use stave_lang::units::{Angle, Length};
use stave_lang::units::{ExactAngle, ExactLength, Rational};
use stave_lang::{Code, Diagnostic, Label};
use stave_model::defs::{
    AxisRule, BodyId, Carrier, CutName, DefId, FaceRule, FrameAxis, FromTerm, KindDef, KindPoint,
    MountSelector, MountTerm, RExpr, RangeDef, RoleIndex, ShapeDef,
};
use stave_model::ids::{HardwareId, KindId, PartId, RegionId};
use stave_model::instance::ItemSlot;
use stave_model::solved::CutKind;
use stave_model::solved::Rules;
use stave_model::value::{AnchorOwner, AnchorValue, ExactBox, Value};
use stave_schema::kinds::BaseField;

use crate::eval::{Env, KindEnv, midpoint};
use crate::joints::{Binding, CutAt};
use crate::mechanism::{ExactPoint, HardwareMechanism};
use crate::place::Siblings;
use crate::rules::{AdjustSetting, Call, Offset};
use crate::{Failure, Fault, Memo, Solved, Solver};

/// What a role of a placed piece of hardware is bound to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RoleBound {
    Part(PartId),
    Region(RegionId),
}

/// The kind's frame placed in the world (grammar, "The kind's frame"):
/// its origin and the world direction of each of its axes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Frame {
    /// The origin, in world coordinates.
    pub origin: PerAxis<ExactLength>,
    pub along: Direction,
    pub across: Direction,
    /// Into the piece.
    pub depth: Direction,
    /// The edge `across` is measured from, when the kind's mount measures
    /// it from one: its world coordinate on the `across` axis.
    pub across_edge: Option<ExactLength>,
}

impl Frame {
    /// The world direction of a frame axis.
    pub fn direction(&self, axis: FrameAxis) -> Direction {
        match axis {
            FrameAxis::Along => self.along,
            FrameAxis::Across => self.across,
            FrameAxis::Depth => self.depth,
        }
    }

    /// The origin's coordinate along a world axis.
    pub fn origin_on(&self, axis: Axis) -> ExactLength {
        self.origin[axis]
    }

    /// A frame range on a frame axis, as a world extent.
    fn world_extent(
        &self,
        axis: FrameAxis,
        (low, high): (ExactLength, ExactLength),
    ) -> Solved<(ExactLength, ExactLength)> {
        let direction = self.direction(axis);
        let origin = self.origin_on(direction.axis());
        Ok(if direction.is_positive() {
            (
                origin.checked_add(low).ok_or(Fault::Overflow)?,
                origin.checked_add(high).ok_or(Fault::Overflow)?,
            )
        } else {
            (
                origin.checked_sub(high).ok_or(Fault::Overflow)?,
                origin.checked_sub(low).ok_or(Fault::Overflow)?,
            )
        })
    }

    /// The frame coordinate of a world plane across a frame axis.
    fn coordinate(&self, axis: FrameAxis, world: ExactLength) -> Solved<ExactLength> {
        let direction = self.direction(axis);
        let offset = world
            .checked_sub(self.origin_on(direction.axis()))
            .ok_or(Fault::Overflow)?;
        if direction.is_positive() {
            Ok(offset)
        } else {
            offset.checked_neg().ok_or(Failure::Fault(Fault::Overflow))
        }
    }

    /// A point in the frame, in the world.
    pub fn world_point(
        &self,
        along: ExactLength,
        across: ExactLength,
        depth: ExactLength,
    ) -> Solved<PerAxis<ExactLength>> {
        let mut out = ORIGIN;
        for (axis, value) in [
            (FrameAxis::Along, along),
            (FrameAxis::Across, across),
            (FrameAxis::Depth, depth),
        ] {
            let direction = self.direction(axis);
            let origin = self.origin_on(direction.axis());
            let world = if direction.is_positive() {
                origin.checked_add(value)
            } else {
                origin.checked_sub(value)
            }
            .ok_or(Fault::Overflow)?;
            out[direction.axis()] = world;
        }
        Ok(out)
    }

    /// A box in the frame, in the world.
    pub fn world_box(&self, frame_box: &FrameBox) -> Solved<ExactBox> {
        let mut extents = PerAxis::from_fn(|_| (ExactLength::ZERO, ExactLength::ZERO));
        for (axis, range) in [
            (FrameAxis::Along, frame_box.along),
            (FrameAxis::Across, frame_box.across),
            (FrameAxis::Depth, frame_box.depth),
        ] {
            let extent = self.world_extent(axis, range)?;
            extents[self.direction(axis).axis()] = extent;
        }
        Ok(ExactBox::new(extents.across, extents.up, extents.deep))
    }
}

/// A box in the kind's frame: each extent low end first.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct FrameBox {
    pub along: (ExactLength, ExactLength),
    pub across: (ExactLength, ExactLength),
    pub depth: (ExactLength, ExactLength),
}

impl FrameBox {
    fn range(&self, axis: FrameAxis) -> (ExactLength, ExactLength) {
        match axis {
            FrameAxis::Along => self.along,
            FrameAxis::Across => self.across,
            FrameAxis::Depth => self.depth,
        }
    }

    fn with(mut self, axis: FrameAxis, range: (ExactLength, ExactLength)) -> FrameBox {
        match axis {
            FrameAxis::Along => self.along = range,
            FrameAxis::Across => self.across = range,
            FrameAxis::Depth => self.depth = range,
        }
        self
    }

    /// The smallest box holding both.
    fn union(&self, other: &FrameBox) -> FrameBox {
        let join = |(a, b): (ExactLength, ExactLength), (c, d): (ExactLength, ExactLength)| {
            (a.min(c), b.max(d))
        };
        FrameBox {
            along: join(self.along, other.along),
            across: join(self.across, other.across),
            depth: join(self.depth, other.depth),
        }
    }
}

/// One shape of a placed member, at rest.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PlacedShape {
    /// Its box in the kind's frame, after the member's own mount.
    pub frame: FrameBox,
    /// Its box in the world.
    pub world: ExactBox,
    /// The frame axis a cylinder's axis runs along, and its slant.
    pub cylinder: Option<Round>,
    /// A prism's or a bar's outline on the across/depth plane, after the
    /// member's own mount.
    pub outline: Option<Outline>,
}

/// The outline of a prism or a bar on the kind's across/depth plane, each
/// point `[across, depth]`; it is extruded along `along`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Outline {
    /// A convex outline, its corners in order around it.
    Prism(Vec<[ExactLength; 2]>),
    /// A bar between two centres, `thickness` wide.
    Bar {
        from: [ExactLength; 2],
        to: [ExactLength; 2],
        thickness: ExactLength,
    },
}

impl Outline {
    /// Moved by `across` and `depth` in the frame.
    fn moved(&self, across: ExactLength, depth: ExactLength) -> Solved<Outline> {
        let shift = |[a, d]: [ExactLength; 2]| -> Solved<[ExactLength; 2]> {
            Ok([
                a.checked_add(across).ok_or(Fault::Overflow)?,
                d.checked_add(depth).ok_or(Fault::Overflow)?,
            ])
        };
        Ok(match self {
            Outline::Prism(points) => Outline::Prism(
                points
                    .iter()
                    .map(|point| shift(*point))
                    .collect::<Solved<Vec<_>>>()?,
            ),
            Outline::Bar {
                from,
                to,
                thickness,
            } => Outline::Bar {
                from: shift(*from)?,
                to: shift(*to)?,
                thickness: *thickness,
            },
        })
    }
}

/// A member's shape at rest in the world, exact.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum WorldShape {
    Box(ExactBox),
    /// A cylinder along `axis`, filling `bounds` across it.
    Cylinder {
        axis: Axis,
        bounds: ExactBox,
    },
    /// A cylinder turned from `axis` toward `toward` by `angle` (0 to
    /// 90deg), `length` long, centred in `bounds`.
    Slanted {
        axis: Axis,
        toward: Direction,
        angle: Angle,
        diameter: ExactLength,
        length: ExactLength,
        bounds: ExactBox,
    },
    /// A convex outline across `axis`, extruded over `bounds` along it;
    /// each point is a world point (its coordinate on `axis` carries no
    /// meaning).
    Prism {
        axis: Axis,
        points: Vec<PerAxis<ExactLength>>,
        bounds: ExactBox,
    },
    /// A bar across `axis` between two world points, over `bounds` along
    /// it.
    Bar {
        axis: Axis,
        from: PerAxis<ExactLength>,
        to: PerAxis<ExactLength>,
        thickness: ExactLength,
        bounds: ExactBox,
    },
}

/// What moves a placed member.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MemberCarrier {
    Part(PartId),
    Region(RegionId),
    /// Placed by its kind's mechanism: this member (with no `with:`) of it.
    Link(BodyId),
}

/// A member with a body, at rest in the world.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PlacedMember {
    pub body: BodyId,
    pub name: String,
    pub carrier: MemberCarrier,
    pub shapes: Vec<WorldShape>,
}

/// A member with a body, placed at rest.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PlacedBody {
    pub body: BodyId,
    /// One per shape, in the kind's order.
    pub shapes: Vec<PlacedShape>,
}

/// A cut a piece of hardware makes in one part: one shape of one member.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct HardwareCut {
    pub part: PartId,
    pub body: BodyId,
    pub name: Option<CutName>,
    /// The world face of the part it is cut from.
    pub face: Face,
    /// The shape's world box; the footprint is where it meets the part.
    pub shape: ExactBox,
    /// The shape's slant, when it is a slanted cylinder.
    pub slant: Option<WorldSlant>,
    pub kind: CutKind,
    /// The least wall its operation asks for (`wall:`).
    pub wall: Option<Length>,
}

/// How a slanted cylinder leans in the world: turned from `axis` toward
/// `toward` by `angle` (grammar, "Slanted members").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WorldSlant {
    pub axis: Axis,
    /// Read toward the positive end of `axis`.
    pub toward: Direction,
    pub angle: Angle,
}

impl WorldSlant {
    /// The slant of a shape, when it is a slanted cylinder.
    pub(crate) fn of(shape: &WorldShape) -> Option<WorldSlant> {
        match shape {
            WorldShape::Slanted {
                axis,
                toward,
                angle,
                ..
            } => Some(WorldSlant {
                axis: *axis,
                toward: *toward,
                angle: *angle,
            }),
            WorldShape::Box(_)
            | WorldShape::Cylinder { .. }
            | WorldShape::Prism { .. }
            | WorldShape::Bar { .. } => None,
        }
    }

    /// The same slant reflected through a plane across `mirror`.
    pub(crate) fn reflected(self, mirror: Axis) -> WorldSlant {
        WorldSlant {
            toward: slant_reflected(self.axis, self.toward, mirror),
            ..self
        }
    }
}

/// Which way a slanted cylinder turned from `along` toward `toward` leans
/// once reflected through a plane across `mirror`: the other way, unless
/// the plane is across the third axis, square to both.
pub(crate) fn slant_reflected(along: Axis, toward: Direction, mirror: Axis) -> Direction {
    if mirror == along || mirror == toward.axis() {
        toward.opposite()
    } else {
        toward
    }
}

/// The cuts one shape of a member makes in a part, for
/// [`Solver::check_slanted_cut`].
pub(crate) struct SlantedCut<'a> {
    pub owner: &'a str,
    pub member: &'a str,
    pub part: PartId,
    pub face: Face,
    pub slant: Option<WorldSlant>,
    pub kinds: &'a [CutKind],
    pub file: stave_lang::FileId,
    pub span: Span,
}

/// The most spare holes a row may have toward one direction: a bound,
/// so a slip of the pen can't ask for millions.
const MAX_ROW: u32 = 1000;

/// Where a row of holes is cut, for [`Solver::row_copies`].
struct RowAt<'a> {
    binding: &'a Binding,
    kind: &'a KindDef,
    part: PartId,
    part_box: &'a ExactBox,
    frame: &'a Frame,
    /// The hardware's name, for messages.
    name: &'a str,
    cut: Option<&'a CutName>,
    file: stave_lang::FileId,
    span: Span,
}

/// A placed piece of hardware.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Placed {
    pub kind: KindId,
    /// The kind's name as written where it was named.
    pub kind_name: String,
    pub base: BaseKind,
    pub piece: PartId,
    pub mount_face: Face,
    /// The roles, by name, in the kind's order.
    pub roles: Vec<(String, RoleBound)>,
    /// The kind's params and fields for this piece of hardware.
    pub entries: BTreeMap<String, Value>,
    /// The cuts it makes, in the kind's order, one per shape.
    pub cuts: Vec<HardwareCut>,
    /// The fasteners its kind uses: name, diameter, length and count.
    pub fasteners: Vec<stave_model::solved::HardwareFastener>,
    /// Its kind's mechanism, exact, in its own plane.
    pub mechanism: Option<HardwareMechanism>,
    /// That mechanism counted on its own, when it has a drive.
    pub own: Option<stave_model::mechanism::Mechanism>,
    pub model: Option<stave_model::solved::Model>,
    /// Its members with a body, in its kind's order.
    pub members: Vec<PlacedMember>,
    /// The settings its kind's `adjust` entries give it at rest.
    pub settings: Vec<AdjustSetting>,
    /// Whether its kind's rules hold where it is placed.
    pub rules: Rules,
    /// Its kind's `clamp`, along the world axis its frame gives it.
    pub clamp: Option<stave_model::solved::Clamp>,
}

/// Where a piece of hardware sits at rest, before its cuts and motion.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Rest {
    pub piece: PartId,
    pub mount_face: Face,
    /// The mount face as an anchor: `mount` in the kind's rules.
    pub mount: AnchorValue,
    pub frame: Frame,
    pub bodies: Vec<PlacedBody>,
    /// Each member's offset from its nominal place on each axis its own
    /// mount sets, in the kind's order.
    pub offsets: Vec<Offset>,
}

/// The world origin: zero on every axis.
pub(crate) const ORIGIN: PerAxis<ExactLength> = PerAxis {
    across: ExactLength::ZERO,
    up: ExactLength::ZERO,
    deep: ExactLength::ZERO,
};

pub(crate) fn positive_direction(axis: Axis) -> Direction {
    match axis {
        Axis::Up => Direction::Up,
        Axis::Across => Direction::Right,
        Axis::Deep => Direction::Back,
    }
}

/// Each shape of a member in the kind's frame, with the axis of a cylinder
/// and the outline of a prism or a bar.
pub(crate) type FrameShapes = Vec<(FrameBox, Option<Round>, Option<Outline>)>;

/// A cylinder's axis in the kind's frame: the direction it runs along and,
/// for a slanted cylinder, its slant (grammar, "Shapes").
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Round {
    pub axis: FrameAxis,
    pub slant: Option<Slant>,
}

/// How a slanted cylinder is turned, and its size; its box in the frame is
/// centred on the middle of its axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Slant {
    /// The frame direction it leans toward.
    pub toward: FrameAxis,
    /// The angle it is turned by, toward `toward` when positive; more than
    /// -90deg and less than 90deg, never 0.
    pub angle: Angle,
    pub diameter: ExactLength,
    /// How long it is along its axis.
    pub length: ExactLength,
}

/// The world plane an anchor names, and which way is into the part it
/// belongs to.
#[derive(Debug, Clone, Copy)]
struct Plane {
    at: ExactLength,
    /// `true` when the part lies on the positive side of the plane.
    inward_positive: bool,
}

impl Solver<'_> {
    /// Places a piece of hardware. Each is placed once: the result is
    /// handed back, not kept.
    pub(crate) fn place_hardware(&mut self, hardware: HardwareId) -> Solved<Placed> {
        let (file, span, name) = match self.model.hardware.get(hardware.index()) {
            Some(inst) => {
                let (file, span) = self.place_of(DefId::Hardware(inst.def));
                (file, span, inst.name.clone())
            }
            None => (self.project_file, self.project_span, String::new()),
        };
        match self.hardware.get(hardware.index()) {
            Some(Memo::Done(())) => {
                return Err(Fault::Internal("a piece of hardware placed a second time").into());
            }
            Some(Memo::Failed(accounted)) => return Err((*accounted).into()),
            Some(Memo::Busy) => {
                return Err(self
                    .error(
                        Code::PlacementCycle,
                        format!("{name} is placed against itself"),
                        file,
                        span,
                        "place it against something placed earlier, such as the part it mounts on",
                    )
                    .into());
            }
            Some(Memo::Todo) | None => {}
        }
        if let Some(slot) = self.hardware.get_mut(hardware.index()) {
            *slot = Memo::Busy;
        }
        let result = self.compute_hardware(hardware);
        let (memo, result) = match result {
            Ok(placed) => (Memo::Done(()), Ok(placed)),
            Err(failure) => {
                let accounted = self.settle(failure, file, span);
                (Memo::Failed(accounted), Err(accounted.into()))
            }
        };
        if let Some(slot) = self.hardware.get_mut(hardware.index()) {
            *slot = memo;
        }
        result
    }

    fn compute_hardware(&mut self, hardware: HardwareId) -> Solved<Placed> {
        let Some(inst) = self.model.hardware.get(hardware.index()) else {
            return Err(Fault::Internal("a piece of hardware").into());
        };
        let def = self.sound(
            self.model.defs.sound_hardware_def(inst.def),
            "a piece of hardware's declaration",
        )?;
        let array = def.array.as_ref().map(|_| {
            (
                DefId::Hardware(inst.def),
                self.array_len(DefId::Hardware(inst.def), inst.ctx),
            )
        });
        let env = Env {
            ctx: inst.ctx,
            file: def.decl.file,
            kind: None,
            array,
        };
        let name = &inst.name;
        let span = def.call.span;
        let binding = self.bind_call(
            &def.call,
            &env,
            &format!("hardware '{name}'"),
            DefId::Hardware(inst.def),
        )?;
        let kind = self.sound(
            self.model.defs.sound_kind(binding.kind),
            "a kind's declaration",
        )?;
        if !kind.base().is_mounted_hardware() {
            return Err(self
                .error(
                    Code::TypeMismatch,
                    format!(
                        "'{name}' needs a slide, hinge or pull kind, found a {} kind",
                        kind.base().word()
                    ),
                    env.file,
                    def.call.kind.span,
                    "mount hardware with a kind of base slide, hinge or pull",
                )
                .into());
        }
        // Reported where the kind is resolved.
        let Some(hardware_kind) = kind.hardware() else {
            return Err(Fault::Earlier.into());
        };
        let roles = self.hardware_roles(kind, &binding)?;
        let entries = self.hardware_entries(&binding, kind, env.file, span)?;
        let fasteners = self.hardware_fasteners(&binding, kind, &hardware_kind.uses)?;
        let written_model = self.base_field_in(
            &KindEnv::of_binding(&binding),
            BaseField::Model,
            env.file,
            span,
        )?;
        let model = match written_model {
            Some(Value::Model(model)) => {
                let declared = self.declared_shapes(&binding, kind)?;
                Some(self.hardware_model(&binding, kind, &model, &declared)?)
            }
            _ => None,
        };
        let Rest {
            piece,
            mount_face,
            mount: mount_anchor,
            frame,
            bodies,
            offsets,
        } = self.rest(hardware, &binding)?;
        self.check_contact(name, kind, &roles, &bodies, piece, env.file, span)?;
        let cuts = self.hardware_cuts(
            &binding,
            kind,
            &hardware_kind.cuts,
            &roles,
            &bodies,
            &frame,
            name,
            env.file,
            span,
        )?;
        // The kind's mechanism moves it, counted on its own first.
        let (mechanism, own) = match kind.mechanism() {
            Some(def) => {
                let mechanism = self.kind_mechanism(&binding, kind, &binding.name, def, &frame)?;
                let own = self.check_mechanism(hardware, &roles, &mechanism, &binding.name)?;
                (Some(mechanism), own)
            }
            None => (None, None),
        };
        let members = self.placed_members(kind, &roles, &bodies, &frame)?;
        // The kind's rules where it is placed: its adjustments and requires.
        let (settings, rules) = self.hardware_rules(
            Call {
                hardware,
                name,
                file: env.file,
                span,
            },
            &binding,
            kind,
            &hardware_kind.adjusts,
            &offsets,
            mount_anchor,
        )?;
        // One mistake, one error: a kind's own rule speaks first.
        let call = Call {
            hardware,
            name,
            file: env.file,
            span,
        };
        let rules = if rules == Rules::Hold
            && !self.seated(call, kind, &hardware_kind.cuts, &roles, &bodies, &frame)?
        {
            Rules::Broken
        } else {
            rules
        };
        // The clamp's frame axis as a world axis; the stack it draws
        // together is checked once every part is placed (stave-check).
        let clamp = hardware_kind
            .clamp
            .as_ref()
            .map(|clamp| stave_model::solved::Clamp {
                bearing: clamp.bearing,
                drawn: clamp.drawn,
                axis: frame.direction(clamp.axis).axis(),
            });
        Ok(Placed {
            kind: binding.kind,
            kind_name: binding.name,
            base: kind.base(),
            piece,
            mount_face,
            roles,
            entries,
            cuts,
            fasteners,
            mechanism,
            own,
            model,
            members,
            settings,
            rules,
            clamp,
        })
    }

    /// Each member's shapes in the kind's frame, as declared: before a
    /// member's own mount and before the frame is placed on the open axes.
    pub(crate) fn declared_shapes(
        &mut self,
        binding: &Binding,
        kind: &KindDef,
    ) -> Solved<Vec<(BodyId, FrameShapes)>> {
        let mut frame_bodies: Vec<(BodyId, FrameShapes)> = Vec::new();
        for (index, body) in kind.bodies.iter().enumerate() {
            let id = BodyId::new(index).ok_or(Fault::Internal("a member's number"))?;
            let boxes = self.in_kind(binding, kind, |solver, kind_env| {
                body.shapes
                    .iter()
                    .map(|shape| solver.shape_box(&shape.shape, kind_env))
                    .collect::<Solved<Vec<_>>>()
            })?;
            frame_bodies.push((id, boxes));
        }
        Ok(frame_bodies)
    }

    /// Where a piece of hardware sits at rest, from its call `binding`:
    /// its frame, its members with their own mounts, and each member's
    /// offset from its nominal place (grammar, "Hardware placement"). The
    /// help search places it again with other settings.
    pub(crate) fn rest(&mut self, hardware: HardwareId, binding: &Binding) -> Solved<Rest> {
        let Some(inst) = self.model.hardware.get(hardware.index()) else {
            return Err(Fault::Internal("a piece of hardware").into());
        };
        let def = self.sound(
            self.model.defs.sound_hardware_def(inst.def),
            "a piece of hardware's declaration",
        )?;
        let array = def.array.as_ref().map(|_| {
            (
                DefId::Hardware(inst.def),
                self.array_len(DefId::Hardware(inst.def), inst.ctx),
            )
        });
        let env = Env {
            ctx: inst.ctx,
            file: def.decl.file,
            kind: None,
            array,
        };
        let name = &inst.name;
        let kind = self.sound(
            self.model.defs.sound_kind(binding.kind),
            "a kind's declaration",
        )?;
        let kind_name = &binding.name;
        let Some(hardware_kind) = kind.hardware() else {
            return Err(Fault::Earlier.into());
        };
        let Some(Value::Part(piece)) = binding.roles.first().cloned() else {
            return Err(Fault::Earlier.into());
        };
        let piece_box = self.drawn_box(piece)?;
        let piece_orientation = self.part_orientation(piece)?;
        // The mount face fixes the depth axis: the origin lies on it.
        let mount = &hardware_kind.mount;
        let mount_face = match (binding.face, mount.face) {
            (Some(face), _) => face,
            (None, FaceRule::Word(face)) => face,
            (None, FaceRule::Nearer { part, region }) => self.face_nearer(binding, part, region)?,
            (None, FaceRule::Toward { part, other }) => self.face_toward(binding, part, other)?,
        };
        let face_axis = mount_face.axis();
        let face_position = if mount_face.is_positive() {
            piece_box.max(face_axis)
        } else {
            piece_box.min(face_axis)
        };
        let piece_length = piece_orientation.direction(PartAxis::Length);
        let length_direction = if piece_length.axis() == face_axis {
            piece_orientation.direction(PartAxis::Width)
        } else {
            piece_length
        };
        let along_axis = length_direction.axis();
        let Some(across_axis) = along_axis.third(face_axis) else {
            return Err(Fault::Internal("the axes a piece of hardware is laid out on").into());
        };
        let mut frame = Frame {
            origin: ORIGIN,
            along: length_direction,
            across: positive_direction(across_axis),
            depth: mount_face.direction().opposite(),
            across_edge: None,
        };
        frame.origin[face_axis] = face_position;
        // The kind's terms for `across` and `along`; the design's
        // `place { }` for the axes it leaves open.
        let mut open: Vec<FrameAxis> = Vec::new();
        for (axis, rule) in [
            (FrameAxis::Across, &mount.across),
            (FrameAxis::Along, &mount.along),
        ] {
            let world = frame.direction(axis).axis();
            match rule {
                AxisRule::Open => open.push(axis),
                AxisRule::Term(MountTerm::Centre { part }) => {
                    let part = self.role_part(binding, *part)?;
                    let part_box = self.drawn_box(part)?;
                    let centre = midpoint(part_box.min(world), part_box.max(world))
                        .ok_or(Fault::Overflow)?;
                    frame.origin[world] = centre;
                }
                AxisRule::Term(MountTerm::From(from)) => {
                    let (plane, at) = self.term_plane(binding, kind, mount_face, world, from)?;
                    let inward = if plane.inward_positive {
                        positive_direction(world)
                    } else {
                        positive_direction(world).opposite()
                    };
                    match axis {
                        FrameAxis::Across => {
                            frame.across = inward;
                            frame.across_edge = Some(plane.at);
                        }
                        FrameAxis::Along => frame.along = inward,
                        FrameAxis::Depth => {}
                    }
                    frame.origin[world] = at;
                }
            }
        }
        // Each member's shapes in the frame, before the frame is placed on
        // the open axes.
        let frame_bodies = self.declared_shapes(binding, kind)?;
        let label = inst.ctx.element.and_then(|element| {
            match self.model.defs.array(DefId::Hardware(inst.def)) {
                Some(stave_model::defs::ArrayDef::Labels(labels)) => {
                    labels.get(element).map(|(l, _)| l.clone())
                }
                Some(stave_model::defs::ArrayDef::Count(_)) => Some(element.to_string()),
                None => None,
            }
        });
        let terms = self.sort_terms(&def.place, &env, name, label.as_deref(), true)?;
        let piece_role = RoleIndex::new(0).ok_or(Fault::Internal("a role's number"))?;
        // `place { }` positions the box around the members on the piece.
        let carried: Option<FrameBox> = frame_bodies
            .iter()
            .filter(|(id, _)| {
                kind.bodies
                    .get(id.index())
                    .is_some_and(|body| body.with == Carrier::Role(piece_role))
            })
            .flat_map(|(_, boxes)| boxes.iter().map(|(b, _, _)| *b))
            .reduce(|a, b| a.union(&b));
        for world in Axis::ALL {
            let term = terms.axis(world).copied();
            let frame_axis = [FrameAxis::Along, FrameAxis::Across, FrameAxis::Depth]
                .into_iter()
                .find(|axis| frame.direction(*axis).axis() == world)
                .ok_or(Fault::Internal(
                    "the axes a piece of hardware is laid out on",
                ))?;
            match (term, open.contains(&frame_axis)) {
                (Some(term), false) => {
                    let why = if world == face_axis {
                        "the mount face already fixes it"
                    } else {
                        "the hardware's kind already fixes it"
                    };
                    let reported = self.report(
                        Diagnostic::error(
                            Code::AxisSetTwice,
                            format!("{name}: position along '{}' is set twice", world.word()),
                            Label::new(env.file, term.span),
                        )
                        .with_help(format!("remove this term: {why}")),
                    );
                    return Err(reported.into());
                }
                (Some(term), true) => {
                    let Some(carried) = carried else {
                        return Err(self.error(
                            Code::MissingField,
                            format!(
                                "kind '{kind_name}' leaves '{}' open but has no member on its piece to place",
                                frame_axis.word()
                            ),
                            env.file,
                            term.span,
                            "give the kind a member with: piece and a shape, or set the axis in its mount",
                        ).into());
                    };
                    let (low, high) = carried.range(frame_axis);
                    let size = high.checked_sub(low).ok_or(Fault::Overflow)?;
                    let count = self.hardware_siblings(hardware);
                    let siblings = Siblings {
                        index: inst.ctx.element.unwrap_or(0),
                        sizes: vec![size; count],
                        part: None,
                    };
                    let placed_low = self.term_low(&term, world, size, &env, Some(&siblings))?;
                    let origin = if frame.direction(frame_axis).is_positive() {
                        placed_low.checked_sub(low)
                    } else {
                        placed_low.checked_add(high)
                    }
                    .ok_or(Fault::Overflow)?;
                    frame.origin[world] = origin;
                }
                (None, true) => {
                    let place_span = def.place_span.unwrap_or(def.decl.span);
                    return Err(self.axis_not_set(name, world, env.file, place_span).into());
                }
                (None, false) => {}
            }
        }
        // Every member at rest: its own mount, then the frame.
        let mut all_offsets = Vec::new();
        let mut bodies = Vec::new();
        for (id, boxes) in &frame_bodies {
            let Some(body) = kind.bodies.get(id.index()) else {
                return Err(Fault::Internal("a member of a kind").into());
            };
            let mut offsets: Vec<(FrameAxis, ExactLength)> = Vec::new();
            for placement in &body.mount {
                let world = frame.direction(placement.axis).axis();
                let (_, at) = self.term_plane(binding, kind, mount_face, world, &placement.from)?;
                let offset = frame.coordinate(placement.axis, at)?;
                offsets.push((placement.axis, offset));
                all_offsets.push((*id, placement.axis, offset));
            }
            let mut shapes = Vec::new();
            for (frame_box, cylinder, outline) in boxes {
                let mut moved = *frame_box;
                let offset_on = |axis: FrameAxis| {
                    offsets
                        .iter()
                        .filter(|(on, _)| *on == axis)
                        .map(|(_, by)| *by)
                        .try_fold(ExactLength::ZERO, |sum, by| sum.checked_add(by))
                        .ok_or(Fault::Overflow)
                };
                let outline = match outline {
                    Some(outline) => Some(
                        outline
                            .moved(offset_on(FrameAxis::Across)?, offset_on(FrameAxis::Depth)?)?,
                    ),
                    None => None,
                };
                for (axis, offset) in &offsets {
                    let (low, high) = moved.range(*axis);
                    moved = moved.with(
                        *axis,
                        (
                            low.checked_add(*offset).ok_or(Fault::Overflow)?,
                            high.checked_add(*offset).ok_or(Fault::Overflow)?,
                        ),
                    );
                }
                shapes.push(PlacedShape {
                    frame: moved,
                    world: frame.world_box(&moved)?,
                    cylinder: *cylinder,
                    outline,
                });
            }
            bodies.push(PlacedBody { body: *id, shapes });
        }
        let mount = AnchorValue {
            axis: face_axis,
            position: face_position,
            face: Some(mount_face),
            owner: AnchorOwner::Part(piece),
        };
        Ok(Rest {
            piece,
            mount_face,
            mount,
            frame,
            bodies,
            offsets: all_offsets,
        })
    }

    /// Every member with a body at rest in the world, with what moves it.
    fn placed_members(
        &mut self,
        kind: &KindDef,
        roles: &[(String, RoleBound)],
        bodies: &[PlacedBody],
        frame: &Frame,
    ) -> Solved<Vec<PlacedMember>> {
        let mut members = Vec::with_capacity(bodies.len());
        for placed in bodies {
            let body = kind
                .bodies
                .get(placed.body.index())
                .ok_or(Fault::Internal("a member of a kind"))?;
            let Some(carrier) = member_carrier(kind, roles, placed.body) else {
                return Err(self
                    .error(
                        Code::MissingField,
                        format!(
                            "member '{}' has no 'with:' and nothing in its kind moves it",
                            body.name.as_str()
                        ),
                        kind.decl.file,
                        body.span,
                        "give it with: a role or another member, or pin it in the kind's mechanism",
                    )
                    .into());
            };
            let shapes = placed
                .shapes
                .iter()
                .map(|shape| world_shape(shape, frame))
                .collect::<Solved<Vec<_>>>()?;
            members.push(PlacedMember {
                body: placed.body,
                name: body.name.as_str().to_owned(),
                carrier,
                shapes,
            });
        }
        Ok(members)
    }

    /// The fasteners a kind's `uses:` names, for one piece of hardware.
    fn hardware_fasteners(
        &mut self,
        binding: &Binding,
        kind: &KindDef,
        uses: &[stave_model::defs::UseDef],
    ) -> Solved<Vec<stave_model::solved::HardwareFastener>> {
        let file = kind.decl.file;
        self.in_kind(binding, kind, |solver, kind_env| {
            let mut out = Vec::new();
            for used in uses {
                let Value::Kind(fastener) = solver.eval(&used.fastener, kind_env)? else {
                    return Err(Fault::Earlier.into());
                };
                let count = match &used.count {
                    Some(count) => solver.eval_count(count, kind_env)?,
                    None => 1,
                };
                let mut size = |field: BaseField| -> Solved<Length> {
                    match solver.base_field(&fastener, field, file, used.span)? {
                        Some(Value::Length(length)) => length
                            .to_nearest_length()
                            .ok_or(Failure::Fault(Fault::Overflow)),
                        _ => Err(Fault::Earlier.into()),
                    }
                };
                let diameter = size(BaseField::Diameter)?;
                let length = size(BaseField::Length)?;
                let name = solver.def_name(DefId::Kind(fastener.kind));
                out.push(stave_model::solved::HardwareFastener {
                    kind: name,
                    diameter,
                    length,
                    count,
                });
            }
            Ok(out)
        })
    }

    /// The roles of a piece of hardware, by name, in the kind's order.
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "takes only the values it names: any other, a new variant included, is a type error"
    )]
    fn hardware_roles(
        &mut self,
        kind: &KindDef,
        binding: &Binding,
    ) -> Solved<Vec<(String, RoleBound)>> {
        kind.roles
            .iter()
            .zip(&binding.roles)
            .map(|(role, value)| match value {
                Value::Part(part) => Ok((role.name.clone(), RoleBound::Part(*part))),
                Value::Region(region) => Ok((role.name.clone(), RoleBound::Region(*region))),
                _ => Err(Fault::Earlier.into()),
            })
            .collect()
    }

    /// Every param and field of the kind, for one piece of hardware.
    fn hardware_entries(
        &mut self,
        binding: &Binding,
        kind: &KindDef,
        file: stave_lang::FileId,
        span: Span,
    ) -> Solved<BTreeMap<String, Value>> {
        let kind_env = KindEnv::of_binding(binding);
        let mut entries = BTreeMap::new();
        for (index, param) in kind.params.iter().enumerate() {
            let value = self.kind_param(&kind_env, index, file, span)?;
            entries.insert(param.decl.name.clone(), value);
        }
        let Some(ctx) = self.module_ctx(DefId::Kind(binding.kind)) else {
            return Err(Fault::Internal("the file an item is declared in").into());
        };
        let kind_eval = Env {
            ctx,
            file: kind.decl.file,
            kind: Some(&kind_env),
            array: None,
        };
        for field in kind.fields() {
            let value = self.eval(&field.value, &kind_eval)?;
            entries.insert(field.name.clone(), value);
        }
        Ok(entries)
    }

    /// The part a role of the hardware is bound to.
    fn role_part(&mut self, binding: &Binding, role: RoleIndex) -> Solved<PartId> {
        match binding.roles.get(role.index()) {
            Some(Value::Part(part)) => Ok(*part),
            _ => Err(Fault::Earlier.into()),
        }
    }

    /// The box of the part or region a role is bound to.
    fn role_box(&mut self, binding: &Binding, role: RoleIndex) -> Solved<ExactBox> {
        match binding.roles.get(role.index()).cloned() {
            Some(Value::Part(part)) => self.drawn_box(part),
            Some(Value::Region(region)) => Ok(self.region_value(region)?.bounds),
            _ => Err(Fault::Earlier.into()),
        }
    }

    /// `face(piece, nearer: region)`: of the two faces across the piece's
    /// thickness, the one nearer the same-named face of the region; the
    /// lower one when they are equally near.
    fn face_nearer(
        &mut self,
        binding: &Binding,
        part: RoleIndex,
        region: RoleIndex,
    ) -> Solved<Face> {
        let part_id = self.role_part(binding, part)?;
        let part_box = self.drawn_box(part_id)?;
        let region_box = self.role_box(binding, region)?;
        let axis = self
            .part_orientation(part_id)?
            .direction(PartAxis::Thickness)
            .axis();
        let low = part_box
            .min(axis)
            .checked_sub(region_box.min(axis))
            .ok_or(Fault::Overflow)?;
        let high = region_box
            .max(axis)
            .checked_sub(part_box.max(axis))
            .ok_or(Fault::Overflow)?;
        let positive = abs(high)? < abs(low)?;
        Face::ALL
            .into_iter()
            .find(|face| face.axis() == axis && face.is_positive() == positive)
            .ok_or(Failure::Fault(Fault::Internal(
                "the face a piece of hardware sits on",
            )))
    }

    /// `face(piece, toward: other)`: of the two faces across the piece's
    /// thickness, the one nearer the middle of the other role's part; the
    /// lower one when the middles are level.
    fn face_toward(
        &mut self,
        binding: &Binding,
        part: RoleIndex,
        other: RoleIndex,
    ) -> Solved<Face> {
        let part_id = self.role_part(binding, part)?;
        let part_box = self.drawn_box(part_id)?;
        let other_box = self.role_box(binding, other)?;
        let axis = self
            .part_orientation(part_id)?
            .direction(PartAxis::Thickness)
            .axis();
        let middle = midpoint(part_box.min(axis), part_box.max(axis)).ok_or(Fault::Overflow)?;
        let other_middle =
            midpoint(other_box.min(axis), other_box.max(axis)).ok_or(Fault::Overflow)?;
        let positive = other_middle > middle;
        Face::ALL
            .into_iter()
            .find(|face| face.axis() == axis && face.is_positive() == positive)
            .ok_or(Failure::Fault(Fault::Internal(
                "the face a piece of hardware sits on",
            )))
    }

    /// The plane a `from(anchor, inset: d)` names across a world axis, and
    /// the origin's coordinate on it.
    fn term_plane(
        &mut self,
        binding: &Binding,
        kind: &KindDef,
        mount_face: Face,
        axis: Axis,
        from: &FromTerm,
    ) -> Solved<(Plane, ExactLength)> {
        let plane = self.anchor_plane(binding, kind, mount_face, axis, &from.anchor, from.span)?;
        let inset = match &from.inset {
            Some(inset) => self.in_kind(binding, kind, |solver, kind_env| {
                solver.kind_length(inset, kind_env)
            })?,
            None => ExactLength::ZERO,
        };
        let at = if plane.inward_positive {
            plane.at.checked_add(inset)
        } else {
            plane.at.checked_sub(inset)
        }
        .ok_or(Fault::Overflow)?;
        Ok((plane, at))
    }

    /// The world plane of an anchor, across a world axis.
    fn anchor_plane(
        &mut self,
        binding: &Binding,
        kind: &KindDef,
        mount_face: Face,
        axis: Axis,
        anchor: &MountSelector,
        span: Span,
    ) -> Solved<Plane> {
        let file = kind.decl.file;
        let side = |part_box: &ExactBox, high: bool| Plane {
            at: if high {
                part_box.max(axis)
            } else {
                part_box.min(axis)
            },
            inward_positive: !high,
        };
        let middle = |b: &ExactBox| midpoint(b.min(axis), b.max(axis)).ok_or(Fault::Overflow);
        let nearer_high = |part_box: &ExactBox, target: ExactLength| -> Solved<bool> {
            let low = abs(target
                .checked_sub(part_box.min(axis))
                .ok_or(Fault::Overflow)?)?;
            let high = abs(target
                .checked_sub(part_box.max(axis))
                .ok_or(Fault::Overflow)?)?;
            Ok(high < low)
        };
        let across_the_face = |solver: &mut Self, what: &str| -> Solved<()> {
            if axis == mount_face.axis() {
                return Err(solver.error(
                    Code::TypeMismatch,
                    format!("{what} is not across this axis"),
                    file,
                    span,
                    "an edge or an end of the mount face places across or along it, not into it",
                ).into());
            }
            Ok(())
        };
        match *anchor {
            MountSelector::EdgeNearer { part, other } => {
                across_the_face(self, "an edge")?;
                let part_box = self.role_box(binding, part)?;
                let target = middle(&self.role_box(binding, other)?)?;
                Ok(side(&part_box, nearer_high(&part_box, target)?))
            }
            MountSelector::EdgeFlushWith { part, other } => {
                across_the_face(self, "an edge")?;
                let part_box = self.role_box(binding, part)?;
                let other_box = self.role_box(binding, other)?;
                let flush =
                    |edge: ExactLength| edge == other_box.min(axis) || edge == other_box.max(axis);
                let high = match (flush(part_box.min(axis)), flush(part_box.max(axis))) {
                    (true, false) => false,
                    (false, true) => true,
                    (true, true) | (false, false) => nearer_high(&part_box, middle(&other_box)?)?,
                };
                Ok(side(&part_box, high))
            }
            MountSelector::EndToward { part, toward } => {
                let part_id = self.role_part(binding, part)?;
                let length = self
                    .part_orientation(part_id)?
                    .direction(PartAxis::Length)
                    .axis();
                if length != toward.axis() {
                    let name = self.part_name(part_id);
                    return Err(self.error(
                        Code::TypeMismatch,
                        format!(
                            "end({name}, toward: {}) needs {name}'s length to run {}, but it runs along '{}'",
                            toward.word(),
                            toward.word(),
                            length.word()
                        ),
                        file,
                        span,
                        "name a direction the part's length runs in, or turn the part so its length runs that way",
                    ).into());
                }
                if toward.axis() != axis {
                    return Err(self
                        .error(
                            Code::TypeMismatch,
                            format!(
                                "an end toward {} places along '{}', not along '{}'",
                                toward.word(),
                                toward.axis().word(),
                                axis.word()
                            ),
                            file,
                            span,
                            "measure along from an end, and across from an edge",
                        )
                        .into());
                }
                let part_box = self.drawn_box(part_id)?;
                Ok(side(&part_box, toward.is_positive()))
            }
            MountSelector::EndNearer { part, other } => {
                let part_id = self.role_part(binding, part)?;
                let length = self
                    .part_orientation(part_id)?
                    .direction(PartAxis::Length)
                    .axis();
                if length != axis {
                    let name = self.part_name(part_id);
                    return Err(self
                        .error(
                            Code::TypeMismatch,
                            format!(
                                "an end of {name} measures along its length, which runs '{}', not '{}'",
                                length.word(),
                                axis.word()
                            ),
                            file,
                            span,
                            "measure along from an end, and across from an edge",
                        )
                        .into());
                }
                let part_box = self.drawn_box(part_id)?;
                let target = middle(&self.role_box(binding, other)?)?;
                Ok(side(&part_box, nearer_high(&part_box, target)?))
            }
            MountSelector::FaceToward { part, other } => {
                let part_box = self.role_box(binding, part)?;
                let target = middle(&self.role_box(binding, other)?)?;
                Ok(side(&part_box, nearer_high(&part_box, target)?))
            }
            MountSelector::FaceNearer { part, region } => {
                // The face nearer the same-named face of the region, as a
                // kind's `face: face(p, nearer: r)`, across this axis.
                let part_box = self.role_box(binding, part)?;
                let region_box = self.role_box(binding, region)?;
                let low = part_box
                    .min(axis)
                    .checked_sub(region_box.min(axis))
                    .ok_or(Fault::Overflow)?;
                let high = region_box
                    .max(axis)
                    .checked_sub(part_box.max(axis))
                    .ok_or(Fault::Overflow)?;
                Ok(side(&part_box, abs(high)? < abs(low)?))
            }
            MountSelector::Crossing { part, other } => {
                let part_box = self.role_box(binding, part)?;
                let other_box = self.role_box(binding, other)?;
                let within =
                    |at: ExactLength| at >= other_box.min(axis) && at <= other_box.max(axis);
                let high = match (within(part_box.min(axis)), within(part_box.max(axis))) {
                    (true, false) => false,
                    (false, true) => true,
                    (true, true) | (false, false) => nearer_high(&part_box, middle(&other_box)?)?,
                };
                Ok(side(&part_box, high))
            }
        }
    }

    /// A shape's box in the kind's frame, the axis of a cylinder and the
    /// outline of a prism or a bar.
    pub(crate) fn shape_box(
        &mut self,
        shape: &ShapeDef,
        env: &Env<'_>,
    ) -> Solved<(FrameBox, Option<Round>, Option<Outline>)> {
        match shape {
            ShapeDef::Box { .. } => Ok((self.frame_box(shape, env)?, None, None)),
            ShapeDef::Cylinder {
                diameter,
                extent,
                at,
                axis,
                tilt,
            } => {
                let slant = match tilt {
                    Some(tilt) => self
                        .cylinder_tilt(&tilt.angle, env)?
                        .map(|angle| (tilt.toward, angle)),
                    None => None,
                };
                if let Some((toward, angle)) = slant {
                    let diameter = self.kind_length(diameter, env)?;
                    let centre = self.kind_point(at, env)?;
                    let range = self.frame_range(extent, env)?;
                    return self.slanted_box(*axis, toward, angle, diameter, centre, range);
                }
                let radius = half(self.kind_length(diameter, env)?)?;
                let centre = self.kind_point(at, env)?;
                let range = self.frame_range(extent, env)?;
                let around = |value: ExactLength| -> Solved<(ExactLength, ExactLength)> {
                    Ok((
                        value.checked_sub(radius).ok_or(Fault::Overflow)?,
                        value.checked_add(radius).ok_or(Fault::Overflow)?,
                    ))
                };
                let frame_box = FrameBox {
                    along: if *axis == FrameAxis::Along {
                        range
                    } else {
                        around(centre.along)?
                    },
                    across: if *axis == FrameAxis::Across {
                        range
                    } else {
                        around(centre.across)?
                    },
                    depth: if *axis == FrameAxis::Depth {
                        range
                    } else {
                        around(centre.depth)?
                    },
                };
                Ok((
                    frame_box,
                    Some(Round {
                        axis: *axis,
                        slant: None,
                    }),
                    None,
                ))
            }
            ShapeDef::Prism { points, along } => {
                let mut points_out = Vec::new();
                for point in points.iter() {
                    points_out.push(self.kind_point(point, env)?);
                }
                let along = self.frame_range(along, env)?;
                let bounds = |pick: fn(&ExactPoint) -> ExactLength| {
                    let values: Vec<ExactLength> = points_out.iter().map(pick).collect();
                    match (values.iter().min(), values.iter().max()) {
                        (Some(low), Some(high)) => Ok((*low, *high)),
                        _ => Err(Fault::Earlier),
                    }
                };
                let outline =
                    Outline::Prism(points_out.iter().map(|p| [p.across, p.depth]).collect());
                Ok((
                    FrameBox {
                        along,
                        across: bounds(|p| p.across)?,
                        depth: bounds(|p| p.depth)?,
                    },
                    None,
                    Some(outline),
                ))
            }
            ShapeDef::Bar {
                from,
                to,
                thickness,
                along,
            } => {
                // Conservative: the segment's box grown by the bar's whole
                // thickness holds the bar whichever way it lies.
                let from = self.kind_point(from, env)?;
                let to = self.kind_point(to, env)?;
                let grow = self.kind_length(thickness, env)?;
                let span = |a: ExactLength, b: ExactLength| -> Solved<(ExactLength, ExactLength)> {
                    Ok((
                        a.min(b).checked_sub(grow).ok_or(Fault::Overflow)?,
                        a.max(b).checked_add(grow).ok_or(Fault::Overflow)?,
                    ))
                };
                Ok((
                    FrameBox {
                        along: self.frame_range(along, env)?,
                        across: span(from.across, to.across)?,
                        depth: span(from.depth, to.depth)?,
                    },
                    None,
                    Some(Outline::Bar {
                        from: [from.across, from.depth],
                        to: [to.across, to.depth],
                        thickness: grow,
                    }),
                ))
            }
        }
    }

    /// Every part the hardware joins must meet it: at least one member that
    /// part carries touches or enters it at rest.
    #[allow(clippy::too_many_arguments)]
    fn check_contact(
        &mut self,
        name: &str,
        kind: &KindDef,
        roles: &[(String, RoleBound)],
        bodies: &[PlacedBody],
        piece: PartId,
        file: stave_lang::FileId,
        span: Span,
    ) -> Solved<()> {
        for (index, (_, bound)) in roles.iter().enumerate() {
            let RoleBound::Part(part) = bound else {
                continue;
            };
            let role = RoleIndex::new(index).ok_or(Fault::Internal("a role's number"))?;
            let carried: Vec<&PlacedBody> = bodies
                .iter()
                .filter(|placed| {
                    kind.bodies
                        .get(placed.body.index())
                        .is_some_and(|body| body.with == Carrier::Role(role))
                })
                .collect();
            if carried.is_empty() {
                continue;
            }
            let part_box = self.drawn_box(*part)?;
            let meets = carried
                .iter()
                .flat_map(|placed| placed.shapes.iter())
                .any(|shape| meets(&shape.world, &part_box));
            if !meets {
                let piece_name = self.part_name(piece);
                let part_name = self.part_name(*part);
                let members: Vec<&str> = carried
                    .iter()
                    .filter_map(|placed| kind.bodies.get(placed.body.index()))
                    .map(|body| body.name.as_str())
                    .collect();
                return Err(self.error(
                    Code::NoContact,
                    format!("{name}: {piece_name} meets none of the parts it should join"),
                    file,
                    span,
                    format!(
                        "its {} must touch {part_name}: place {piece_name} so its mount face lies where the kind needs it, or name the face that does with face: ...",
                        members.join(" and ")
                    ),
                ).into());
            }
        }
        Ok(())
    }

    /// Every seated member lies wholly on its part across the face it is
    /// seated on (grammar, "Cuts stay in their part", `error[overhang]`):
    /// a member a cut houses, across the face the cut is made from; one
    /// its own `mount` places against its part, across the face it lies
    /// against. Only its depth may run past, and a housed member
    /// must enter its part, not only touch it. `false` when one doesn't,
    /// reported like a failed rule: once for a call, the rest of an array
    /// as notes.
    fn seated(
        &mut self,
        call: Call<'_>,
        kind: &KindDef,
        cuts: &[stave_model::defs::MemberCutDef],
        roles: &[(String, RoleBound)],
        bodies: &[PlacedBody],
        frame: &Frame,
    ) -> Solved<bool> {
        let name = call.name;
        let part_of = |role: RoleIndex| match roles.get(role.index()) {
            Some((_, RoleBound::Part(part))) => Some(*part),
            Some((_, RoleBound::Region(_))) | None => None,
        };
        for placed in bodies {
            let Some(def) = kind.bodies.get(placed.body.index()) else {
                continue;
            };
            let mut seats: Vec<(PartId, Seat)> = cuts
                .iter()
                .filter(|cut| cut.body == placed.body)
                .filter_map(|cut| part_of(cut.part).map(|part| (part, Seat::Cut)))
                .collect();
            if let Carrier::Role(role) = def.with
                && let Some(part) = part_of(role)
                && !def.mount.is_empty()
            {
                seats.push((part, Seat::Against));
            }
            for (part, seat) in seats {
                let part_box = self.drawn_box(part)?;
                for (index, shape) in placed.shapes.iter().enumerate() {
                    let over = if seat == Seat::Cut
                        && let Some(axis) = shape.world.touching_axis(&part_box)
                    {
                        // Housed, but only touching its part: it lies wholly
                        // past it, and its cut would have no length.
                        Some(Overhang {
                            axis,
                            held: ExactLength::ZERO,
                            whole: size_on(&shape.world, axis),
                        })
                    } else {
                        let normal = match seat {
                            Seat::Cut => {
                                crossing_face(&shape.world, &part_box, frame).map(Face::axis)
                            }
                            Seat::Against => shape.world.touching_axis(&part_box).or_else(|| {
                                crossing_face(&shape.world, &part_box, frame).map(Face::axis)
                            }),
                        };
                        normal.and_then(|normal| overhang(&shape.world, &part_box, normal))
                    };
                    let Some(over) = over else {
                        continue;
                    };
                    let member = def.name.as_str();
                    let member_text = match def.shapes.get(index).and_then(|s| s.name.as_deref()) {
                        Some(shape_name) if def.shapes.len() > 1 => {
                            format!("the {shape_name} shape of its {member}")
                        }
                        Some(_) | None => format!("its {member}"),
                    };
                    let part_name = self.part_name(part);
                    let surface = match self.part_orientation(part)?.part_axis(over.axis) {
                        PartAxis::Length => "hangs off the end",
                        PartAxis::Width => "hangs off the edge",
                        PartAxis::Thickness => "hangs past the face",
                    };
                    let note = format!(
                        "{part_name} holds {} of the {} {member}",
                        self.describe(&Value::Length(over.held)),
                        self.describe(&Value::Length(over.whole)),
                    );
                    if self.noted_with_earlier(Code::Overhang, call) {
                        return Ok(false);
                    }
                    let diagnostic = Diagnostic::error(
                        Code::Overhang,
                        format!("{name}: {member_text} {surface} of {part_name}"),
                        Label::new(call.file, call.span),
                    )
                    .with_note(note)
                    .with_help(format!(
                        "place {name} so its {member} lies wholly on {part_name}"
                    ));
                    self.deferred.push(diagnostic);
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    /// The cuts a kind makes for one piece of hardware: one per shape of
    /// the member each cut names.
    #[allow(clippy::too_many_arguments)]
    fn hardware_cuts(
        &mut self,
        binding: &Binding,
        kind: &KindDef,
        cuts: &[stave_model::defs::MemberCutDef],
        roles: &[(String, RoleBound)],
        bodies: &[PlacedBody],
        frame: &Frame,
        name: &str,
        file: stave_lang::FileId,
        span: Span,
    ) -> Solved<Vec<HardwareCut>> {
        let mut out = Vec::new();
        for cut in cuts {
            let Some((_, RoleBound::Part(part))) = roles.get(cut.part.index()).cloned() else {
                return Err(Fault::Earlier.into());
            };
            let Some(placed) = bodies.iter().find(|placed| placed.body == cut.body) else {
                return Err(Fault::Internal("a member a cut is made by").into());
            };
            let (kinds, walls) = self.in_kind(binding, kind, |solver, kind_env| {
                let mut kinds = Vec::new();
                let mut walls = Vec::new();
                for operation in &cut.operations {
                    let made = solver.cut_kind(operation, kind_env)?;
                    walls.push(solver.cut_wall(operation, kind_env)?);
                    let at = CutAt {
                        owner: name,
                        file,
                        span,
                    };
                    solver.check_cut_size(at, cut.name.as_ref(), operation, &made, part)?;
                    kinds.push(made);
                }
                Ok((kinds, walls))
            })?;
            let part_box = self.drawn_box(part)?;
            let first = out.len();
            for (index, shape) in placed.shapes.iter().enumerate() {
                let Some(face) = crossing_face(&shape.world, &part_box, frame) else {
                    let part_name = self.part_name(part);
                    let body = kind.bodies.get(cut.body.index());
                    let member = body
                        .map(|body| body.name.as_str().to_owned())
                        .unwrap_or_default();
                    // A member of several shapes: name the one that misses,
                    // since another may meet the part.
                    let missing = match body.filter(|body| body.shapes.len() > 1).and_then(|body| {
                        body.shapes
                            .get(index)
                            .and_then(|named| named.name.as_deref())
                    }) {
                        Some(shape_name) => format!("the {shape_name} shape of {member}"),
                        None => member.clone(),
                    };
                    return Err(self.error(
                        Code::NoContact,
                        format!("{name}: {missing} meets none of the parts it should join"),
                        file,
                        span,
                        format!(
                            "its {member} must touch or enter {part_name} for the kind's cut there"
                        ),
                    ).into());
                };
                let slant = WorldSlant::of(&world_shape(shape, frame)?);
                let member = kind
                    .bodies
                    .get(cut.body.index())
                    .map(|body| body.name.as_str().to_owned())
                    .unwrap_or_default();
                self.check_slanted_cut(&SlantedCut {
                    owner: name,
                    member: &member,
                    part,
                    face,
                    slant,
                    kinds: &kinds,
                    file,
                    span,
                })?;
                for (kind_of_cut, wall) in kinds.iter().zip(&walls) {
                    out.push(HardwareCut {
                        part,
                        body: cut.body,
                        name: cut.name.clone(),
                        face,
                        shape: shape.world,
                        slant,
                        kind: kind_of_cut.clone(),
                        wall: *wall,
                    });
                }
            }
            if let Some(row) = &cut.row {
                let own: Vec<HardwareCut> = out.get(first..).unwrap_or_default().to_vec();
                let at = RowAt {
                    binding,
                    kind,
                    part,
                    part_box: &part_box,
                    frame,
                    name,
                    cut: cut.name.as_ref(),
                    file,
                    span,
                };
                let copies = self.row_copies(&at, row, &own)?;
                out.extend(copies);
            }
        }
        Ok(out)
    }

    /// The copies a `row(...)` cut adds beside the member's own cuts
    /// (grammar, "Rows of holes"): each moved a whole number of pitches
    /// toward a direction along the face it is cut in, with nothing in
    /// it. A copy that would not lie wholly on that face, along the row,
    /// is `error[wall]`.
    fn row_copies(
        &mut self,
        at: &RowAt<'_>,
        row: &stave_model::defs::RowDef,
        own: &[HardwareCut],
    ) -> Solved<Vec<HardwareCut>> {
        let what = at
            .cut
            .map_or_else(|| "hole".to_owned(), stave_model::defs::CutName::words);
        let part_name = self.part_name(at.part);
        let name = at.name;
        let (pitch, counts) = self.in_kind(at.binding, at.kind, |solver, kind_env| {
            let pitch = solver.kind_length(&row.pitch, kind_env)?;
            let mut counts = Vec::new();
            for copies in &row.copies {
                let Value::Count(count) = solver.eval(&copies.count, kind_env)? else {
                    return Err(Fault::Earlier.into());
                };
                counts.push((copies.toward, count));
            }
            Ok((pitch, counts))
        })?;
        if pitch <= ExactLength::ZERO {
            let pitch_text = self.describe(&Value::Length(pitch));
            return Err(self
                .error(
                    Code::OutOfRange,
                    format!("{name}: the row of {what}s has a pitch of {pitch_text}"),
                    at.file,
                    at.span,
                    "the pitch is how far apart the holes are, from centre to centre; make it more than 0mm",
                )
                .into());
        }
        let depth_axis = at.frame.depth.axis();
        let length_axis = self
            .part_orientation(at.part)?
            .direction(PartAxis::Length)
            .axis();
        let mut out = Vec::new();
        for (toward, count) in counts {
            let word = toward.word();
            let whole = if count.is_integer() && !count.is_negative() {
                u32::try_from(count.numerator())
                    .ok()
                    .filter(|n| *n <= MAX_ROW)
            } else {
                None
            };
            let Some(whole) = whole else {
                let count_text = self.describe(&Value::Count(count));
                return Err(self
                    .error(
                        Code::OutOfRange,
                        format!("{name}: the row of {what}s has {count_text} holes {word}"),
                        at.file,
                        at.span,
                        format!("give a whole number of spare holes {word}, from 0 to {MAX_ROW}"),
                    )
                    .into());
            };
            let axis = toward.axis();
            if axis == depth_axis {
                return Err(self
                    .error(
                        Code::TypeMismatch,
                        format!("{name}: the row of {what}s runs {word}, into {part_name}"),
                        at.file,
                        at.span,
                        format!(
                            "a row of holes runs along the face of {part_name} it is drilled in; name a direction along that face"
                        ),
                    )
                    .into());
            }
            for step in 1..=whole {
                let mut offset = pitch
                    .checked_mul(Rational::from_int(i128::from(step)))
                    .ok_or(Fault::Overflow)?;
                if !toward.is_positive() {
                    offset = offset.checked_neg().ok_or(Fault::Overflow)?;
                }
                for cut in own {
                    let low = cut
                        .shape
                        .min(axis)
                        .checked_add(offset)
                        .ok_or(Fault::Overflow)?;
                    let high = cut
                        .shape
                        .max(axis)
                        .checked_add(offset)
                        .ok_or(Fault::Overflow)?;
                    if low < at.part_box.min(axis) || high > at.part_box.max(axis) {
                        let room = step - 1;
                        let side = if axis == length_axis { "end" } else { "edge" };
                        return Err(self
                            .error(
                                Code::Wall,
                                format!(
                                    "{name}: its row of {what}s runs off {part_name}: hole {step} {word} is past its {side}"
                                ),
                                at.file,
                                at.span,
                                format!(
                                    "there is room for {room} {word} from where it sits; drill fewer holes {word}, or move it"
                                ),
                            )
                            .into());
                    }
                    let mut copy = cut.clone();
                    copy.shape = cut.shape.with(axis, low, high);
                    out.push(copy);
                }
            }
        }
        Ok(out)
    }

    /// The cuts a slanted member makes (grammar, "Slanted members"): only
    /// `drill` follows a slant, and only into a face the cylinder runs
    /// into end first. `error[type-mismatch]` at the call otherwise.
    pub(crate) fn check_slanted_cut(&mut self, cut: &SlantedCut<'_>) -> Solved<()> {
        let Some(slant) = cut.slant else {
            return Ok(());
        };
        let part_name = self.part_name(cut.part);
        let (owner, member) = (cut.owner, cut.member);
        if let Some(other) = cut
            .kinds
            .iter()
            .find(|kind| !matches!(kind, CutKind::Drill { .. }))
        {
            let word = other.operation().word();
            return Err(self
                .error(
                    Code::TypeMismatch,
                    format!("{owner}: a {word} can't follow the slant of its {member}"),
                    cut.file,
                    cut.span,
                    "cut round a slanted member with drill",
                )
                .into());
        }
        if cut.face.axis() != slant.axis {
            return Err(self
                .error(
                    Code::TypeMismatch,
                    format!(
                        "{owner}: its {member} goes into {part_name} from the side, so its hole can't follow the slant"
                    ),
                    cut.file,
                    cut.span,
                    format!(
                        "slant the shape less, or lay it out so it goes into {part_name} end first"
                    ),
                )
                .into());
        }
        Ok(())
    }

    /// The environment a hardware kind's own expressions are evaluated
    /// in, for one piece of hardware, handed to `body`.
    pub(crate) fn in_kind<T>(
        &mut self,
        binding: &Binding,
        kind: &KindDef,
        body: impl FnOnce(&mut Self, &Env<'_>) -> Solved<T>,
    ) -> Solved<T> {
        let kind_env = KindEnv::of_binding(binding);
        let Some(ctx) = self.module_ctx(DefId::Kind(binding.kind)) else {
            return Err(Fault::Internal("the file an item is declared in").into());
        };
        let env = Env {
            ctx,
            file: kind.decl.file,
            kind: Some(&kind_env),
            array: None,
        };
        body(self, &env)
    }

    /// A cylinder's `tilt:`: `None` for 0deg (a straight cylinder);
    /// more than -90deg and less than 90deg, or `error[type-mismatch]` at
    /// the tilt (grammar, "Shapes").
    fn cylinder_tilt(&mut self, expr: &RExpr, env: &Env<'_>) -> Solved<Option<Angle>> {
        let exact = self.kind_angle(expr, env)?;
        let angle = exact.to_nearest_angle().ok_or(Fault::Overflow)?;
        let right = Angle::from_deg(90);
        if angle >= right || angle <= Angle::ZERO.checked_sub(right).ok_or(Fault::Overflow)? {
            return Err(self
                .error(
                    Code::TypeMismatch,
                    format!("a cylinder can lean less than 90deg either way, found {angle}"),
                    env.file,
                    expr.span,
                    "write a tilt between -90deg and 90deg, such as 15deg",
                )
                .into());
        }
        Ok((angle != Angle::ZERO).then_some(angle))
    }

    /// The box in the kind's frame around a slanted cylinder: its axis
    /// turned about `at` from `axis` toward `toward` by `angle`, over
    /// `range` along the turned axis. Each length worked out from the
    /// angle is rounded once, to the nanometre.
    fn slanted_box(
        &mut self,
        axis: FrameAxis,
        toward: FrameAxis,
        angle: Angle,
        diameter: ExactLength,
        at: ExactPoint,
        range: (ExactLength, ExactLength),
    ) -> Solved<(FrameBox, Option<Round>, Option<Outline>)> {
        let (sin, cos) = angle.sin_cos();
        let round = crate::joints::round_length;
        let times = |value: Length, factor: i128| -> Solved<ExactLength> {
            Ok(value.scaled(factor).ok_or(Fault::Overflow)?.to_exact())
        };
        let (low, high) = range;
        let length = high.checked_sub(low).ok_or(Fault::Overflow)?;
        let middle = round(midpoint(low, high).ok_or(Fault::Overflow)?)?;
        let half_length = round(half(length)?)?;
        let radius = round(half(diameter)?)?;
        let magnitude = |value: i128| value.abs();
        // The middle of the axis, and how far the shape reaches from it.
        let shift_axis = times(middle, cos)?;
        let shift_toward = times(middle, sin)?;
        let reach_axis = times(half_length, magnitude(cos))?
            .checked_add(times(radius, magnitude(sin))?)
            .ok_or(Fault::Overflow)?;
        let reach_toward = times(half_length, magnitude(sin))?
            .checked_add(times(radius, magnitude(cos))?)
            .ok_or(Fault::Overflow)?;
        let reach_other = radius.to_exact();
        let coordinate = |point: &ExactPoint, on: FrameAxis| match on {
            FrameAxis::Along => point.along,
            FrameAxis::Across => point.across,
            FrameAxis::Depth => point.depth,
        };
        let range_on = |on: FrameAxis| -> Solved<(ExactLength, ExactLength)> {
            let (centre, reach) = if on == axis {
                (
                    coordinate(&at, on)
                        .checked_add(shift_axis)
                        .ok_or(Fault::Overflow)?,
                    reach_axis,
                )
            } else if on == toward {
                (
                    coordinate(&at, on)
                        .checked_add(shift_toward)
                        .ok_or(Fault::Overflow)?,
                    reach_toward,
                )
            } else {
                (coordinate(&at, on), reach_other)
            };
            Ok((
                centre.checked_sub(reach).ok_or(Fault::Overflow)?,
                centre.checked_add(reach).ok_or(Fault::Overflow)?,
            ))
        };
        let frame_box = FrameBox {
            along: range_on(FrameAxis::Along)?,
            across: range_on(FrameAxis::Across)?,
            depth: range_on(FrameAxis::Depth)?,
        };
        Ok((
            frame_box,
            Some(Round {
                axis,
                slant: Some(Slant {
                    toward,
                    angle,
                    diameter,
                    length,
                }),
            }),
            None,
        ))
    }

    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "takes only the values it names: any other, a new variant included, is a type error"
    )]
    pub(crate) fn kind_length(&mut self, expr: &RExpr, env: &Env<'_>) -> Solved<ExactLength> {
        match self.eval(expr, env)? {
            Value::Length(length) => Ok(length),
            _ => Err(Fault::Earlier.into()),
        }
    }

    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "takes only the values it names: any other, a new variant included, is a type error"
    )]
    pub(crate) fn kind_angle(&mut self, expr: &RExpr, env: &Env<'_>) -> Solved<ExactAngle> {
        match self.eval(expr, env)? {
            Value::Angle(angle) => Ok(angle),
            _ => Err(Fault::Earlier.into()),
        }
    }

    pub(crate) fn kind_point(&mut self, point: &KindPoint, env: &Env<'_>) -> Solved<ExactPoint> {
        Ok(ExactPoint {
            along: self.kind_length(&point.along, env)?,
            across: self.kind_length(&point.across, env)?,
            depth: self.kind_length(&point.depth, env)?,
        })
    }

    /// A range in the kind's frame, low end first.
    pub(crate) fn frame_range(
        &mut self,
        range: &RangeDef,
        env: &Env<'_>,
    ) -> Solved<(ExactLength, ExactLength)> {
        let (low, high) = match range {
            RangeDef::Centred(size) => {
                let half = half(self.kind_length(size, env)?)?;
                (half.checked_neg().ok_or(Fault::Overflow)?, half)
            }
            RangeDef::Between { low, high } => {
                (self.kind_length(low, env)?, self.kind_length(high, env)?)
            }
        };
        Ok((low.min(high), low.max(high)))
    }

    /// A box shape's three ranges.
    pub(crate) fn frame_box(&mut self, shape: &ShapeDef, env: &Env<'_>) -> Solved<FrameBox> {
        let ShapeDef::Box {
            along,
            across,
            depth,
        } = shape
        else {
            return Err(Fault::Internal("a box shape").into());
        };
        Ok(FrameBox {
            along: self.frame_range(along, env)?,
            across: self.frame_range(across, env)?,
            depth: self.frame_range(depth, env)?,
        })
    }

    fn hardware_siblings(&self, hardware: HardwareId) -> usize {
        let Some(inst) = self.model.hardware.get(hardware.index()) else {
            return 1;
        };
        match self.model.item(DefId::Hardware(inst.def), inst.ctx) {
            Some(ItemSlot::Hardware(slot)) => slot.as_slice().len(),
            _ => 1,
        }
    }
}

/// What moves a member: the part or region of the role it is `with:`
/// (following the members it rides on), or the member the kind's
/// mechanism places that it is (or rides on). `None` for a member nothing
/// moves.
fn member_carrier(
    kind: &KindDef,
    roles: &[(String, RoleBound)],
    body: BodyId,
) -> Option<MemberCarrier> {
    let mut current = body;
    // A member rides on at most every other member.
    for _ in 0..=kind.bodies.len() {
        match &kind.bodies.get(current.index())?.with {
            Carrier::Role(role) => {
                return roles.get(role.index()).map(|(_, bound)| match *bound {
                    RoleBound::Part(part) => MemberCarrier::Part(part),
                    RoleBound::Region(region) => MemberCarrier::Region(region),
                });
            }
            Carrier::Body(other) => current = *other,
            Carrier::Mechanism => {
                return kind
                    .mechanism()
                    .is_some()
                    .then_some(MemberCarrier::Link(current));
            }
            // A joint's member: never in a hardware kind.
            Carrier::Contact(_) => return None,
        }
    }
    None
}

/// A placed shape in the world, exactly.
pub(crate) fn world_shape(shape: &PlacedShape, frame: &Frame) -> Solved<WorldShape> {
    let along = frame.along.axis();
    let low = shape.frame.along.0;
    let point = |[across, depth]: [ExactLength; 2]| frame.world_point(low, across, depth);
    Ok(match (&shape.outline, shape.cylinder) {
        (Some(Outline::Prism(points)), _) => WorldShape::Prism {
            axis: along,
            points: points
                .iter()
                .map(|p| point(*p))
                .collect::<Solved<Vec<_>>>()?,
            bounds: shape.world,
        },
        (
            Some(Outline::Bar {
                from,
                to,
                thickness,
            }),
            _,
        ) => WorldShape::Bar {
            axis: along,
            from: point(*from)?,
            to: point(*to)?,
            thickness: *thickness,
            bounds: shape.world,
        },
        (
            None,
            Some(Round {
                axis,
                slant: Some(slant),
            }),
        ) => {
            let along = frame.direction(axis);
            let lean = frame.direction(slant.toward);
            let lean = if slant.angle < Angle::ZERO {
                lean.opposite()
            } else {
                lean
            };
            // Read toward the positive end of the axis it is turned from.
            let toward = if along.is_positive() {
                lean
            } else {
                lean.opposite()
            };
            let angle = if slant.angle < Angle::ZERO {
                Angle::ZERO
                    .checked_sub(slant.angle)
                    .ok_or(Fault::Overflow)?
            } else {
                slant.angle
            };
            WorldShape::Slanted {
                axis: along.axis(),
                toward,
                angle,
                diameter: slant.diameter,
                length: slant.length,
                bounds: shape.world,
            }
        }
        (None, Some(Round { axis, slant: None })) => WorldShape::Cylinder {
            axis: frame.direction(axis).axis(),
            bounds: shape.world,
        },
        (None, None) => WorldShape::Box(shape.world),
    })
}

/// `true` when two boxes touch over an area or overlap.
/// How a member is seated on its part (grammar, "Cuts stay in their
/// part"): what decides the face it must lie wholly on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Seat {
    /// A cut of the kind houses it: the face the cut is made from.
    Cut,
    /// Its own `mount` places it against its part: the face it lies
    /// against.
    Against,
}

/// Where a seated shape runs past its part across the face it is seated
/// on: the first world axis it does, how much of it the part holds there
/// and its whole size.
struct Overhang {
    axis: Axis,
    held: ExactLength,
    whole: ExactLength,
}

/// The first axis other than `normal` along which `shape` runs past
/// `part`, if any.
fn overhang(shape: &ExactBox, part: &ExactBox, normal: Axis) -> Option<Overhang> {
    Axis::ALL
        .into_iter()
        .filter(|axis| *axis != normal)
        .find(|axis| shape.min(*axis) < part.min(*axis) || shape.max(*axis) > part.max(*axis))
        .map(|axis| {
            let low = shape.min(axis).max(part.min(axis));
            let high = shape.max(axis).min(part.max(axis));
            let held = high
                .checked_sub(low)
                .unwrap_or(ExactLength::ZERO)
                .max(ExactLength::ZERO);
            Overhang {
                axis,
                held,
                whole: size_on(shape, axis),
            }
        })
}

/// A box's size along a world axis.
fn size_on(shape: &ExactBox, axis: Axis) -> ExactLength {
    shape
        .max(axis)
        .checked_sub(shape.min(axis))
        .unwrap_or(ExactLength::ZERO)
}

fn meets(a: &ExactBox, b: &ExactBox) -> bool {
    let mut strict = 0;
    for axis in Axis::ALL {
        let low = a.min(axis).max(b.min(axis));
        let high = a.max(axis).min(b.max(axis));
        if low > high {
            return false;
        }
        if low < high {
            strict += 1;
        }
    }
    strict >= 2
}

/// The face of a part a shape touches or enters (grammar, "Faces to
/// cut"): across the frame's depth first, then across the other axes; on
/// one axis, the face nearer the frame's origin when the shape passes
/// right through.
pub(crate) fn crossing_face(shape: &ExactBox, part: &ExactBox, frame: &Frame) -> Option<Face> {
    let depth = frame.depth.axis();
    let order = std::iter::once(depth).chain(Axis::ALL.into_iter().filter(|a| *a != depth));
    for axis in order {
        let area = Axis::ALL.into_iter().filter(|a| *a != axis).all(|other| {
            shape.min(other).max(part.min(other)) < shape.max(other).min(part.max(other))
        });
        if !area {
            continue;
        }
        let within = |at: ExactLength| at >= shape.min(axis) && at <= shape.max(axis);
        let origin = frame.origin_on(axis);
        let distance = |at: ExactLength| {
            let d = at.checked_sub(origin)?;
            if d < ExactLength::ZERO {
                d.checked_neg()
            } else {
                Some(d)
            }
        };
        let high = match (within(part.min(axis)), within(part.max(axis))) {
            (true, false) => false,
            (false, true) => true,
            (true, true) => distance(part.max(axis))? < distance(part.min(axis))?,
            (false, false) => continue,
        };
        return Face::ALL
            .into_iter()
            .find(|face| face.axis() == axis && face.is_positive() == high);
    }
    None
}

fn half(value: ExactLength) -> Solved<ExactLength> {
    value
        .checked_div(Rational::from(2i64))
        .ok_or(Failure::Fault(Fault::Overflow))
}

fn abs(value: ExactLength) -> Solved<ExactLength> {
    if value < ExactLength::ZERO {
        value.checked_neg().ok_or(Failure::Fault(Fault::Overflow))
    } else {
        Ok(value)
    }
}
