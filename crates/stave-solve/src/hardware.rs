//! Hardware: mounting a slide, hinge or pull on a face of its piece, the
//! cuts it makes and the motion it gives its assembly.

use stave_lang::ast::{BaseKind, Span};
use stave_lang::placement::{Axis, Direction, Face, PartAxis};
use stave_lang::units::{ExactAngle, ExactLength, Rational};
use stave_lang::{Code, Diagnostic, Label};
use stave_model::defs::DefId;
use stave_model::ids::{HardwareId, KindId, PartId, RegionId};
use stave_model::instance::ItemSlot;
use stave_model::solved::Orientation;
use stave_model::value::{Envelope, ExactBox, ModelRef, Value};
use stave_schema::kinds::{MountRule, OpenAxes, base};

use crate::eval::{Env, KindEnv, midpoint, mm};
use crate::joints::Binding;
use crate::place::Siblings;
use crate::{Memo, Reported, Solved, Solver};

/// Catalog data of a placed piece of hardware, exact.
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
    },
    Pull {
        centres: Option<ExactLength>,
        diameter: Option<ExactLength>,
        screw_diameter: ExactLength,
    },
}

/// A hole hardware bores in its piece.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Bore {
    /// Centre on the mount face, in world coordinates.
    pub centre: [ExactLength; 3],
    pub diameter: ExactLength,
    /// `None`: through.
    pub depth: Option<ExactLength>,
    pub count: u32,
    pub centres: Option<ExactLength>,
}

/// A mortise hardware is housed in (a butt hinge's leaf): a notch the size
/// of the member, cut into one face of a part.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Recess {
    pub part: PartId,
    /// The world face of the part the notch is cut into.
    pub face: Face,
    /// The region the member fills, in world coordinates.
    pub bounds: ExactBox,
    pub depth: ExactLength,
}

/// Which kind of hinge a hinge kind is.
#[derive(Debug, Clone, Copy, PartialEq)]
enum HingeStyle {
    /// A cup bored into the piece and a plate on `on`.
    Cup {
        diameter: ExactLength,
        depth: ExactLength,
        inset: ExactLength,
    },
    /// Two leaves on a pin, each housed in a mortise.
    Butt { leaf: Envelope },
}

/// The motion a piece of hardware gives its assembly.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum MotionExact {
    Turn {
        axis: Axis,
        through: [ExactLength; 3],
        free_side: Direction,
        swing_toward: Direction,
        angle: ExactAngle,
    },
    Travel {
        toward: Direction,
        travel: ExactLength,
    },
}

/// The member of a piece of hardware that is fixed to the carcass and stays
/// put when the assembly moves: a hinge's mounting plate, a slide's cabinet
/// member.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Member {
    pub envelope: Envelope,
    pub bounds: ExactBox,
    pub orientation: Orientation,
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
    /// The member on the piece (it moves with the piece's assembly): a
    /// hinge's cup, a slide's drawer member, the whole of a pull.
    pub envelope: Envelope,
    pub bounds: ExactBox,
    pub orientation: Orientation,
    /// The member fixed to the carcass, for hinges and slides.
    pub fixed: Option<Member>,
    pub data: Data,
    pub bore: Option<Bore>,
    /// Mortises the hardware is housed in.
    pub recesses: Vec<Recess>,
    pub motion: Option<MotionExact>,
    pub model: Option<ModelRef>,
}

pub(crate) fn world_index(axis: Axis) -> usize {
    match axis {
        Axis::Across => 0,
        Axis::Up => 1,
        Axis::Deep => 2,
    }
}

fn positive_direction(axis: Axis) -> Direction {
    match axis {
        Axis::Up => Direction::Up,
        Axis::Across => Direction::Right,
        Axis::Deep => Direction::Back,
    }
}

