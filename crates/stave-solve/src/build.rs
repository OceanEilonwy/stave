//! Putting it together: runs every step in order and writes the
//! [`SolvedDesign`].

use std::collections::{BTreeMap, HashMap};

use stave_lang::placement::{Axis, PartAxis};
use stave_lang::units::{ExactLength, Length};
use stave_lang::{Code, Diagnostic, Label};
use stave_model::defs::{BodyId, DefId, Word};
use stave_model::ids::{AssemblyId, HardwareId, JointId, PartId, RegionId, SceneId, StockId};
use stave_model::instance::Ctx;
use stave_model::mechanism::MemberIndex;
use stave_model::solved::{
    self, Assembly, Cut, CutPlace, CutSource, FastenerUse, Hardware, HardwareBase, HardwareData,
    Joint, JointPair, LocalBox, LocalPoint, PartOrigin, Point, PoseValue, ProjectInfo, Region,
    RequireResult, RoleBinding, Scene, SceneItem, ScenePose, Shape, Size3, SolvedDesign, Station,
    Stock, Tool,
};
use stave_model::value::{Envelope as ExactEnvelope, ExactBox, Value};
use stave_schema::words::{Grain, MotionWord, StockForm};

use crate::eval::{Env, midpoint};
use crate::hardware::{
    HardwareCut, MemberCarrier, Placed, PlacedMember, RoleBound, WorldShape, world_index,
};
use crate::joints::{GeomTable, JointWork, PartGeom, local_slot, round_length};
use crate::legacy::{CupParts, Data, LegacyMotion, legacy_assembly_motion};
use crate::mechanism::AssemblyMechanism;
use crate::place::reflect;
use crate::{Reported, Solved, Solver};

/// A part a joint made.
pub(crate) struct MadeInfo {
    /// Which joint and pair, or which part's boards, made it.
    pub origin: PartOrigin,
    /// The assembly it belongs to: its joint's, or its part's.
    pub assembly: Option<AssemblyId>,
    pub name: String,
    pub stock: StockId,
    pub note: Option<String>,
}

fn point(values: [ExactLength; 3]) -> Solved<Point> {
    let [across, up, deep] = values;
    Ok(Point {
        across: round_length(across)?,
        up: round_length(up)?,
        deep: round_length(deep)?,
    })
}

fn world_box(bounds: &ExactBox) -> Solved<solved::WorldBox> {
    let low = point([
        bounds.min(Axis::Across),
        bounds.min(Axis::Up),
        bounds.min(Axis::Deep),
    ])?;
    let high = point([
        bounds.max(Axis::Across),
        bounds.max(Axis::Up),
        bounds.max(Axis::Deep),
    ])?;
    Ok(solved::WorldBox::new(low, high))
}

/// A world point reflected across the plane at `plane` on `axis`.
fn reflect_point(
    point: [ExactLength; 3],
    axis: Axis,
    plane: ExactLength,
) -> Solved<[ExactLength; 3]> {
    let mut out = point;
    if let Some(slot) = out.get_mut(world_index(axis)) {
        let twice = plane.checked_add(plane).ok_or(Reported)?;
        *slot = twice.checked_sub(*slot).ok_or(Reported)?;
    }
    Ok(out)
}

/// A piece of hardware's members reflected, each carried by what the
/// reflected roles bind (the links stay links).
fn reflect_members(
    members: &[PlacedMember],
    source_roles: &[(String, RoleBound)],
    roles: &[(String, RoleBound)],
    axis: Axis,
    plane: ExactLength,
) -> Solved<Vec<PlacedMember>> {
    let rebound = |carrier: MemberCarrier| -> Solved<MemberCarrier> {
        let was = match carrier {
            MemberCarrier::Part(part) => RoleBound::Part(part),
            MemberCarrier::Region(region) => RoleBound::Region(region),
            MemberCarrier::Link(_) => return Ok(carrier),
        };
        let index = source_roles
            .iter()
            .position(|(_, bound)| *bound == was)
            .ok_or(Reported)?;
        match roles.get(index).map(|(_, bound)| *bound) {
            Some(RoleBound::Part(part)) => Ok(MemberCarrier::Part(part)),
            Some(RoleBound::Region(region)) => Ok(MemberCarrier::Region(region)),
            None => Err(Reported),
        }
    };
    let shape = |shape: &WorldShape| -> Solved<WorldShape> {
        Ok(match shape {
            WorldShape::Box(bounds) => WorldShape::Box(reflect_box(bounds, axis, plane)?),
            WorldShape::Cylinder {
                axis: along,
                bounds,
            } => WorldShape::Cylinder {
                axis: *along,
                bounds: reflect_box(bounds, axis, plane)?,
            },
            WorldShape::Prism {
                axis: along,
                points,
                bounds,
            } => WorldShape::Prism {
                axis: *along,
                points: points
                    .iter()
                    .map(|point| reflect_point(*point, axis, plane))
                    .collect::<Solved<Vec<_>>>()?,
                bounds: reflect_box(bounds, axis, plane)?,
            },
            WorldShape::Bar {
                axis: along,
                from,
                to,
                thickness,
                bounds,
            } => WorldShape::Bar {
                axis: *along,
                from: reflect_point(*from, axis, plane)?,
                to: reflect_point(*to, axis, plane)?,
                thickness: *thickness,
                bounds: reflect_box(bounds, axis, plane)?,
            },
        })
    };
    members
        .iter()
        .map(|member| {
            Ok(PlacedMember {
                carrier: rebound(member.carrier)?,
                shapes: member
                    .shapes
                    .iter()
                    .map(shape)
                    .collect::<Solved<Vec<_>>>()?,
                ..member.clone()
            })
        })
        .collect()
}

