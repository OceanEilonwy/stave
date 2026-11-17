//! Hardware: mounting a slide, hinge or pull on a face of its piece where
//! its kind's `mount { }` and the design's `place { }` say, placing its
//! members' shapes, the cuts its kind makes and the motion it gives its
//! assembly. Nothing here knows a base or a member by name.

use std::collections::BTreeMap;

use stave_lang::ast::{BaseKind, Span};
use stave_lang::placement::{Axis, Direction, Face, PartAxis};
use stave_lang::units::Length;
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

use crate::eval::{Env, KindEnv, midpoint};
use crate::joints::Binding;
use crate::mechanism::{ExactPoint, HardwareMechanism};
use crate::place::Siblings;
use crate::rules::{AdjustSetting, Call, Offset};
use crate::{Memo, Reported, Solved, Solver};

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
    pub origin: [ExactLength; 3],
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
        self.origin
            .get(world_index(axis))
            .copied()
            .unwrap_or(ExactLength::ZERO)
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
                origin.checked_add(low).ok_or(Reported)?,
                origin.checked_add(high).ok_or(Reported)?,
            )
        } else {
            (
                origin.checked_sub(high).ok_or(Reported)?,
                origin.checked_sub(low).ok_or(Reported)?,
            )
        })
    }

    /// The frame coordinate of a world plane across a frame axis.
    fn coordinate(&self, axis: FrameAxis, world: ExactLength) -> Solved<ExactLength> {
        let direction = self.direction(axis);
        let offset = world
            .checked_sub(self.origin_on(direction.axis()))
            .ok_or(Reported)?;
        if direction.is_positive() {
            Ok(offset)
        } else {
            offset.checked_neg().ok_or(Reported)
        }
    }

    /// A point in the frame, in the world.
    pub fn world_point(
        &self,
        along: ExactLength,
        across: ExactLength,
        depth: ExactLength,
    ) -> Solved<[ExactLength; 3]> {
        let mut out = [ExactLength::ZERO; 3];
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
            .ok_or(Reported)?;
            if let Some(slot) = out.get_mut(world_index(direction.axis())) {
                *slot = world;
            }
        }
        Ok(out)
    }

    /// A box in the frame, in the world.
    pub fn world_box(&self, frame_box: &FrameBox) -> Solved<ExactBox> {
        let mut extents = [(ExactLength::ZERO, ExactLength::ZERO); 3];
        for (axis, range) in [
            (FrameAxis::Along, frame_box.along),
            (FrameAxis::Across, frame_box.across),
            (FrameAxis::Depth, frame_box.depth),
        ] {
            let extent = self.world_extent(axis, range)?;
            if let Some(slot) = extents.get_mut(world_index(self.direction(axis).axis())) {
                *slot = extent;
            }
        }
        let [x, y, z] = extents;
        Ok(ExactBox::new(x, y, z))
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
    /// The frame axis a cylinder's axis runs along.
    pub cylinder: Option<FrameAxis>,
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
                a.checked_add(across).ok_or(Reported)?,
                d.checked_add(depth).ok_or(Reported)?,
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
    /// A convex outline across `axis`, extruded over `bounds` along it;
    /// each point is a world point (its coordinate on `axis` carries no
    /// meaning).
    Prism {
        axis: Axis,
        points: Vec<[ExactLength; 3]>,
        bounds: ExactBox,
    },
    /// A bar across `axis` between two world points, over `bounds` along
    /// it.
    Bar {
        axis: Axis,
        from: [ExactLength; 3],
        to: [ExactLength; 3],
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
    pub kind: CutKind,
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

pub(crate) fn world_index(axis: Axis) -> usize {
    match axis {
        Axis::Across => 0,
        Axis::Up => 1,
        Axis::Deep => 2,
    }
}

pub(crate) fn positive_direction(axis: Axis) -> Direction {
    match axis {
        Axis::Up => Direction::Up,
        Axis::Across => Direction::Right,
        Axis::Deep => Direction::Back,
    }
}

/// Each shape of a member in the kind's frame, with the axis of a cylinder
/// and the outline of a prism or a bar.
pub(crate) type FrameShapes = Vec<(FrameBox, Option<FrameAxis>, Option<Outline>)>;

/// The world plane an anchor names, and which way is into the part it
/// belongs to.
#[derive(Debug, Clone, Copy)]
struct Plane {
    at: ExactLength,
    /// `true` when the part lies on the positive side of the plane.
    inward_positive: bool,
}

impl Solver<'_> {
    /// Places a piece of hardware (once).
    pub(crate) fn place_hardware(&mut self, hardware: HardwareId) -> Solved<Placed> {
        match self.hardware.get(hardware.index()) {
            Some(Memo::Done(placed)) => return Ok(placed.clone()),
            Some(Memo::Failed | Memo::Busy) => return Err(Reported),
            _ => {}
        }
        if let Some(slot) = self.hardware.get_mut(hardware.index()) {
            *slot = Memo::Busy;
        }
        let result = self.compute_hardware(hardware);
        if let Some(slot) = self.hardware.get_mut(hardware.index()) {
            *slot = match &result {
                Ok(placed) => Memo::Done(placed.clone()),
                Err(_) => Memo::Failed,
            };
        }
        result
    }

    fn compute_hardware(&mut self, hardware: HardwareId) -> Solved<Placed> {
        if let Some(inst) = self.model.hardware.get(hardware.index())
            && self.poisoned(DefId::Hardware(inst.def))
        {
            return Err(self.skip());
        }
        let Some(inst) = self.model.hardware.get(hardware.index()).cloned() else {
            return Err(Reported);
        };
        let Some(def) = self.model.defs.hardware_def(inst.def).cloned() else {
            return Err(Reported);
        };
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
        let name = inst.name.clone();
        let span = def.call.span;
        let binding = self.bind_call(&def.call, &env, &format!("hardware '{name}'"))?;
        let Some(kind) = self.model.defs.kind(binding.kind).cloned() else {
            return Err(Reported);
        };
        let kind_name = binding.name.clone();
        if !kind.base.is_mounted_hardware() {
            return Err(self.error(
                Code::TypeMismatch,
                format!(
                    "'{name}' needs a slide, hinge or pull kind, found a {} kind",
                    kind.base.word()
                ),
                env.file,
                def.call.kind.span,
                "mount hardware with a kind of base slide, hinge or pull",
            ));
        }
        // Reported where the kind is resolved.
        let Some(hardware_kind) = kind.hardware.clone() else {
            return Err(Reported);
        };
        let roles = self.hardware_roles(&kind, &binding)?;
        let entries = self.hardware_entries(&binding, &kind, env.file, span)?;
        let fasteners = self.hardware_fasteners(&binding, &kind, &hardware_kind.uses)?;
        let model = match entries.get("model") {
            Some(Value::Model(model)) => {
                let declared = self.declared_shapes(&binding, &kind)?;
                Some(self.hardware_model(&binding, &kind, model, &declared)?)
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
        self.check_contact(&name, &kind, &roles, &bodies, piece, env.file, span)?;
        let cuts = self.hardware_cuts(
            &binding,
            &kind,
            &hardware_kind.cuts,
            &roles,
            &bodies,
            &frame,
            &name,
            env.file,
            span,
        )?;
        // The kind's mechanism moves it, counted on its own first.
        let (mechanism, own) = match &kind.mechanism {
            Some(def) => {
                let mechanism = self.kind_mechanism(&binding, &kind, &kind_name, def, &frame)?;
                let own = self.check_mechanism(hardware, &roles, &mechanism, &kind_name)?;
                (Some(mechanism), own)
            }
            None => (None, None),
        };
        let members = self.placed_members(&kind, &roles, &bodies, &frame)?;
        // The kind's rules where it is placed: its adjustments and requires.
        let (settings, rules) = self.hardware_rules(
            Call {
                hardware,
                name: &name,
                file: env.file,
                span,
            },
            &binding,
            &kind,
            &hardware_kind.adjusts,
            &offsets,
            mount_anchor,
        )?;
        Ok(Placed {
            kind: binding.kind,
            kind_name: binding.name.clone(),
            base: kind.base,
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
            let id = BodyId::new(index).ok_or(Reported)?;
            let shapes: Vec<ShapeDef> = body.shapes.iter().map(|s| s.shape.clone()).collect();
            let boxes = self.in_kind(binding, kind, |solver, kind_env| {
                shapes
                    .iter()
                    .map(|shape| solver.shape_box(shape, kind_env))
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
        let Some(inst) = self.model.hardware.get(hardware.index()).cloned() else {
            return Err(Reported);
        };
        let Some(def) = self.model.defs.hardware_def(inst.def).cloned() else {
            return Err(Reported);
        };
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
        let name = inst.name.clone();
        let Some(kind) = self.model.defs.kind(binding.kind).cloned() else {
            return Err(Reported);
        };
        let kind_name = binding.name.clone();
        let Some(hardware_kind) = kind.hardware.clone() else {
            return Err(Reported);
        };
        let Some(Value::Part(piece)) = binding.roles.first().cloned() else {
            return Err(Reported);
        };
        let piece_box = self.drawn_box(piece)?;
        let piece_orientation = self.part_orientation(piece)?;
        // The mount face fixes the depth axis: the origin lies on it.
        let mount = &hardware_kind.mount;
        let mount_face = match (binding.face, mount.face) {
            (Some(face), _) => face,
            (None, FaceRule::Word(face)) => face,
            (None, FaceRule::Nearer { part, region }) => self.face_nearer(binding, part, region)?,
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
            return Err(Reported);
        };
        let mut frame = Frame {
            origin: [ExactLength::ZERO; 3],
            along: length_direction,
            across: positive_direction(across_axis),
            depth: mount_face.direction().opposite(),
            across_edge: None,
        };
        if let Some(slot) = frame.origin.get_mut(world_index(face_axis)) {
            *slot = face_position;
        }
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
                    let centre =
                        midpoint(part_box.min(world), part_box.max(world)).ok_or(Reported)?;
                    if let Some(slot) = frame.origin.get_mut(world_index(world)) {
                        *slot = centre;
                    }
                }
                AxisRule::Term(MountTerm::From(from)) => {
                    let (plane, at) = self.term_plane(binding, &kind, mount_face, world, from)?;
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
                    if let Some(slot) = frame.origin.get_mut(world_index(world)) {
                        *slot = at;
                    }
                }
            }
        }
        // Each member's shapes in the frame, before the frame is placed on
        // the open axes.
        let frame_bodies = self.declared_shapes(binding, &kind)?;
        let label = inst.ctx.element.and_then(|element| {
            match self.model.defs.array(DefId::Hardware(inst.def)) {
                Some(stave_model::defs::ArrayDef::Labels(labels)) => {
                    labels.get(element).map(|(l, _)| l.clone())
                }
                Some(stave_model::defs::ArrayDef::Count(_)) => Some(element.to_string()),
                None => None,
            }
        });
        let terms = self.sort_terms(&def.place, &env, &name, label.as_deref(), true)?;
        let piece_role = RoleIndex::new(0).ok_or(Reported)?;
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
            let term = terms.axis(world).cloned();
            let frame_axis = [FrameAxis::Along, FrameAxis::Across, FrameAxis::Depth]
                .into_iter()
                .find(|axis| frame.direction(*axis).axis() == world)
                .ok_or(Reported)?;
            match (term, open.contains(&frame_axis)) {
                (Some(term), false) => {
                    let why = if world == face_axis {
                        "the mount face already fixes it"
                    } else {
                        "the hardware's kind already fixes it"
                    };
                    self.report(
                        Diagnostic::error(
                            Code::AxisSetTwice,
                            format!("{name}: position along '{}' is set twice", world.word()),
                            Label::new(env.file, term.span),
                        )
                        .with_help(format!("remove this term: {why}")),
                    );
                    return Err(Reported);
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
                        ));
                    };
                    let (low, high) = carried.range(frame_axis);
                    let size = high.checked_sub(low).ok_or(Reported)?;
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
                    .ok_or(Reported)?;
                    if let Some(slot) = frame.origin.get_mut(world_index(world)) {
                        *slot = origin;
                    }
                }
                (None, true) => {
                    let place_span = def.place_span.unwrap_or(def.decl.span);
                    return Err(self.axis_not_set(&name, world, env.file, place_span));
                }
                (None, false) => {}
            }
        }
        // Every member at rest: its own mount, then the frame.
        let mut all_offsets = Vec::new();
        let mut bodies = Vec::new();
        for (id, boxes) in &frame_bodies {
            let Some(body) = kind.bodies.get(id.index()) else {
                return Err(Reported);
            };
            let mut offsets: Vec<(FrameAxis, ExactLength)> = Vec::new();
            for placement in &body.mount {
                let world = frame.direction(placement.axis).axis();
                let (_, at) =
                    self.term_plane(binding, &kind, mount_face, world, &placement.from)?;
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
                        .ok_or(Reported)
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
                            low.checked_add(*offset).ok_or(Reported)?,
                            high.checked_add(*offset).ok_or(Reported)?,
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
            let body = kind.bodies.get(placed.body.index()).ok_or(Reported)?;
            let Some(carrier) = member_carrier(kind, roles, placed.body) else {
                return Err(self.error(
                    Code::MissingField,
                    format!(
                        "member '{}' has no 'with:' and nothing in its kind moves it",
                        body.name.as_str()
                    ),
                    kind.decl.file,
                    body.span,
                    "give it with: a role or another member, or pin it in the kind's mechanism",
                ));
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
        let uses = uses.to_vec();
        self.in_kind(binding, kind, |solver, kind_env| {
            let mut out = Vec::new();
            for used in &uses {
                let Value::Kind(fastener) = solver.eval(&used.fastener, kind_env)? else {
                    return Err(Reported);
                };
                let count = match &used.count {
                    Some(count) => solver.eval_count(count, kind_env)?,
                    None => 1,
                };
                let mut size = |name: &str| -> Solved<Length> {
                    match solver.kind_entry(&fastener, name, file, used.span)? {
                        Value::Length(length) => length.to_nearest_length().ok_or(Reported),
                        _ => Err(Reported),
                    }
                };
                let diameter = size("diameter")?;
                let length = size("length")?;
                let name = solver
                    .model
                    .defs
                    .kind(fastener.kind)
                    .map(|k| k.decl.name.clone())
                    .unwrap_or_default();
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
                _ => Err(Reported),
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
        let kind_env = KindEnv::new(
            binding.kind,
            binding.roles.clone(),
            binding.settings.clone(),
            kind.params.len(),
        );
        let mut entries = BTreeMap::new();
        for (index, param) in kind.params.iter().enumerate() {
            let value = self.kind_param(&kind_env, index, file, span)?;
            entries.insert(param.decl.name.clone(), value);
        }
        let Some(ctx) = self.module_ctx(DefId::Kind(binding.kind)) else {
            return Err(Reported);
        };
        let kind_eval = Env {
            ctx,
            file: kind.decl.file,
            kind: Some(&kind_env),
            array: None,
        };
        for field in &kind.fields {
            let value = self.eval(&field.value, &kind_eval)?;
            entries.insert(field.name.clone(), value);
        }
        Ok(entries)
    }

    /// The part a role of the hardware is bound to.
    fn role_part(&mut self, binding: &Binding, role: RoleIndex) -> Solved<PartId> {
        match binding.roles.get(role.index()) {
            Some(Value::Part(part)) => Ok(*part),
            _ => Err(Reported),
        }
    }

    /// The box of the part or region a role is bound to.
    fn role_box(&mut self, binding: &Binding, role: RoleIndex) -> Solved<ExactBox> {
        match binding.roles.get(role.index()).cloned() {
            Some(Value::Part(part)) => self.drawn_box(part),
            Some(Value::Region(region)) => Ok(self.region_value(region)?.bounds),
            _ => Err(Reported),
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
            .ok_or(Reported)?;
        let high = region_box
            .max(axis)
            .checked_sub(part_box.max(axis))
            .ok_or(Reported)?;
        let positive = abs(high)? < abs(low)?;
        Face::ALL
            .into_iter()
            .find(|face| face.axis() == axis && face.is_positive() == positive)
            .ok_or(Reported)
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
            Some(inset) => {
                let inset = inset.clone();
                self.in_kind(binding, kind, |solver, kind_env| {
                    solver.kind_length(&inset, kind_env)
                })?
            }
            None => ExactLength::ZERO,
        };
        let at = if plane.inward_positive {
            plane.at.checked_add(inset)
        } else {
            plane.at.checked_sub(inset)
        }
        .ok_or(Reported)?;
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
        let middle = |b: &ExactBox| midpoint(b.min(axis), b.max(axis)).ok_or(Reported);
        let nearer_high = |part_box: &ExactBox, target: ExactLength| -> Solved<bool> {
            let low = abs(target.checked_sub(part_box.min(axis)).ok_or(Reported)?)?;
            let high = abs(target.checked_sub(part_box.max(axis)).ok_or(Reported)?)?;
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
                ));
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
                    ));
                }
                if toward.axis() != axis {
                    return Err(self.error(
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
                    ));
                }
                let part_box = self.drawn_box(part_id)?;
                Ok(side(&part_box, toward.is_positive()))
            }
            MountSelector::FaceToward { part, other } => {
                let part_box = self.role_box(binding, part)?;
                let target = middle(&self.role_box(binding, other)?)?;
                Ok(side(&part_box, nearer_high(&part_box, target)?))
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
    ) -> Solved<(FrameBox, Option<FrameAxis>, Option<Outline>)> {
        match shape {
            ShapeDef::Box { .. } => Ok((self.frame_box(shape, env)?, None, None)),
            ShapeDef::Cylinder {
                diameter,
                extent,
                at,
                axis,
            } => {
                let radius = half(self.kind_length(diameter, env)?)?;
                let centre = self.kind_point(at, env)?;
                let range = self.frame_range(extent, env)?;
                let around = |value: ExactLength| -> Solved<(ExactLength, ExactLength)> {
                    Ok((
                        value.checked_sub(radius).ok_or(Reported)?,
                        value.checked_add(radius).ok_or(Reported)?,
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
                Ok((frame_box, Some(*axis), None))
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
                        _ => Err(Reported),
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
                        a.min(b).checked_sub(grow).ok_or(Reported)?,
                        a.max(b).checked_add(grow).ok_or(Reported)?,
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
            let role = RoleIndex::new(index).ok_or(Reported)?;
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
                ));
            }
        }
        Ok(())
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
                return Err(Reported);
            };
            let Some(placed) = bodies.iter().find(|placed| placed.body == cut.body) else {
                return Err(Reported);
            };
            let operations = cut.operations.clone();
            let kinds = self.in_kind(binding, kind, |solver, kind_env| {
                operations
                    .iter()
                    .map(|operation| solver.cut_kind(operation, kind_env))
                    .collect::<Solved<Vec<_>>>()
            })?;
            let part_box = self.drawn_box(part)?;
            for shape in &placed.shapes {
                let Some(face) = crossing_face(&shape.world, &part_box, frame) else {
                    let part_name = self.part_name(part);
                    let member = kind
                        .bodies
                        .get(cut.body.index())
                        .map(|body| body.name.as_str().to_owned())
                        .unwrap_or_default();
                    return Err(self.error(
                        Code::NoContact,
                        format!("{name}: {member} meets none of the parts it should join"),
                        file,
                        span,
                        format!(
                            "its {member} must touch or enter {part_name} for the kind's cut there"
                        ),
                    ));
                };
                for kind_of_cut in &kinds {
                    out.push(HardwareCut {
                        part,
                        body: cut.body,
                        name: cut.name.clone(),
                        face,
                        shape: shape.world,
                        kind: kind_of_cut.clone(),
                    });
                }
            }
        }
        Ok(out)
    }

    /// The environment a hardware kind's own expressions are evaluated
    /// in, for one piece of hardware, handed to `body`.
    pub(crate) fn in_kind<T>(
        &mut self,
        binding: &Binding,
        kind: &KindDef,
        body: impl FnOnce(&mut Self, &Env<'_>) -> Solved<T>,
    ) -> Solved<T> {
        let kind_env = KindEnv::new(
            binding.kind,
            binding.roles.clone(),
            binding.settings.clone(),
            kind.params.len(),
        );
        let Some(ctx) = self.module_ctx(DefId::Kind(binding.kind)) else {
            return Err(Reported);
        };
        let env = Env {
            ctx,
            file: kind.decl.file,
            kind: Some(&kind_env),
            array: None,
        };
        body(self, &env)
    }

    pub(crate) fn kind_length(&mut self, expr: &RExpr, env: &Env<'_>) -> Solved<ExactLength> {
        match self.eval(expr, env)? {
            Value::Length(length) => Ok(length),
            _ => Err(Reported),
        }
    }

    pub(crate) fn kind_angle(&mut self, expr: &RExpr, env: &Env<'_>) -> Solved<ExactAngle> {
        match self.eval(expr, env)? {
            Value::Angle(angle) => Ok(angle),
            _ => Err(Reported),
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
                (half.checked_neg().ok_or(Reported)?, half)
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
            return Err(Reported);
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
            Some(ItemSlot::Hardware(slot)) => slot.all().len(),
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
                    .mechanism
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
        (None, Some(axis)) => WorldShape::Cylinder {
            axis: frame.direction(axis).axis(),
            bounds: shape.world,
        },
        (None, None) => WorldShape::Box(shape.world),
    })
}

/// `true` when two boxes touch over an area or overlap.
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
    value.checked_div(Rational::from(2i64)).ok_or(Reported)
}

fn abs(value: ExactLength) -> Solved<ExactLength> {
    if value < ExactLength::ZERO {
        value.checked_neg().ok_or(Reported)
    } else {
        Ok(value)
    }
}