impl Solver<'_> {
    fn hardware_entry(
        &mut self,
        binding: &Binding,
        name: &str,
        env: &Env<'_>,
        span: Span,
    ) -> Solved<Option<Value>> {
        if let Some((_, value)) = binding.settings.iter().find(|(n, _)| n == name) {
            return Ok(Some(value.clone()));
        }
        let Some(kind) = self.model.defs.kind(binding.kind).cloned() else {
            return Err(Reported);
        };
        let kind_env = KindEnv::new(
            binding.kind,
            binding.roles.clone(),
            binding.settings.clone(),
            kind.params.len(),
        );
        if let Some(index) = kind.param(name) {
            return self.kind_param(&kind_env, index, env.file, span).map(Some);
        }
        if let Some(field) = kind.field(name) {
            let Some(ctx) = self.module_ctx(DefId::Kind(binding.kind)) else {
                return Err(Reported);
            };
            let kind_eval = Env {
                ctx,
                file: kind.decl.file,
                kind: Some(&kind_env),
                array: None,
            };
            return self.eval(&field.value, &kind_eval).map(Some);
        }
        Ok(None)
    }

    fn hardware_length(
        &mut self,
        binding: &Binding,
        name: &str,
        env: &Env<'_>,
        span: Span,
    ) -> Solved<Option<ExactLength>> {
        match self.hardware_entry(binding, name, env, span)? {
            Some(Value::Length(length)) => Ok(Some(length)),
            Some(other) => Err(self.error(
                Code::UnitMismatch,
                format!("'{name}' needs a length, found {}", other.description()),
                env.file,
                span,
                "write a length",
            )),
            None => Ok(None),
        }
    }

    fn required_length(
        &mut self,
        binding: &Binding,
        name: &str,
        env: &Env<'_>,
        span: Span,
        kind_name: &str,
    ) -> Solved<ExactLength> {
        match self.hardware_length(binding, name, env, span)? {
            Some(length) => Ok(length),
            None => Err(self.error(
                Code::MissingField,
                format!("kind '{kind_name}' is missing '{name}'"),
                env.file,
                span,
                format!("give the kind {name}: ..."),
            )),
        }
    }

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
        let spec = base(kind.base);
        let Some(Value::Part(piece)) = binding.roles.first().cloned() else {
            return Err(Reported);
        };
        let piece_box = self.drawn_box(piece)?;
        let piece_orientation = self.part_orientation(piece)?;
        // A hinge's member on the piece is its cup or its leaf, sunk into
        // the mount face; every other base declares its envelope.
        let hinge_style = if kind.base == BaseKind::Hinge {
            match self.hardware_entry(&binding, "leaf", &env, span)? {
                Some(Value::Envelope(leaf @ Envelope::Box { .. })) => {
                    Some(HingeStyle::Butt { leaf })
                }
                Some(Value::Envelope(Envelope::Cylinder { .. })) => {
                    return Err(self.error(
                        Code::TypeMismatch,
                        format!("kind '{kind_name}': a hinge's leaf is a box"),
                        env.file,
                        span,
                        "write leaf: box(length: ..., width: ..., thickness: ...)",
                    ));
                }
                _ => Some(HingeStyle::Cup {
                    diameter: self.required_length(
                        &binding,
                        "cup_diameter",
                        &env,
                        span,
                        &kind_name,
                    )?,
                    depth: self.required_length(&binding, "cup_depth", &env, span, &kind_name)?,
                    inset: self.required_length(&binding, "cup_inset", &env, span, &kind_name)?,
                }),
            }
        } else {
            None
        };
        let envelope = match hinge_style {
            Some(HingeStyle::Cup {
                diameter, depth, ..
            }) => Envelope::Cylinder {
                diameter,
                length: depth,
            },
            Some(HingeStyle::Butt { leaf }) => leaf,
            None => match self.hardware_entry(&binding, "envelope", &env, span)? {
                Some(Value::Envelope(envelope)) => envelope,
                _ => {
                    return Err(self.error(
                        Code::MissingField,
                        format!("kind '{kind_name}' is missing 'envelope'"),
                        env.file,
                        span,
                        "give the kind envelope: box(...) or cylinder(...)",
                    ));
                }
            },
        };
        let model = match self.hardware_entry(&binding, "model", &env, span)? {
            Some(Value::Model(model)) => Some(model),
            _ => None,
        };
        // The mount face.
        let mount_face = match (binding.face, spec.mount) {
            (Some(face), _) => face,
            (None, Some(MountRule::Face(face))) => face,
            (None, Some(MountRule::NearerRegionSide)) => {
                let Some(Value::Region(region)) = binding.roles.get(1).cloned() else {
                    return Err(Reported);
                };
                let region_box = self.region_value(region)?.bounds;
                let left = piece_box
                    .min(Axis::Across)
                    .checked_sub(region_box.min(Axis::Across))
                    .ok_or(Reported)?;
                let right = region_box
                    .max(Axis::Across)
                    .checked_sub(piece_box.max(Axis::Across))
                    .ok_or(Reported)?;
                if abs(right)? < abs(left)? {
                    Face::Right
                } else {
                    Face::Left
                }
            }
            (None, None) => return Err(Reported),
        };
        // The envelope's frame on the face: its thickness stands off the
        // face, except a hinge cup's or leaf's, which runs into the piece.
        let mount_direction = mount_face.direction();
        let thickness_direction = if hinge_style.is_some() {
            mount_direction.opposite()
        } else {
            mount_direction
        };
        let piece_length = piece_orientation.direction(PartAxis::Length);
        let length_direction = if piece_length.axis() == thickness_direction.axis() {
            piece_orientation.direction(PartAxis::Width)
        } else {
            piece_length
        };
        let Some(width_axis) = length_direction.axis().third(thickness_direction.axis()) else {
            return Err(Reported);
        };
        let size = match envelope {
            Envelope::Box {
                length,
                width,
                thickness,
            } => [length, width, thickness],
            Envelope::Cylinder { diameter, length } => [diameter, diameter, length],
        };
        let [env_length, env_width, env_thickness] = size;
        let mut lows: [Option<ExactLength>; 3] = [None, None, None];
        let face_axis = thickness_direction.axis();
        let face_position = if mount_face.is_positive() {
            piece_box.max(face_axis)
        } else {
            piece_box.min(face_axis)
        };
        let face_low = if thickness_direction.is_positive() {
            face_position
        } else {
            face_position.checked_sub(env_thickness).ok_or(Reported)?
        };
        if let Some(slot) = lows.get_mut(world_index(face_axis)) {
            *slot = Some(face_low);
        }
        let mut width_direction = positive_direction(width_axis);
        let length_axis = length_direction.axis();
        let mut on_part = None;
        match kind.base {
            BaseKind::Slide => {
                let low = piece_box.min(length_axis);
                let centre = midpoint(piece_box.min(width_axis), piece_box.max(width_axis))
                    .ok_or(Reported)?;
                if let Some(slot) = lows.get_mut(world_index(length_axis)) {
                    *slot = Some(low);
                }
                if let Some(slot) = lows.get_mut(world_index(width_axis)) {
                    *slot = Some(centre.checked_sub(half(env_width)?).ok_or(Reported)?);
                }
            }
            BaseKind::Hinge => {
                let Some(Value::Part(on)) = binding.roles.get(1).cloned() else {
                    return Err(Reported);
                };
                on_part = Some(on);
                let on_box = self.drawn_box(on)?;
                let on_centre =
                    midpoint(on_box.min(width_axis), on_box.max(width_axis)).ok_or(Reported)?;
                let (low_edge, high_edge) = (piece_box.min(width_axis), piece_box.max(width_axis));
                let low_distance = abs(on_centre.checked_sub(low_edge).ok_or(Reported)?)?;
                let high_distance = abs(on_centre.checked_sub(high_edge).ok_or(Reported)?)?;
                let nearer_low = low_distance <= high_distance;
                // A cup is `cup_inset` in from the edge nearer `on`. A butt
                // hinge's leaves start at the pin edge: the one where the
                // piece is flush with a face of `on`.
                let (inset, at_low) = match hinge_style {
                    Some(HingeStyle::Cup { inset, .. }) => (inset, nearer_low),
                    Some(HingeStyle::Butt { .. }) => {
                        let flush = |edge: ExactLength| {
                            edge == on_box.min(width_axis) || edge == on_box.max(width_axis)
                        };
                        let at_low = match (flush(low_edge), flush(high_edge)) {
                            (true, false) => true,
                            (false, true) => false,
                            (true, true) | (false, false) => nearer_low,
                        };
                        (ExactLength::ZERO, at_low)
                    }
                    None => (ExactLength::ZERO, nearer_low),
                };
                let low = if at_low {
                    width_direction = positive_direction(width_axis);
                    low_edge.checked_add(inset).ok_or(Reported)?
                } else {
                    width_direction = positive_direction(width_axis).opposite();
                    high_edge
                        .checked_sub(inset)
                        .and_then(|edge| edge.checked_sub(env_width))
                        .ok_or(Reported)?
                };
                if let Some(slot) = lows.get_mut(world_index(width_axis)) {
                    *slot = Some(low);
                }
            }
            BaseKind::Pull | BaseKind::Joint | BaseKind::Fastener | BaseKind::Tool => {}
        }
        // The place { } terms for the axes the base leaves open.
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
        let open: Vec<Axis> = match spec.open_axes {
            Some(OpenAxes::None) | None => Vec::new(),
            Some(OpenAxes::AlongPieceLength) => vec![length_axis],
            Some(OpenAxes::Both) => vec![length_axis, width_axis],
        };
        let size_along = |axis: Axis| -> ExactLength {
            if axis == length_axis {
                env_length
            } else if axis == width_axis {
                env_width
            } else {
                env_thickness
            }
        };
        for axis in Axis::ALL {
            let term = terms.axis(axis).cloned();
            match (term, open.contains(&axis)) {
                (Some(term), false) => {
                    let why = if axis == face_axis {
                        "the mount face already fixes it"
                    } else {
                        "the hardware's base already fixes it"
                    };
                    self.report(
                        Diagnostic::error(
                            Code::AxisSetTwice,
                            format!("{name}: position along '{}' is set twice", axis.word()),
                            Label::new(env.file, term.span),
                        )
                        .with_help(format!("remove this term: {why}")),
                    );
                    return Err(Reported);
                }
                (Some(term), true) => {
                    let count = self.hardware_siblings(hardware);
                    let siblings = Siblings {
                        index: inst.ctx.element.unwrap_or(0),
                        sizes: vec![size_along(axis); count],
                        part: None,
                    };
                    let low =
                        self.term_low(&term, axis, size_along(axis), &env, Some(&siblings))?;
                    if let Some(slot) = lows.get_mut(world_index(axis)) {
                        *slot = Some(low);
                    }
                }
                (None, true) => {
                    let place_span = def.place_span.unwrap_or(def.decl.span);
                    return Err(self.axis_not_set(&name, axis, env.file, place_span));
                }
                (None, false) => {}
            }
        }
        let mut extents = Vec::new();
        for axis in [Axis::Across, Axis::Up, Axis::Deep] {
            let low = lows
                .get(world_index(axis))
                .copied()
                .flatten()
                .ok_or(Reported)?;
            extents.push((low, low.checked_add(size_along(axis)).ok_or(Reported)?));
        }
        let [across, up, deep] = extents.as_slice() else {
            return Err(Reported);
        };
        let bounds = ExactBox::new(*across, *up, *deep);
        let orientation = Orientation::new(length_direction, width_direction, thickness_direction)
            .map_err(|_| Reported)?;
        // Data, bores and motion.
        let centre_of = |axis: Axis| midpoint(bounds.min(axis), bounds.max(axis)).ok_or(Reported);
        let mut centre = [ExactLength::ZERO; 3];
        for axis in Axis::ALL {
            if let Some(slot) = centre.get_mut(world_index(axis)) {
                *slot = if axis == face_axis {
                    face_position
                } else {
                    centre_of(axis)?
                };
            }
        }
        let mut fixed = None;
        let mut recesses = Vec::new();
        let mut envelope = envelope;
        let mut bounds = bounds;
        let (data, bore, motion) = match kind.base {
            BaseKind::Slide => {
                // Two members, each half the thickness: the drawer member
                // against the piece, the cabinet member outside it.
                if let Envelope::Box {
                    length,
                    width,
                    thickness,
                } = envelope
                {
                    let half_thickness = half(thickness)?;
                    let middle =
                        midpoint(bounds.min(face_axis), bounds.max(face_axis)).ok_or(Reported)?;
                    let (near, far) = if mount_direction.is_positive() {
                        (
                            (bounds.min(face_axis), middle),
                            (middle, bounds.max(face_axis)),
                        )
                    } else {
                        (
                            (middle, bounds.max(face_axis)),
                            (bounds.min(face_axis), middle),
                        )
                    };
                    envelope = Envelope::Box {
                        length,
                        width,
                        thickness: half_thickness,
                    };
                    fixed = Some(Member {
                        envelope,
                        bounds: bounds.with(face_axis, far.0, far.1),
                        orientation,
                    });
                    bounds = bounds.with(face_axis, near.0, near.1);
                }
                let Some(Value::Region(opening)) = binding.roles.get(1).cloned() else {
                    return Err(Reported);
                };
                let travel = self.required_length(&binding, "travel", &env, span, &kind_name)?;
                let data = Data::Slide {
                    length: self.required_length(&binding, "length", &env, span, &kind_name)?,
                    clearance: self.required_length(
                        &binding,
                        "clearance",
                        &env,
                        span,
                        &kind_name,
                    )?,
                    travel,
                    opening,
                };
                (
                    data,
                    None,
                    Some(MotionExact::Travel {
                        toward: Direction::Front,
                        travel,
                    }),
                )
            }
            BaseKind::Hinge => {
                let on = on_part.ok_or(Reported)?;
                let angle = match self.hardware_entry(&binding, "opening_angle", &env, span)? {
                    Some(Value::Angle(angle)) => angle,
                    _ => {
                        return Err(self.error(
                            Code::MissingField,
                            format!("kind '{kind_name}' is missing 'opening_angle'"),
                            env.file,
                            span,
                            "give the kind opening_angle: ...",
                        ));
                    }
                };
                // The door's edge nearer `on` (a butt hinge's pin edge): the
                // cup or leaf is measured from it and the door turns about it.
                let edge = if width_direction.is_positive() {
                    piece_box.min(width_axis)
                } else {
                    piece_box.max(width_axis)
                };
                if let Some(HingeStyle::Butt { leaf }) = hinge_style {
                    let leaf = self.butt_hinge(
                        &name,
                        piece,
                        on,
                        mount_face,
                        face_position,
                        bounds,
                        &env,
                        span,
                        leaf,
                    )?;
                    let mut through = [ExactLength::ZERO; 3];
                    for axis in Axis::ALL {
                        if let Some(slot) = through.get_mut(world_index(axis)) {
                            *slot = if axis == face_axis {
                                face_position
                            } else if axis == width_axis {
                                edge
                            } else {
                                piece_box.min(axis)
                            };
                        }
                    }
                    let (fixed_bounds, depth) = leaf;
                    fixed = Some(Member {
                        envelope,
                        bounds: fixed_bounds,
                        orientation: Orientation::new(
                            length_direction,
                            width_direction,
                            mount_direction,
                        )
                        .map_err(|_| Reported)?,
                    });
                    recesses = vec![
                        Recess {
                            part: piece,
                            face: mount_face,
                            bounds,
                            depth,
                        },
                        Recess {
                            part: on,
                            face: mount_face.opposite(),
                            bounds: fixed_bounds,
                            depth,
                        },
                    ];
                    let motion = MotionExact::Turn {
                        axis: length_axis,
                        through,
                        free_side: width_direction,
                        swing_toward: mount_face.opposite().direction(),
                        angle,
                    };
                    (
                        Data::ButtHinge {
                            opening_angle: angle,
                            on,
                        },
                        None,
                        Some(motion),
                    )
                } else {
                    let Some(HingeStyle::Cup {
                        diameter: cup_diameter,
                        depth: cup_depth,
                        inset: cup_inset,
                    }) = hinge_style
                    else {
                        return Err(Reported);
                    };
                    let offset = cup_inset.checked_add(half(cup_diameter)?).ok_or(Reported)?;
                    let cup_centre = if width_direction.is_positive() {
                        edge.checked_add(offset)
                    } else {
                        edge.checked_sub(offset)
                    }
                    .ok_or(Reported)?;
                    let mut bore_centre = centre;
                    if let Some(slot) = bore_centre.get_mut(world_index(width_axis)) {
                        *slot = cup_centre;
                    }
                    let opposite_face = mount_face.opposite();
                    let opposite_position = if opposite_face.is_positive() {
                        piece_box.max(face_axis)
                    } else {
                        piece_box.min(face_axis)
                    };
                    let mut through = [ExactLength::ZERO; 3];
                    for axis in Axis::ALL {
                        if let Some(slot) = through.get_mut(world_index(axis)) {
                            *slot = if axis == face_axis {
                                opposite_position
                            } else if axis == width_axis {
                                edge
                            } else {
                                piece_box.min(axis)
                            };
                        }
                    }
                    fixed = Some(self.hinge_plate(
                        &binding,
                        &env,
                        span,
                        &kind_name,
                        on,
                        HingeFrame {
                            length_direction,
                            length_centre: centre_of(length_axis)?,
                            toward_door_middle: width_direction,
                            mount_direction,
                        },
                    )?);
                    let data = Data::Hinge {
                        opening_angle: angle,
                        cup_diameter,
                        cup_depth,
                        cup_inset,
                        on,
                    };
                    let bore = Bore {
                        centre: bore_centre,
                        diameter: cup_diameter,
                        depth: Some(cup_depth),
                        count: 1,
                        centres: None,
                    };
                    let motion = MotionExact::Turn {
                        axis: length_axis,
                        through,
                        free_side: width_direction,
                        swing_toward: opposite_face.direction(),
                        angle,
                    };
                    (data, Some(bore), Some(motion))
                }
            }
            BaseKind::Pull | BaseKind::Joint | BaseKind::Fastener | BaseKind::Tool => {
                let centres = self.hardware_length(&binding, "centres", &env, span)?;
                let diameter = self.hardware_length(&binding, "diameter", &env, span)?;
                let screw_diameter = self
                    .hardware_length(&binding, "screw_diameter", &env, span)?
                    .unwrap_or_else(|| mm(stave_schema::kinds::DEFAULT_SCREW_DIAMETER_NM));
                let bore = Bore {
                    centre,
                    diameter: screw_diameter,
                    depth: None,
                    count: if centres.is_some() { 2 } else { 1 },
                    centres,
                };
                (
                    Data::Pull {
                        centres,
                        diameter,
                        screw_diameter,
                    },
                    Some(bore),
                    None,
                )
            }
        };
        Ok(Placed {
            kind: binding.kind,
            kind_name: binding.name.clone(),
            base: kind.base,
            piece,
            mount_face,
            envelope,
            bounds,
            orientation,
            fixed,
            data,
            bore,
            recesses,
            motion,
            model,
        })
    }

    /// A butt hinge's second leaf: the moving leaf (`leaf_bounds`, sunk
    /// into the piece's mount face) reflected through the mount face's
    /// plane into `on`, which must lie against that face. Returns the
    /// fixed leaf's bounds and the depth of both mortises.
    #[allow(clippy::too_many_arguments)]
    fn butt_hinge(
        &mut self,
        name: &str,
        piece: PartId,
        on: PartId,
        mount_face: Face,
        face_position: ExactLength,
        leaf_bounds: ExactBox,
        env: &Env<'_>,
        span: Span,
        leaf: Envelope,
    ) -> Solved<(ExactBox, ExactLength)> {
        let Envelope::Box { thickness, .. } = leaf else {
            return Err(Reported);
        };
        let face_axis = mount_face.axis();
        let on_box = self.drawn_box(on)?;
        if on_box.min(face_axis) != face_position && on_box.max(face_axis) != face_position {
            let (piece_name, on_name) = (self.part_name(piece), self.part_name(on));
            return Err(self.error(
                Code::NoContact,
                format!("{name}: {piece_name} meets none of the parts it should join"),
                env.file,
                span,
                format!(
                    "a butt hinge's leaves meet where the {} face of {piece_name} lies against {on_name}; place them together, or name the face that does with face: ...",
                    mount_face.word()
                ),
            ));
        }
        let twice = face_position
            .checked_mul(Rational::from(2i64))
            .ok_or(Reported)?;
        let low = twice
            .checked_sub(leaf_bounds.max(face_axis))
            .ok_or(Reported)?;
        let high = twice
            .checked_sub(leaf_bounds.min(face_axis))
            .ok_or(Reported)?;
        Ok((leaf_bounds.with(face_axis, low, high), thickness))
    }

    /// A hinge's mounting plate: on the face of `on` that looks toward the
    /// middle of the door, centred on the cup along the door's length, its
    /// middle `plate_setback` back from the face of `on` the door closes
    /// against.
    fn hinge_plate(
        &mut self,
        binding: &Binding,
        env: &Env<'_>,
        span: Span,
        kind_name: &str,
        on: PartId,
        frame: HingeFrame,
    ) -> Solved<Member> {
        let setback = self.required_length(binding, "plate_setback", env, span, kind_name)?;
        let envelope = match self.hardware_entry(binding, "plate", env, span)? {
            Some(Value::Envelope(envelope @ Envelope::Box { .. })) => envelope,
            Some(Value::Envelope(Envelope::Cylinder { .. })) => {
                return Err(self.error(
                    Code::TypeMismatch,
                    format!("kind '{kind_name}': a hinge's plate is a box"),
                    env.file,
                    span,
                    "write plate: box(length: ..., width: ..., thickness: ...)",
                ));
            }
            _ => {
                return Err(self.error(
                    Code::MissingField,
                    format!("kind '{kind_name}' is missing 'plate'"),
                    env.file,
                    span,
                    "give the kind plate: box(length: ..., width: ..., thickness: ...)",
                ));
            }
        };
        let Envelope::Box {
            length,
            width,
            thickness,
        } = envelope
        else {
            return Err(Reported);
        };
        let on_box = self.drawn_box(on)?;
        let HingeFrame {
            length_direction,
            length_centre,
            toward_door_middle,
            mount_direction,
        } = frame;
        let span_from = |start: ExactLength, size: ExactLength, positive: bool| {
            if positive {
                start.checked_add(size).map(|end| (start, end))
            } else {
                start.checked_sub(size).map(|end| (end, start))
            }
        };
        // Across the door: standing off the inside face of `on`.
        let inside_axis = toward_door_middle.axis();
        let inside_face = if toward_door_middle.is_positive() {
            on_box.max(inside_axis)
        } else {
            on_box.min(inside_axis)
        };
        let across =
            span_from(inside_face, thickness, toward_door_middle.is_positive()).ok_or(Reported)?;
        // Along the door's length: centred on the cup.
        let along = (
            length_centre.checked_sub(half(length)?).ok_or(Reported)?,
            length_centre.checked_add(half(length)?).ok_or(Reported)?,
        );
        // Back from the face of `on` the door closes against.
        let depth_axis = mount_direction.axis();
        let closing_face = if mount_direction.is_positive() {
            on_box.min(depth_axis)
        } else {
            on_box.max(depth_axis)
        };
        let middle = if mount_direction.is_positive() {
            closing_face.checked_add(setback)
        } else {
            closing_face.checked_sub(setback)
        }
        .ok_or(Reported)?;
        let back = (
            middle.checked_sub(half(width)?).ok_or(Reported)?,
            middle.checked_add(half(width)?).ok_or(Reported)?,
        );
        let mut extents = [(ExactLength::ZERO, ExactLength::ZERO); 3];
        for (axis, extent) in [
            (inside_axis, across),
            (length_direction.axis(), along),
            (depth_axis, back),
        ] {
            if let Some(slot) = extents.get_mut(world_index(axis)) {
                *slot = extent;
            }
        }
        let [x, y, z] = extents;
        Ok(Member {
            envelope,
            bounds: ExactBox::new(x, y, z),
            orientation: Orientation::new(length_direction, mount_direction, toward_door_middle)
                .map_err(|_| Reported)?,
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

/// Where a hinge's plate goes, from the placed cup.
struct HingeFrame {
    length_direction: Direction,
    /// The cup's centre along the door's length.
    length_centre: ExactLength,
    /// From the hinge edge toward the middle of the door.
    toward_door_middle: Direction,
    /// The mount face's outward direction (out of the door's back).
    mount_direction: Direction,
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
