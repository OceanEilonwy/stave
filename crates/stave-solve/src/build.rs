//! Putting it together: runs every step in order and writes the
//! [`SolvedDesign`].

use std::collections::HashMap;

use stave_lang::placement::{Axis, Direction, PartAxis};
use stave_lang::units::{ExactLength, Length};
use stave_lang::{Code, Diagnostic, Label};
use stave_model::defs::{DefId, Word};
use stave_model::ids::{AssemblyId, HardwareId, JointId, PartId, RegionId, SceneId, StockId};
use stave_model::instance::Ctx;
use stave_model::solved::{
    self, Assembly, Cut, CutPlace, CutSource, FastenerUse, Hardware, HardwareBase, HardwareData,
    Joint, JointPair, LocalBox, LocalPoint, Motion, PartOrigin, Pivot, Point, PoseValue,
    ProjectInfo, Region, RequireResult, RoleBinding, Scene, SceneItem, ScenePose, Shape, Size3,
    SolvedDesign, Station, Stock, Tool,
};
use stave_model::value::{Envelope as ExactEnvelope, ExactBox, Value};
use stave_schema::words::{Grain, MotionWord, StockForm};

use crate::eval::Env;
use crate::hardware::{HardwareCut, MotionExact, Placed, RoleBound, world_index};
use crate::joints::{GeomTable, JointWork, PartGeom, local_slot, round_length};
use crate::legacy::Data;
use crate::place::reflect;
use crate::{Reported, Solved, Solver};