/// A member's shape in the solved design, to the nearest nanometre.
fn member_shape(shape: &WorldShape) -> Solved<solved::MemberShape> {
    let pair = |axis: Axis, point: [ExactLength; 3]| -> Solved<[Length; 2]> {
        let [first, second] = axis.others();
        let at = |on: Axis| point.get(world_index(on)).copied().ok_or(Reported);
        Ok([round_length(at(first)?)?, round_length(at(second)?)?])
    };
    let ends = |axis: Axis, bounds: &ExactBox| -> Solved<(Length, Length)> {
        Ok((
            round_length(bounds.min(axis))?,
            round_length(bounds.max(axis))?,
        ))
    };
    Ok(match shape {
        WorldShape::Box(bounds) => solved::MemberShape::Box(world_box(bounds)?),
        WorldShape::Cylinder { axis, bounds } => {
            let [first, second] = axis.others();
            let centre = |on: Axis| -> Solved<Length> {
                round_length(midpoint(bounds.min(on), bounds.max(on)).ok_or(Reported)?)
            };
            let diameter = bounds
                .max(first)
                .checked_sub(bounds.min(first))
                .ok_or(Reported)?;
            let (start, end) = ends(*axis, bounds)?;
            solved::MemberShape::Cylinder {
                axis: *axis,
                centre: [centre(first)?, centre(second)?],
                diameter: round_length(diameter)?,
                start,
                end,
            }
        }
        WorldShape::Prism {
            axis,
            points,
            bounds,
        } => {
            let (start, end) = ends(*axis, bounds)?;
            solved::MemberShape::Prism {
                axis: *axis,
                points: points
                    .iter()
                    .map(|point| pair(*axis, *point))
                    .collect::<Solved<Vec<_>>>()?,
                start,
                end,
            }
        }
        WorldShape::Bar {
            axis,
            from,
            to,
            thickness,
            bounds,
        } => {
            let (start, end) = ends(*axis, bounds)?;
            solved::MemberShape::Bar {
                axis: *axis,
                from: pair(*axis, *from)?,
                to: pair(*axis, *to)?,
                thickness: round_length(*thickness)?,
                start,
                end,
            }
        }
    })
}
/// A piece of hardware's members in the solved design.
/// A piece of hardware's members in the solved design.
/// A placed member that its kind's mechanism places is the member of its
/// assembly's mechanism it became; with no mechanism moving it (hardware
/// outside a moving assembly) it stays with the piece.
fn members(
    hardware: HardwareId,
    placed: &[PlacedMember],
    placed_by: &BTreeMap<(HardwareId, BodyId), MemberIndex>,
    piece: PartId,
) -> Solved<Vec<solved::Body>> {
    placed
        .iter()
        .map(|member| {
            let carrier = match member.carrier {
                MemberCarrier::Part(part) => solved::BodyCarrier::Part(part),
                MemberCarrier::Region(region) => solved::BodyCarrier::Region(region),
                MemberCarrier::Link(body) => match placed_by.get(&(hardware, body)) {
                    Some(index) => solved::BodyCarrier::Link(*index),
                    None => solved::BodyCarrier::Part(piece),
                },
            };
            let shapes = member
                .shapes
                .iter()
                .map(member_shape)
                .collect::<Solved<Vec<_>>>()?;
            solved::Body::new(
                solved::BodyRef {
                    hardware,
                    body: member.body,
                },
                member.name.clone(),
                carrier,
                shapes,
            )
            .ok_or(Reported)
        })
        .collect()
}

fn reflect_box(bounds: &ExactBox, axis: Axis, plane: ExactLength) -> Solved<ExactBox> {
    let (low, high) = reflect(bounds.min(axis), bounds.max(axis), plane)?;
    Ok(bounds.with(axis, low, high))
}

/// The world position of a box's local corner (0, 0, 0) in a frame.
fn pose_origin(bounds: &ExactBox, orientation: &solved::Orientation) -> [ExactLength; 3] {
    let mut origin = [ExactLength::ZERO; 3];
    for axis in PartAxis::ALL {
        let direction = orientation.direction(axis);
        let world = direction.axis();
        if let Some(slot) = origin.get_mut(world_index(world)) {
            *slot = if direction.is_positive() {
                bounds.min(world)
            } else {
                bounds.max(world)
            };
        }
    }
    origin
}

pub(crate) fn build(solver: &mut Solver<'_>) -> Option<SolvedDesign> {
    let errors_before = solver.diagnostics.iter().filter(|d| d.is_error()).count();
    let part_count = solver.model.parts.len();

    // 1. Every param and let (so domains are checked even when unused),
    //    every part's orientation, sizes and position, every region.
    for index in 0..solver.model.params.len() {
        let _ = solver.param_value(stave_model::ids::ParamId::from_usize(index));
    }
    for index in 0..solver.model.lets.len() {
        let _ = solver.let_value(stave_model::ids::LetId::from_usize(index));
    }
    for index in 0..part_count {
        let part = PartId::from_usize(index);
        let _ = solver.drawn_box(part);
    }
    for index in 0..solver.model.regions.len() {
        let _ = solver.region_value(RegionId::from_usize(index));
    }
    // Every profile of the project, used or not, so stations that go
    // backwards are reported once, at the profile.
    for index in 0..solver.model.defs.profiles.len() {
        let profile = stave_model::ids::ProfileId::from_usize(index);
        let in_project = solver.model.defs.profile(profile).is_some_and(|def| {
            solver.module_ctx(DefId::Profile(profile)).is_some()
                && def.decl.scope == stave_model::defs::Scope::Module(solver.model.root)
        });
        if in_project {
            let _ = solver.profile_stations(profile);
        }
    }

    // 2. Joints, pass A: pairs, requires, extensions.
    let mut work: Vec<Option<JointWork>> = Vec::new();
    for index in 0..solver.model.joints.len() {
        let joint = JointId::from_usize(index);
        let mirrored = solver
            .model
            .joints
            .get(index)
            .is_some_and(|j| j.mirror_of.is_some());
        work.push(if mirrored {
            None
        } else {
            solver.work_joint(joint).ok()
        });
    }
    // 3. Geometry of every declared part with what joints add.
    let mut geoms = GeomTable::default();
    for index in 0..part_count {
        let part = PartId::from_usize(index);
        if let Ok(geom) = solver.part_geom(part) {
            let _ = solver.check_max_length(part, &geom);
            geoms.set(part, geom);
        }
    }

    // 4. Joints, pass B: made parts, cuts, steps.
    let mut cuts: Vec<Vec<Cut>> = vec![Vec::new(); part_count];
    let mut memberships: Vec<Vec<JointId>> = vec![Vec::new(); part_count];
    let mut made: Vec<MadeInfo> = Vec::new();
    let mut joints: Vec<Option<Joint>> = vec![None; solver.model.joints.len()];
    let mut fasteners: Vec<FastenerUse> = Vec::new();
    for (index, slot) in work.iter_mut().enumerate() {
        let Some(joint_work) = slot.as_mut() else {
            continue;
        };
        let joint = JointId::from_usize(index);
        if let Ok(solved) = solver.finish_joint(
            joint,
            joint_work,
            &mut geoms,
            &mut cuts,
            &mut memberships,
            &mut made,
            &mut fasteners,
        ) && let Some(entry) = joints.get_mut(index)
        {
            *entry = Some(solved);
        }
    }

    // 5. Hardware.
    let mut placed: Vec<Option<Placed>> = vec![None; solver.model.hardware.len()];
    for index in 0..solver.model.hardware.len() {
        let hardware = HardwareId::from_usize(index);
        if solver
            .model
            .hardware
            .get(index)
            .is_some_and(|h| h.mirror_of.is_some())
        {
            continue;
        }
        if let Ok(hw) = solver.place_hardware(hardware) {
            for cut in &hw.cuts {
                if let (Some(geom), Some(list)) =
                    (geoms.get(cut.part), cuts.get_mut(cut.part.index()))
                    && let Ok(cut) = hardware_cut(geom, cut, hardware)
                {
                    list.push(cut);
                }
            }
            if let Some(entry) = placed.get_mut(index) {
                *entry = Some(hw);
            }
        }
    }

    // 6. Mirrored assemblies: reflect the source's results.
    solver.mirror_assemblies(
        &mut geoms,
        &mut cuts,
        &mut memberships,
        &mut joints,
        &mut placed,
    );

    // 6b. Parts wider than their stock: boards and the joints between them.
    let boards = solver.make_boards(&mut geoms, &mut cuts, &mut memberships, &mut made);

    // 7. Every assembly's mechanism, from the hardware that moves it.
    let mechanisms = solver.assembly_mechanisms(&placed);

    // 8. Requires at the current values.
    let mut requires = Vec::new();
    for index in 0..solver.model.requires.len() {
        let Some(inst) = solver.model.requires.get(index).cloned() else {
            continue;
        };
        if inst.mirror_of.is_some() {
            continue;
        }
        let Some(def) = solver.model.defs.require(inst.def).cloned() else {
            continue;
        };
        if solver.poisoned(DefId::Require(inst.def)) {
            solver.skipped = true;
            continue;
        }
        let env = Env {
            ctx: inst.ctx,
            file: def.decl.file,
            kind: None,
            array: None,
        };
        let (Ok(holds), Ok(message)) = (
            solver.eval_bool(&def.condition, &env),
            solver.format_text(&def.message, &env),
        ) else {
            continue;
        };
        if !holds {
            solver.report(
                Diagnostic::error(
                    Code::Require,
                    format!("'{}' fails: {message}", inst.name),
                    Label::new(def.decl.file, def.decl.span),
                )
                .with_help("change the design or its params so this holds"),
            );
        }
        requires.push(RequireResult {
            name: inst.name.clone(),
            holds,
            message,
        });
    }
    for joint in joints.iter().flatten() {
        for pair in &joint.pairs {
            requires.extend(pair.requires.iter().cloned());
        }
    }

    // An item with an error was skipped along with everything depending on
    // it; the design is written only when nothing failed.
    if solver.diagnostics.iter().filter(|d| d.is_error()).count() > errors_before || solver.skipped
    {
        return None;
    }

    // 9. Write everything out.
    let parts = solver
        .write_parts(&geoms, cuts, memberships, &made, boards)
        .ok()?;
    let hardware = solver.write_hardware(&placed, &mechanisms).ok()?;
    let regions = solver.write_regions().ok()?;
    let assemblies = solver.write_assemblies(&placed, &mechanisms).ok()?;
    let scenes = solver.write_scenes(&mechanisms).ok()?;
    let project = solver.write_project().ok()?;
    let joints: Vec<Joint> = joints.into_iter().collect::<Option<Vec<_>>>()?;
    if solver.diagnostics.iter().filter(|d| d.is_error()).count() > errors_before {
        return None;
    }
    Some(SolvedDesign {
        project,
        parts,
        hardware,
        fasteners,
        joints,
        regions,
        assemblies,
        scenes,
        requires,
        overruns: {
            let mut overruns = solver.overruns.clone();
            overruns.sort_by_key(|o| o.part);
            overruns.dedup();
            overruns
        },
    })
}

/// A cut a piece of hardware makes: its member's shape where it meets the
/// part, from the face the solver chose.
fn hardware_cut(geom: &PartGeom, cut: &HardwareCut, hardware: HardwareId) -> Solved<Cut> {
    let face = geom.face_toward(cut.face.axis(), cut.face.is_positive());
    let mut region = cut.shape;
    for axis in Axis::ALL {
        let low = region.min(axis).max(geom.cut_box.min(axis));
        let high = region.max(axis).min(geom.cut_box.max(axis));
        region = region.with(axis, low, high.max(low));
    }
    let local = geom.local_box(&region)?;
    let [length, width, thickness] = local;
    let corner = |pick: fn(&(ExactLength, ExactLength)) -> ExactLength| -> Solved<LocalPoint> {
        Ok(LocalPoint {
            length: round_length(pick(&length))?,
            width: round_length(pick(&width))?,
            thickness: round_length(pick(&thickness))?,
        })
    };
    Ok(Cut {
        source: CutSource::Hardware { hardware },
        name: cut.name.clone(),
        face,
        place: CutPlace::Member {
            hardware,
            body: cut.body,
            footprint: LocalBox {
                min: corner(|p| p.0)?,
                max: corner(|p| p.1)?,
            },
        },
        kind: cut.kind.clone(),
    })
}

fn exact_envelope(envelope: &ExactEnvelope) -> Solved<solved::Envelope> {
    Ok(match envelope {
        ExactEnvelope::Box {
            length,
            width,
            thickness,
        } => solved::Envelope::Box {
            length: round_length(*length)?,
            width: round_length(*width)?,
            thickness: round_length(*thickness)?,
        },
        ExactEnvelope::Cylinder { diameter, length } => solved::Envelope::Cylinder {
            diameter: round_length(*diameter)?,
            length: round_length(*length)?,
        },
    })
}