/// A part a joint made.
struct MadeInfo {
    joint: JointId,
    pair: usize,
    name: String,
    stock: StockId,
    note: Option<String>,
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

fn reflect_direction(direction: Direction, axis: Axis) -> Direction {
    if direction.axis() == axis {
        direction.opposite()
    } else {
        direction
    }
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

    // 7. Requires at the current values.
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

    // 8. Write everything out.
    let parts = solver.write_parts(&geoms, cuts, memberships, &made).ok()?;
    let hardware = solver.write_hardware(&placed).ok()?;
    let regions = solver.write_regions().ok()?;
    let assemblies = solver.write_assemblies(&placed).ok()?;
    let scenes = solver.write_scenes().ok()?;
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

fn motion(motion: &MotionExact) -> Solved<Motion> {
    Ok(match motion {
        MotionExact::Turn {
            axis,
            through,
            free_side,
            swing_toward,
            angle,
        } => Motion::Turn {
            pivot: Pivot {
                axis: *axis,
                through: point(*through)?,
                free_side: *free_side,
                swing_toward: *swing_toward,
            },
            opening_angle: angle.to_nearest_angle().ok_or(Reported)?,
        },
        MotionExact::Linkage(linkage) => Motion::Linkage { linkage: *linkage },
        MotionExact::Travel { toward, travel } => Motion::Travel {
            toward: *toward,
            travel: round_length(*travel)?,
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
                    joint,
                    pair: pair_index,
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
            let roles = kind
                .roles
                .iter()
                .zip(&pair.parts)
                .map(|(role, part)| RoleBinding {
                    role: role.name.clone(),
                    part: *part,
                })
                .collect();
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
            let reflected_motion = source_placed.motion.and_then(|m| match m {
                MotionExact::Turn {
                    axis: pivot_axis,
                    through,
                    free_side,
                    swing_toward,
                    angle,
                } => {
                    let mut through = through;
                    if let Some(slot) = through.get_mut(world_index(axis)) {
                        let twice = plane.checked_add(plane).unwrap_or(plane);
                        *slot = twice.checked_sub(*slot).unwrap_or(*slot);
                    }
                    Some(MotionExact::Turn {
                        axis: pivot_axis,
                        through,
                        free_side: reflect_direction(free_side, axis),
                        swing_toward: reflect_direction(swing_toward, axis),
                        angle,
                    })
                }
                MotionExact::Linkage(linkage) => {
                    // The linkage's points in its own frame stay; its frame
                    // is reflected, the origin exactly, then to the nearest
                    // nanometre.
                    let at = linkage.frame().origin().get(axis).to_exact();
                    let twice = plane.checked_add(plane)?;
                    let origin = twice.checked_sub(at)?.to_nearest_length()?;
                    Some(MotionExact::Linkage(
                        linkage.in_frame(linkage.frame().reflected(axis, origin)),
                    ))
                }
                MotionExact::Travel { toward, travel } => Some(MotionExact::Travel {
                    toward: reflect_direction(toward, axis),
                    travel,
                }),
            });
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
            let mechanism = match &source_placed.mechanism {
                Some(mechanism) => match (
                    reflect_member(&mechanism.arm),
                    reflect_member(&mechanism.knuckle),
                ) {
                    (Ok(arm), Ok(knuckle)) => Some(crate::hardware::Mechanism {
                        arm,
                        knuckle,
                        ..mechanism.clone()
                    }),
                    _ => continue,
                },
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
            let reflected = Placed {
                piece,
                mount_face,
                roles,
                cuts: reflected_cuts,
                motion: reflected_motion,
                mechanism,
                legacy: crate::legacy::Legacy {
                    moving,
                    fixed,
                    data,
                },
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
                    let assembly = self
                        .model
                        .joints
                        .get(info.joint.index())
                        .and_then(|j| j.assembly);
                    (
                        info.name.clone(),
                        assembly,
                        PartOrigin::Made {
                            joint: info.joint,
                            pair: info.pair,
                        },
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
            });
        }
        Ok(parts)
    }

    fn write_hardware(&mut self, placed: &[Option<Placed>]) -> Solved<Vec<Hardware>> {
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
            let mechanism = match (&hw.mechanism, &hw.motion) {
                (Some(mechanism), Some(MotionExact::Linkage(linkage))) => {
                    Some(solved::CupMechanism {
                        linkage: *linkage,
                        arm: member(&mechanism.arm)?,
                        knuckle: member(&mechanism.knuckle)?,
                        links: mechanism.links,
                        pin_diameter: mechanism.pin_diameter,
                        pin_length: mechanism.pin_length,
                    })
                }
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
                    overlay_range,
                } => HardwareData::Hinge {
                    opening_angle: opening_angle.to_nearest_angle().ok_or(Reported)?,
                    cup_diameter: round_length(*cup_diameter)?,
                    cup_depth: round_length(*cup_depth)?,
                    cup_inset: round_length(*cup_inset)?,
                    on: *on,
                    overlay: round_length(*overlay)?,
                    overlay_range: overlay_range
                        .map(|range| -> Solved<solved::OverlayRange> {
                            Ok(solved::OverlayRange {
                                base: round_length(range.base)?,
                                plate_height: round_length(range.plate_height)?,
                                adjustment: round_length(range.adjustment)?,
                            })
                        })
                        .transpose()?,
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
                model: hw.model.as_ref().map(|m| solved::Model {
                    source: m.source.clone(),
                    node: match &m.align {
                        stave_model::value::ModelAlign::Node(node) => Some(node.clone()),
                        stave_model::value::ModelAlign::Bounds => None,
                    },
                }),
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

    fn within(&self, assembly: Option<AssemblyId>, target: AssemblyId) -> bool {
        let mut current = assembly;
        while let Some(id) = current {
            if id == target {
                return true;
            }
            current = self.model.assemblies.get(id.index()).and_then(|a| a.parent);
        }
        false
    }

    fn write_assemblies(&mut self, placed: &[Option<Placed>]) -> Solved<Vec<Assembly>> {
        let mut out = Vec::new();
        for inst in &self.model.assemblies {
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
            let motions: Vec<MotionExact> = self
                .model
                .hardware
                .iter()
                .filter(|h| h.assembly == Some(inst.id))
                .filter_map(|h| {
                    placed
                        .get(h.id.index())
                        .and_then(|p| p.as_ref())
                        .and_then(|p| p.motion)
                })
                .collect();
            let turns = motions
                .iter()
                .filter(|m| matches!(m, MotionExact::Turn { .. }))
                .count();
            let assembly_motion = match motions.first() {
                Some(first) if turns == 0 || turns == motions.len() => Some(motion(first)?),
                _ => None,
            };
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

    fn write_scenes(&mut self) -> Solved<Vec<Scene>> {
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