impl Solver<'_> {
    #[allow(clippy::too_many_arguments)]
    fn finish_joint(
        &mut self,
        joint: JointId,
        work: &mut JointWork,
        geoms: &mut GeomTable,
        cuts: &mut Vec<Vec<Cut>>,
        memberships: &mut Vec<Vec<JointId>>,
        made: &mut Vec<MadeInfo>,
        fasteners: &mut Vec<FastenerUse>,
    ) -> Solved<Joint> {
        let Some(inst) = self.model.joints.get(joint.index()).cloned() else {
            return Err(Reported);
        };
        let kind_id = work.binding.kind;
        let Some(kind) = self.model.defs.kind(kind_id).cloned() else {
            return Err(Reported);
        };
        let Some(ctx) = self.module_ctx(DefId::Kind(kind_id)) else {
            return Err(Reported);
        };
        let joint_def = kind.joint.clone().unwrap_or_default();
        let pair_count = work.pairs.len();
        let mut pairs = Vec::new();
        let mut note = None;
        let mut fastener_totals: Vec<(String, Length, Length, u32)> = Vec::new();
        for (pair_index, pair) in work.pairs.iter_mut().enumerate() {
            // Made parts.
            let mut made_ids = Vec::new();
            for made_def in &joint_def.makes {
                let id = PartId::from_usize(self.model.parts.len() + made.len());
                let name = if pair_count > 1 {
                    format!("{}.{}[{pair_index}]", inst.name, made_def.name)
                } else {
                    format!("{}.{}", inst.name, made_def.name)
                };
                let env = Env {
                    ctx,
                    file: kind.decl.file,
                    kind: Some(&pair.env),
                    array: None,
                };
                let placed = self.place_made_part(made_def, pair, geoms, &env, &name)?;
                let note_text = match &made_def.body.note {
                    Some(text) => Some(self.format_text(text, &env)?),
                    None => None,
                };
                geoms.set(id, placed.geom);
                if cuts.len() <= id.index() {
                    cuts.resize(id.index() + 1, Vec::new());
                    memberships.resize(id.index() + 1, Vec::new());
                }
                if let Some(list) = memberships.get_mut(id.index()) {
                    list.push(joint);
                }
                made.push(MadeInfo {
                    origin: PartOrigin::Made {
                        joint,
                        pair: pair_index,
                    },
                    assembly: inst.assembly,
                    name,
                    stock: placed.stock,
                    note: note_text,
                });
                made_ids.push(id);
            }
            pair.env.made = made_ids.clone();
            let env = Env {
                ctx,
                file: kind.decl.file,
                kind: Some(&pair.env),
                array: None,
            };
            for part in &pair.parts {
                if let Some(list) = memberships.get_mut(part.index())
                    && !list.contains(&joint)
                {
                    list.push(joint);
                }
            }
            // Cuts.
            for cut in &joint_def.cuts {
                for (part, face, place, cut_kind) in self.pair_cuts(cut, pair, geoms, &env)? {
                    if let Some(list) = cuts.get_mut(part.index()) {
                        list.push(Cut {
                            source: CutSource::Joint {
                                joint,
                                pair: pair_index,
                            },
                            name: cut.name.clone(),
                            face,
                            place,
                            kind: cut_kind,
                        });
                    }
                }
            }
            let insert = self.pair_steps(kind_id, pair, &env)?;
            let mut cut_order = Vec::new();
            if !joint_def.cut_first.is_empty() {
                for role in &joint_def.cut_first {
                    if let Some(part) = pair.parts.get(*role)
                        && !cut_order.contains(part)
                    {
                        cut_order.push(*part);
                    }
                }
                for part in &pair.parts {
                    if !cut_order.contains(part) {
                        cut_order.push(*part);
                    }
                }
                cut_order.extend(made_ids.iter().copied());
            }
            for uses in &joint_def.uses {
                let Value::Kind(fastener) = self.eval(&uses.fastener, &env)? else {
                    continue;
                };
                let count = match &uses.count {
                    Some(count) => self.eval_count(count, &env)?,
                    None => 1,
                };
                let span = uses.span;
                let diameter = match self.kind_entry(&fastener, "diameter", kind.decl.file, span)? {
                    Value::Length(l) => round_length(l)?,
                    _ => return Err(Reported),
                };
                let length = match self.kind_entry(&fastener, "length", kind.decl.file, span)? {
                    Value::Length(l) => round_length(l)?,
                    _ => return Err(Reported),
                };
                let name = self
                    .model
                    .defs
                    .kind(fastener.kind)
                    .map(|k| k.decl.name.clone())
                    .unwrap_or_default();
                match fastener_totals
                    .iter_mut()
                    .find(|(n, d, l, _)| *n == name && *d == diameter && *l == length)
                {
                    Some(entry) => entry.3 = entry.3.saturating_add(count),
                    None => fastener_totals.push((name, diameter, length, count)),
                }
            }
            if note.is_none()
                && let Some(text) = &joint_def.note
            {
                note = Some(self.format_text(text, &env)?);
            }
            let roles = if pair.across {
                // One `Parts` role: every part is bound to it.
                let name = kind
                    .roles
                    .first()
                    .map(|r| r.name.clone())
                    .unwrap_or_default();
                pair.parts
                    .iter()
                    .map(|part| RoleBinding {
                        role: name.clone(),
                        part: *part,
                    })
                    .collect()
            } else {
                kind.roles
                    .iter()
                    .zip(&pair.parts)
                    .map(|(role, part)| RoleBinding {
                        role: role.name.clone(),
                        part: *part,
                    })
                    .collect()
            };
            pairs.push(JointPair {
                roles,
                contact: pair.contact,
                extensions: pair.extensions.clone(),
                cut_order,
                insert,
                made_parts: made_ids,
                requires: work.requires.get(pair_index).cloned().unwrap_or_default(),
            });
        }
        for (name, diameter, length, count) in fastener_totals {
            fasteners.push(FastenerUse {
                joint,
                kind: name,
                diameter,
                length,
                count,
            });
        }
        let methods = work
            .binding
            .methods
            .iter()
            .filter_map(|r| self.model.defs.recipe(*r).map(|d| d.decl.name.clone()))
            .collect();
        Ok(Joint {
            id: joint,
            name: inst.name.clone(),
            kind: work.binding.name.clone(),
            kind_id: work.binding.kind,
            assembly: inst.assembly,
            glue: work.binding.glue,
            methods,
            pairs,
            unmatched: work.unmatched.clone(),
            note,
        })
    }

    /// A kind's name, qualified with its module unless it is from the
    /// prelude or the project.
    fn kind_name(&self, kind: stave_model::ids::KindId) -> String {
        let Some(def) = self.model.defs.kind(kind) else {
            return String::new();
        };
        match def.decl.scope {
            stave_model::defs::Scope::Module(module)
                if module != self.model.root && !self.is_prelude(module) =>
            {
                let module_name = self
                    .model
                    .defs
                    .modules
                    .get(module.index())
                    .map(|m| m.name.clone())
                    .unwrap_or_default();
                format!("{module_name}.{}", def.decl.name)
            }
            _ => def.decl.name.clone(),
        }
    }

    fn is_prelude(&self, module: stave_modules::ModuleId) -> bool {
        self.model.defs.prelude.contains(&module)
    }

    // -----------------------------------------------------------------------
    // Mirrored assemblies
    // -----------------------------------------------------------------------

    fn mirror_assemblies(
        &mut self,
        geoms: &mut GeomTable,
        cuts: &mut [Vec<Cut>],
        memberships: &mut [Vec<JointId>],
        joints: &mut [Option<Joint>],
        placed: &mut [Option<Placed>],
    ) {
        let mut part_map: HashMap<PartId, PartId> = HashMap::new();
        let mut joint_map: HashMap<JointId, JointId> = HashMap::new();
        let mut hardware_map: HashMap<HardwareId, HardwareId> = HashMap::new();
        for part in &self.model.parts {
            if let Some(source) = part.mirror_of {
                part_map.insert(source, part.id);
            }
        }
        for joint in &self.model.joints {
            if let Some(source) = joint.mirror_of {
                joint_map.insert(source, joint.id);
            }
        }
        for hardware in &self.model.hardware {
            if let Some(source) = hardware.mirror_of {
                hardware_map.insert(source, hardware.id);
            }
        }
        // Parts: cuts and joint memberships of the source, remapped.
        let mirrored_parts: Vec<(PartId, PartId)> = self
            .model
            .parts
            .iter()
            .filter_map(|p| p.mirror_of.map(|s| (p.id, s)))
            .collect();
        for (part, source) in &mirrored_parts {
            let source_cuts = cuts.get(source.index()).cloned().unwrap_or_default();
            let remapped: Vec<Cut> = source_cuts
                .into_iter()
                .map(|mut cut| {
                    cut.source = match cut.source {
                        CutSource::Joint { joint, pair } => CutSource::Joint {
                            joint: *joint_map.get(&joint).unwrap_or(&joint),
                            pair,
                        },
                        CutSource::Hardware { hardware } => CutSource::Hardware {
                            hardware: *hardware_map.get(&hardware).unwrap_or(&hardware),
                        },
                        CutSource::Boards { part, seam } => CutSource::Boards {
                            part: *part_map.get(&part).unwrap_or(&part),
                            seam,
                        },
                    };
                    cut.place = match cut.place {
                        CutPlace::End { toward } => CutPlace::End {
                            toward: *part_map.get(&toward).unwrap_or(&toward),
                        },
                        CutPlace::Side {
                            from,
                            along_from,
                            across_axis,
                            across_from,
                        } => CutPlace::Side {
                            from: *part_map.get(&from).unwrap_or(&from),
                            along_from,
                            across_axis,
                            across_from,
                        },
                        CutPlace::Crossing { other, footprint } => CutPlace::Crossing {
                            other: *part_map.get(&other).unwrap_or(&other),
                            footprint,
                        },
                        CutPlace::Member {
                            hardware,
                            body,
                            footprint,
                        } => CutPlace::Member {
                            hardware: *hardware_map.get(&hardware).unwrap_or(&hardware),
                            body,
                            footprint,
                        },
                    };
                    cut
                })
                .collect();
            if let Some(list) = cuts.get_mut(part.index()) {
                *list = remapped;
            }
            let source_joints = memberships.get(source.index()).cloned().unwrap_or_default();
            if let Some(list) = memberships.get_mut(part.index()) {
                *list = source_joints
                    .iter()
                    .map(|j| *joint_map.get(j).unwrap_or(j))
                    .collect();
            }
            let _ = &geoms;
        }
        // Joints: roles remapped; outside parts re-bound.
        let mirrored_joints: Vec<(JointId, JointId)> = self
            .model
            .joints
            .iter()
            .filter_map(|j| j.mirror_of.map(|s| (j.id, s)))
            .collect();
        for (joint, source) in mirrored_joints {
            let Some(Some(source_joint)) = joints.get(source.index()).cloned() else {
                continue;
            };
            let Some(inst) = self.model.joints.get(joint.index()).cloned() else {
                continue;
            };
            let Some(assembly) = inst.assembly else {
                continue;
            };
            let Ok((axis, plane)) = self.assembly_mirror_plane(assembly) else {
                continue;
            };
            let mut copy = source_joint.clone();
            copy.id = joint;
            copy.name = inst.name.clone();
            copy.assembly = Some(assembly);
            let mut failed = false;
            let mut map = |this: &mut Self, part: PartId| -> PartId {
                if let Some(mapped) = part_map.get(&part) {
                    return *mapped;
                }
                match this.rebind_part(part, axis, plane, assembly) {
                    Ok(found) => found,
                    Err(_) => {
                        failed = true;
                        part
                    }
                }
            };
            for pair in &mut copy.pairs {
                for require in &mut pair.requires {
                    require.name = require.name.replacen(&source_joint.name, &inst.name, 1);
                }
                for role in &mut pair.roles {
                    role.part = map(self, role.part);
                }
                for extension in &mut pair.extensions {
                    extension.part = map(self, extension.part);
                }
                pair.cut_order = pair.cut_order.iter().map(|p| map(self, *p)).collect();
                for step in &mut pair.insert {
                    step.part = map(self, step.part);
                }
            }
            copy.unmatched = copy.unmatched.iter().map(|p| map(self, *p)).collect();
            if !failed && let Some(entry) = joints.get_mut(joint.index()) {
                *entry = Some(copy);
            }
        }
        // Hardware: reflected, with the piece and outside roles re-bound.
        let mirrored_hardware: Vec<(HardwareId, HardwareId)> = self
            .model
            .hardware
            .iter()
            .filter_map(|h| h.mirror_of.map(|s| (h.id, s)))
            .collect();
        for (hardware, source) in mirrored_hardware {
            let Some(Some(source_placed)) = placed.get(source.index()).cloned() else {
                continue;
            };
            let Some(inst) = self.model.hardware.get(hardware.index()).cloned() else {
                continue;
            };
            let Some(assembly) = inst.assembly else {
                continue;
            };
            let Ok((axis, plane)) = self.assembly_mirror_plane(assembly) else {
                continue;
            };
            // Every role re-bound, whatever its name.
            let mut roles = Vec::new();
            let mut unbound = false;
            for (role, bound) in &source_placed.roles {
                let rebound = match *bound {
                    RoleBound::Part(part) => match part_map.get(&part) {
                        Some(mapped) => Ok(RoleBound::Part(*mapped)),
                        None => self
                            .rebind_part(part, axis, plane, assembly)
                            .map(RoleBound::Part),
                    },
                    RoleBound::Region(region) => self
                        .rebind_region(region, axis, plane, assembly)
                        .map(RoleBound::Region),
                };
                match rebound {
                    Ok(rebound) => roles.push((role.clone(), rebound)),
                    Err(_) => {
                        unbound = true;
                        break;
                    }
                }
            }
            if unbound {
                continue;
            }
            let Some((_, RoleBound::Part(piece))) = roles.first().cloned() else {
                continue;
            };
            let data =
                Solver::legacy_rebound(&source_placed.legacy.data, &source_placed.roles, &roles);
            let reflected_motion = source_placed
                .motion
                .and_then(|motion| motion.reflected(axis, plane));
            if source_placed.motion.is_some() && reflected_motion.is_none() {
                continue;
            }
            let mount_face = if source_placed.mount_face.axis() == axis {
                source_placed.mount_face.opposite()
            } else {
                source_placed.mount_face
            };
            let reflect_member = |member: &crate::legacy::Member| {
                reflect_box(&member.bounds, axis, plane).map(|bounds| crate::legacy::Member {
                    envelope: member.envelope,
                    bounds,
                    orientation: member.orientation.mirrored(axis),
                })
            };
            let Ok(moving) = reflect_member(&source_placed.legacy.moving) else {
                continue;
            };
            let fixed = match &source_placed.legacy.fixed {
                Some(member) => match reflect_member(member) {
                    Ok(member) => Some(member),
                    Err(_) => continue,
                },
                None => None,
            };
            let cup = match &source_placed.cup {
                Some(cup) => match (reflect_member(&cup.arm), reflect_member(&cup.knuckle)) {
                    (Ok(arm), Ok(knuckle)) => Some(CupParts {
                        arm,
                        knuckle,
                        ..cup.clone()
                    }),
                    _ => continue,
                },
                None => None,
            };
            // The mechanism's plane is reflected; its points stay.
            let mechanism = match &source_placed.mechanism {
                Some(mechanism) => match mechanism.reflected(axis, plane) {
                    Some(reflected) => Some(reflected),
                    None => continue,
                },
                None => None,
            };
            let own = match &mechanism {
                Some(reflected) => {
                    let kind_name = source_placed.kind_name.clone();
                    match self.check_mechanism(hardware, &roles, reflected, &kind_name) {
                        Ok(own) => own,
                        Err(_) => continue,
                    }
                }
                None => None,
            };
            // Cuts: those in a part inside the assembly came with the
            // mirrored part's cuts; one in a part outside it is cut in the
            // part that occupies the reflected place.
            let mut reflected_cuts = Vec::new();
            for cut in &source_placed.cuts {
                let (part, outside) = match part_map.get(&cut.part) {
                    Some(mapped) => (*mapped, false),
                    None => match self.rebind_part(cut.part, axis, plane, assembly) {
                        Ok(found) => (found, true),
                        Err(_) => {
                            unbound = true;
                            break;
                        }
                    },
                };
                let Ok(shape) = reflect_box(&cut.shape, axis, plane) else {
                    unbound = true;
                    break;
                };
                let reflected_cut = HardwareCut {
                    part,
                    face: if cut.face.axis() == axis {
                        cut.face.opposite()
                    } else {
                        cut.face
                    },
                    shape,
                    ..cut.clone()
                };
                if outside
                    && let (Some(geom), Some(list)) = (geoms.get(part), cuts.get_mut(part.index()))
                    && let Ok(cut) = hardware_cut(geom, &reflected_cut, hardware)
                {
                    list.push(cut);
                }
                reflected_cuts.push(reflected_cut);
            }
            if unbound {
                continue;
            }
            let Ok(members) = reflect_members(
                &source_placed.members,
                &source_placed.roles,
                &roles,
                axis,
                plane,
            ) else {
                continue;
            };
            let reflected = Placed {
                piece,
                mount_face,
                roles,
                cuts: reflected_cuts,
                motion: reflected_motion,
                mechanism,
                own,
                cup,
                legacy: crate::legacy::Legacy {
                    moving,
                    fixed,
                    data,
                },
                members,
                ..source_placed
            };
            if let Some(entry) = placed.get_mut(hardware.index()) {
                *entry = Some(reflected);
            }
        }
    }
    /// The part that occupies the reflected place of an outside part.
    fn rebind_part(
        &mut self,
        part: PartId,
        axis: Axis,
        plane: ExactLength,
        assembly: AssemblyId,
    ) -> Solved<PartId> {
        let source_box = self.drawn_box(part)?;
        let target = reflect_box(&source_box, axis, plane)?;
        for index in 0..self.model.parts.len() {
            let candidate = PartId::from_usize(index);
            if let Ok(candidate_box) = self.drawn_box(candidate)
                && candidate_box == target
            {
                return Ok(candidate);
            }
        }
        let name = self.part_name(part);
        let (file, span) = self.assembly_place(assembly);
        Err(self.error(
            Code::Mirror,
            format!("{name} has no counterpart on the other side of the mirror"),
            file,
            span,
            format!("make the design symmetric about the mirror plane, or give the mirrored assembly its own {name}"),
        ))
    }

    fn rebind_region(
        &mut self,
        region: RegionId,
        axis: Axis,
        plane: ExactLength,
        assembly: AssemblyId,
    ) -> Solved<RegionId> {
        let source = self.region_value(region)?.bounds;
        let target = reflect_box(&source, axis, plane)?;
        for index in 0..self.model.regions.len() {
            let candidate = RegionId::from_usize(index);
            if let Ok(value) = self.region_value(candidate)
                && value.bounds == target
            {
                return Ok(candidate);
            }
        }
        let name = self
            .model
            .regions
            .get(region.index())
            .map(|r| r.name.clone())
            .unwrap_or_default();
        let (file, span) = self.assembly_place(assembly);
        Err(self.error(
            Code::Mirror,
            format!("{name} has no counterpart on the other side of the mirror"),
            file,
            span,
            "make the design symmetric about the mirror plane",
        ))
    }

    fn assembly_place(&self, assembly: AssemblyId) -> (stave_lang::FileId, stave_lang::ast::Span) {
        let mut current = Some(assembly);
        while let Some(id) = current {
            let Some(inst) = self.model.assemblies.get(id.index()) else {
                break;
            };
            if self.is_mirror_declaration(id) {
                return self.place_of(DefId::Assembly(inst.def));
            }
            current = inst.parent;
        }
        (self.project_file, self.project_span)
    }

    // -----------------------------------------------------------------------
    // Writing the solved design
    // -----------------------------------------------------------------------

    fn write_stock(&mut self, stock: StockId) -> Solved<Stock> {
        let Some(def) = self.model.defs.stock(stock).cloned() else {
            return Err(Reported);
        };
        let length = |this: &mut Self, field| -> Solved<Option<Length>> {
            this.stock_length(stock, field)?
                .map(round_length)
                .transpose()
        };
        use stave_model::defs::StockField as F;
        Ok(Stock {
            name: def.decl.name.clone(),
            species: def.species,
            form: def.form,
            thickness: if def.form == StockForm::Dowel {
                None
            } else {
                length(self, F::Thickness)?
            },
            diameter: length(self, F::Diameter)?,
            width: if def.width.is_some() {
                length(self, F::Width)?
            } else {
                None
            },
            max_width: length(self, F::MaxWidth)?,
            max_length: length(self, F::MaxLength)?,
        })
    }

    fn write_parts(
        &mut self,
        geoms: &GeomTable,
        cuts: Vec<Vec<Cut>>,
        memberships: Vec<Vec<JointId>>,
        made: &[MadeInfo],
        mut boards: Vec<Option<solved::Boards>>,
    ) -> Solved<Vec<solved::Part>> {
        let mut parts = Vec::new();
        let mut cuts = cuts.into_iter();
        let mut memberships = memberships.into_iter();
        let total = self.model.parts.len() + made.len();
        for index in 0..total {
            let id = PartId::from_usize(index);
            let part_cuts = cuts.next().unwrap_or_default();
            let part_joints = memberships.next().unwrap_or_default();
            let Some(geom) = geoms.get(id).cloned() else {
                return Err(Reported);
            };
            let (name, assembly, origin, stock, shape, grain, note) =
                if let Some(inst) = self.model.parts.get(index).cloned() {
                    let mirror = self.mirror_source(id)?;
                    let origin = match (inst.mirror_of, mirror) {
                        (Some(source), _) => PartOrigin::MirroredAssembly { source },
                        (None, Some((source, _, _))) => PartOrigin::Mirror { source },
                        (None, None) => PartOrigin::Declared,
                    };
                    let stock = self.part_stock(id)?;
                    let shape = match self.part_profile(id)? {
                        Some(profile) => {
                            let stations = self
                                .profile_stations(profile)?
                                .into_iter()
                                .map(|(at, diameter)| {
                                    Ok(Station {
                                        at: round_length(at)?,
                                        diameter: round_length(diameter)?,
                                    })
                                })
                                .collect::<Solved<Vec<_>>>()?;
                            let def = self.model.defs.profile(profile).ok_or(Reported)?;
                            Shape::Turned {
                                profile: def.decl.name.clone(),
                                stations,
                                smooth: def.smooth,
                                measure: def.measure,
                            }
                        }
                        None => match self
                            .stock_length(stock, stave_model::defs::StockField::Diameter)?
                        {
                            Some(diameter)
                                if self
                                    .model
                                    .defs
                                    .stock(stock)
                                    .is_some_and(|s| s.form == StockForm::Dowel) =>
                            {
                                Shape::Round {
                                    diameter: round_length(diameter)?,
                                }
                            }
                            _ => Shape::Board,
                        },
                    };
                    let source_body = {
                        let mut current = id;
                        while let Some((source, _, _)) = self.mirror_source(current)? {
                            current = source;
                        }
                        current
                    };
                    let body = self.part_body(source_body).cloned();
                    let grain = body.as_ref().map_or(Grain::Length, |b| b.grain);
                    let note = match body.as_ref().and_then(|b| b.note.clone()) {
                        Some(text) => {
                            let env = self.part_env(source_body).ok_or(Reported)?;
                            Some(self.format_text(&text, &env)?)
                        }
                        None => None,
                    };
                    (
                        inst.name.clone(),
                        inst.assembly,
                        origin,
                        stock,
                        shape,
                        grain,
                        note,
                    )
                } else {
                    let info = made.get(index - self.model.parts.len()).ok_or(Reported)?;
                    (
                        info.name.clone(),
                        info.assembly,
                        info.origin,
                        info.stock,
                        Shape::Board,
                        Grain::Length,
                        info.note.clone(),
                    )
                };
            let stock = self.write_stock(stock)?;
            let size = |values: [ExactLength; 3]| -> Solved<Size3> {
                let [length, width, thickness] = values;
                Ok(Size3 {
                    length: round_length(length)?,
                    width: round_length(width)?,
                    thickness: round_length(thickness)?,
                })
            };
            let mut cut_values = [ExactLength::ZERO; 3];
            for axis in PartAxis::ALL {
                if let Some(slot) = cut_values.get_mut(local_slot(axis)) {
                    *slot = geom.cut_size(axis)?;
                }
            }
            let mut origin_values = [ExactLength::ZERO; 3];
            for axis in PartAxis::ALL {
                let direction = geom.orientation.direction(axis);
                let world = direction.axis();
                let value = if direction.is_positive() {
                    geom.cut_box.min(world)
                } else {
                    geom.cut_box.max(world)
                };
                if let Some(slot) = origin_values.get_mut(world_index(world)) {
                    *slot = value;
                }
            }
            parts.push(solved::Part {
                id,
                name,
                assembly,
                origin,
                stock,
                drawn: size(geom.drawn)?,
                cut: size(cut_values)?,
                drawn_offset: size(geom.start)?,
                pose: solved::Pose {
                    origin: point(origin_values)?,
                    orientation: geom.orientation,
                },
                shape,
                grain,
                reference_face: geom.reference_face,
                reference_edge: geom.reference_edge,
                cuts: part_cuts,
                joints: part_joints,
                note,
                boards: boards.get_mut(index).and_then(Option::take),
            });
        }
        Ok(parts)
    }

    fn write_hardware(
        &mut self,
        placed: &[Option<Placed>],
        mechanisms: &[Option<AssemblyMechanism>],
    ) -> Solved<Vec<Hardware>> {
        let placed_by: BTreeMap<(HardwareId, BodyId), MemberIndex> = mechanisms
            .iter()
            .flatten()
            .flat_map(|m| m.bodies.iter().map(|(key, index)| (*key, *index)))
            .collect();
        let mut out = Vec::new();
        for (index, entry) in placed.iter().enumerate() {
            let Some(hw) = entry else {
                return Err(Reported);
            };
            let Some(inst) = self.model.hardware.get(index).cloned() else {
                return Err(Reported);
            };
            let base = HardwareBase::from_base(hw.base).ok_or(Reported)?;
            let moving = &hw.legacy.moving;
            let origin = pose_origin(&moving.bounds, &moving.orientation);
            let member = |member: &crate::legacy::Member| -> Solved<solved::HardwareMember> {
                Ok(solved::HardwareMember {
                    envelope: exact_envelope(&member.envelope)?,
                    pose: solved::Pose {
                        origin: point(pose_origin(&member.bounds, &member.orientation))?,
                        orientation: member.orientation,
                    },
                    bounds: world_box(&member.bounds)?,
                })
            };
            let fixed = hw.legacy.fixed.as_ref().map(member).transpose()?;
            let linkage = hw.motion.as_ref().and_then(LegacyMotion::linkage);
            let mechanism = match (&hw.cup, linkage) {
                (Some(cup_parts), Some(linkage)) => Some(solved::CupMechanism {
                    linkage,
                    arm: member(&cup_parts.arm)?,
                    knuckle: member(&cup_parts.knuckle)?,
                    links: cup_parts.links,
                    pin_diameter: cup_parts.pin_diameter,
                    pin_length: cup_parts.pin_length,
                }),
                _ => None,
            };
            let data = match &hw.legacy.data {
                Data::Slide {
                    length,
                    clearance,
                    travel,
                    opening,
                } => HardwareData::Slide {
                    length: round_length(*length)?,
                    clearance: round_length(*clearance)?,
                    travel: round_length(*travel)?,
                    opening: *opening,
                },
                Data::ButtHinge { opening_angle, on } => HardwareData::ButtHinge {
                    opening_angle: opening_angle.to_nearest_angle().ok_or(Reported)?,
                    on: *on,
                },
                Data::Hinge {
                    opening_angle,
                    cup_diameter,
                    cup_depth,
                    cup_inset,
                    on,
                    overlay,
                } => HardwareData::Hinge {
                    opening_angle: opening_angle.to_nearest_angle().ok_or(Reported)?,
                    cup_diameter: round_length(*cup_diameter)?,
                    cup_depth: round_length(*cup_depth)?,
                    cup_inset: round_length(*cup_inset)?,
                    on: *on,
                    overlay: round_length(*overlay)?,
                },
                Data::Pull {
                    centres,
                    diameter,
                    screw_diameter,
                } => HardwareData::Pull {
                    centres: centres.map(round_length).transpose()?,
                    diameter: diameter.map(round_length).transpose()?,
                    screw_diameter: round_length(*screw_diameter)?,
                },
            };
            out.push(Hardware {
                id: inst.id,
                name: inst.name.clone(),
                kind: hw.kind_name.clone(),
                kind_id: hw.kind,
                base,
                assembly: inst.assembly,
                piece: hw.piece,
                mount_face: hw.mount_face,
                envelope: exact_envelope(&moving.envelope)?,
                pose: solved::Pose {
                    origin: point(origin)?,
                    orientation: moving.orientation,
                },
                bounds: world_box(&moving.bounds)?,
                fixed,
                data,
                mechanism,
                model: hw.model.clone(),
                roles: hw
                    .roles
                    .iter()
                    .filter_map(|(name, bound)| {
                        let value = match bound {
                            RoleBound::Part(part) => solved::RoleValue::Part(*part),
                            RoleBound::Region(region) => solved::RoleValue::Region(*region),
                        };
                        solved::RoleName::new(name).map(|name| (name, value))
                    })
                    .collect(),
                fasteners: hw.fasteners.clone(),
                members: members(inst.id, &hw.members, &placed_by, hw.piece)?,
                own_mechanism: hw.own.clone(),
                settings: hw
                    .settings
                    .iter()
                    .map(|setting| -> Solved<solved::Setting> {
                        Ok(solved::Setting {
                            body: solved::BodyRef {
                                hardware: inst.id,
                                body: setting.body,
                            },
                            axis: setting.axis,
                            offset: round_length(setting.offset)?,
                        })
                    })
                    .collect::<Solved<Vec<_>>>()?,
                rules: hw.rules,
                entries: hw
                    .entries
                    .iter()
                    .filter_map(|(name, value)| {
                        let value = match value {
                            Value::Length(length) => {
                                solved::EntryValue::Length(length.to_nearest_length()?)
                            }
                            Value::Angle(angle) => {
                                solved::EntryValue::Angle(angle.to_nearest_angle()?)
                            }
                            Value::Count(count) if count.is_integer() => {
                                solved::EntryValue::Count(u32::try_from(count.numerator()).ok()?)
                            }
                            Value::Bool(value) => solved::EntryValue::Bool(*value),
                            Value::Text(text) => solved::EntryValue::Text(text.clone()),
                            _ => return None,
                        };
                        Some((solved::EntryName::new(name)?, value))
                    })
                    .collect(),
            });
        }
        Ok(out)
    }

    fn write_regions(&mut self) -> Solved<Vec<Region>> {
        let mut out = Vec::new();
        for index in 0..self.model.regions.len() {
            let id = RegionId::from_usize(index);
            let value = self.region_value(id)?;
            let Some(inst) = self.model.regions.get(index) else {
                return Err(Reported);
            };
            out.push(Region {
                id,
                name: inst.name.clone(),
                assembly: inst.assembly,
                bounds: world_box(&value.bounds)?,
            });
        }
        Ok(out)
    }

    pub(crate) fn within(&self, assembly: Option<AssemblyId>, target: AssemblyId) -> bool {
        let mut current = assembly;
        while let Some(id) = current {
            if id == target {
                return true;
            }
            current = self.model.assemblies.get(id.index()).and_then(|a| a.parent);
        }
        false
    }

    fn write_assemblies(
        &mut self,
        placed: &[Option<Placed>],
        mechanisms: &[Option<AssemblyMechanism>],
    ) -> Solved<Vec<Assembly>> {
        let mut out = Vec::new();
        for inst in &self.model.assemblies {
            let mechanism = mechanisms
                .get(inst.id.index())
                .and_then(Option::as_ref)
                .map(|m| m.mechanism.clone());
            // The old contract's motion: the first driver's.
            let assembly_motion = match mechanism
                .as_ref()
                .and_then(|m| m.hardware().first())
                .and_then(|first| placed.get(first.index()))
                .and_then(Option::as_ref)
                .and_then(|p| p.motion.as_ref())
            {
                Some(motion) => {
                    let extent = mechanism
                        .as_ref()
                        .map(|m| m.drive().extent())
                        .ok_or(Reported)?;
                    Some(legacy_assembly_motion(motion, extent)?)
                }
                None => None,
            };
            let parts = self
                .model
                .parts
                .iter()
                .filter(|p| self.within(p.assembly, inst.id))
                .map(|p| p.id)
                .collect();
            let hardware: Vec<HardwareId> = self
                .model
                .hardware
                .iter()
                .filter(|h| self.within(h.assembly, inst.id))
                .map(|h| h.id)
                .collect();
            let component = self
                .model
                .defs
                .component(inst.component)
                .map(|c| c.decl.name.clone())
                .unwrap_or_default();
            out.push(Assembly {
                id: inst.id,
                name: inst.name.clone(),
                component,
                parent: inst.parent,
                mirror_of: inst.mirror_of,
                parts,
                hardware,
                motion: assembly_motion,
                mechanism,
            });
        }
        Ok(out)
    }

    fn scene_items(&self, value: &Value, out: &mut Vec<SceneItem>) {
        match value {
            Value::Part(part) => out.push(SceneItem::Part(*part)),
            Value::Parts(parts) => out.extend(parts.iter().map(|p| SceneItem::Part(*p))),
            Value::Assembly(assembly) => out.push(SceneItem::Assembly(*assembly)),
            Value::Assemblies(list) => out.extend(list.iter().map(|a| SceneItem::Assembly(*a))),
            Value::Hardware(hardware) => out.push(SceneItem::Hardware(*hardware)),
            Value::List(items) => {
                for item in items {
                    self.scene_items(item, out);
                }
            }
            _ => {}
        }
    }

    fn write_scenes(&mut self, mechanisms: &[Option<AssemblyMechanism>]) -> Solved<Vec<Scene>> {
        let mut scenes = vec![Scene {
            id: SceneId::from_usize(0),
            name: stave_schema::REST_SCENE.to_owned(),
            title: Some(stave_schema::REST_SCENE_TITLE.to_owned()),
            poses: Vec::new(),
            show: None,
            hide: Vec::new(),
            explode: false,
            section: None,
        }];
        let root = self
            .model
            .module_instance(self.model.root)
            .ok_or(Reported)?;
        for scene in self.model.scenes.clone() {
            // A scene with a name or type error is already reported.
            if self.poisoned(DefId::Scene(scene)) {
                self.skipped = true;
                continue;
            }
            let Some(def) = self.model.defs.scene(scene).cloned() else {
                continue;
            };
            let env = Env {
                ctx: Ctx {
                    instance: root,
                    element: None,
                },
                file: def.decl.file,
                kind: None,
                array: None,
            };
            let mut poses = Vec::new();
            for pose in &def.poses {
                let Value::Assembly(assembly) = self.eval(&pose.target, &env)? else {
                    return Err(self.error(
                        Code::TypeMismatch,
                        "'pose' needs an assembly",
                        def.decl.file,
                        pose.target.span,
                        "name an assembly with hinges or slides",
                    ));
                };
                let value = match self.eval(&pose.value, &env)? {
                    Value::Word(Word::Motion(MotionWord::Open)) => PoseValue::Open,
                    Value::Word(Word::Motion(MotionWord::Closed)) => PoseValue::Closed,
                    Value::Angle(angle) => {
                        PoseValue::Angle(angle.to_nearest_angle().ok_or(Reported)?)
                    }
                    Value::Length(length) => PoseValue::Travel(round_length(length)?),
                    _ => return Err(Reported),
                };
                let mechanism = mechanisms
                    .get(assembly.index())
                    .and_then(Option::as_ref)
                    .map(|m| &m.mechanism);
                if let Some(mechanism) = mechanism {
                    self.pose_within(assembly, mechanism, value, def.decl.file, pose.span)?;
                }
                poses.push(ScenePose { assembly, value });
            }
            let show = match &def.show {
                Some(expr) => {
                    let value = self.eval(expr, &env)?;
                    let mut items = Vec::new();
                    self.scene_items(&value, &mut items);
                    Some(items)
                }
                None => None,
            };
            let hide = match &def.hide {
                Some(expr) => {
                    let value = self.eval(expr, &env)?;
                    let mut items = Vec::new();
                    self.scene_items(&value, &mut items);
                    items
                }
                None => Vec::new(),
            };
            let explode = match &def.explode {
                Some(expr) => self.eval_bool(expr, &env)?,
                None => false,
            };
            let section = match &def.section {
                Some(expr) => {
                    let anchor = self.eval_anchor(expr, &env)?;
                    Some(solved::Plane {
                        axis: anchor.axis,
                        position: round_length(anchor.position)?,
                    })
                }
                None => None,
            };
            scenes.push(Scene {
                id: SceneId::from_usize(scenes.len()),
                name: def.decl.name.clone(),
                title: def.title.clone(),
                poses,
                show,
                hide,
                explode,
                section,
            });
        }
        Ok(scenes)
    }

    fn write_project(&mut self) -> Solved<ProjectInfo> {
        let Some(project) = self.model.defs.project(self.model.root).cloned() else {
            return Err(Reported);
        };
        let root = self
            .model
            .module_instance(self.model.root)
            .ok_or(Reported)?;
        let env = Env {
            ctx: Ctx {
                instance: root,
                element: None,
            },
            file: project.file,
            kind: None,
            array: None,
        };
        let resolution = match &project.resolution {
            Some(expr) => {
                let value = round_length(self.eval_length(expr, &env)?)?;
                if value <= Length::ZERO {
                    return Err(self.error(
                        Code::OutOfRange,
                        format!("'resolution' must be within more than 0mm, found {value}"),
                        project.file,
                        expr.span,
                        "write a positive length, such as 0.5mm",
                    ));
                }
                value
            }
            None => Length::from_nm(stave_schema::fields::DEFAULT_RESOLUTION_NM),
        };
        let tools = match &project.tools {
            Some(list) => {
                let mut tools = Vec::new();
                for tool in list {
                    let Some(kind) = self.model.defs.kind(*tool).cloned() else {
                        continue;
                    };
                    let kind_value = stave_model::value::KindValue {
                        kind: *tool,
                        name: self.kind_name(*tool),
                        settings: Vec::new(),
                    };
                    let title = match kind.field("title") {
                        Some(_) => match self.kind_entry(
                            &kind_value,
                            "title",
                            project.file,
                            project.span,
                        )? {
                            Value::Text(text) => Some(text),
                            _ => None,
                        },
                        None => None,
                    };
                    let powered = match kind.field("powered") {
                        Some(_) => {
                            self.kind_entry(&kind_value, "powered", project.file, project.span)?
                                == Value::Bool(true)
                        }
                        None => false,
                    };
                    tools.push(Tool {
                        name: self.kind_name(*tool),
                        title,
                        powered,
                    });
                }
                Some(tools)
            }
            None => None,
        };
        let methods = project
            .methods
            .iter()
            .filter_map(|r| self.model.defs.recipe(*r).map(|d| d.decl.name.clone()))
            .collect();
        Ok(ProjectInfo {
            title: project.title.clone(),
            goal: project.goal.clone(),
            units: project.units,
            resolution,
            style: project.style.clone(),
            tools,
            methods,
        })
    }
}
