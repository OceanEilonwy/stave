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
    self, Assembly, BodyOwner, Cut, CutPlace, CutSource, FastenerUse, Hardware, HardwareBase,
    Joint, JointOrigin, JointPair, LocalBox, LocalPoint, PartOrigin, Point, PoseValue, ProjectInfo,
    Region, RequireResult, RoleBinding, Scene, SceneItem, ScenePose, Shape, Size3, SolvedDesign,
    Station, Stock, Tool,
};
use stave_model::value::{ExactBox, Value};
use stave_schema::words::{Grain, MotionWord, StockForm};

use crate::eval::{Env, midpoint};
use crate::hardware::{
    HardwareCut, MemberCarrier, Placed, PlacedMember, RoleBound, WorldShape, crossing_face,
    world_index, world_shape,
};
use crate::joints::{GeomTable, JointWork, Making, PairWork, PartGeom, local_slot, round_length};
use crate::mechanism::AssemblyMechanism;
use crate::members::{LaidOut, WorldMemberCut};
use crate::mirror_cuts::MirroredJoint;
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
                    owner: BodyOwner::Hardware { hardware },
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

/// A member's shape in the solved design, reflected through a plane
/// across `axis`.
fn reflect_member_shape(
    shape: &solved::MemberShape,
    axis: Axis,
    plane: ExactLength,
) -> Solved<solved::MemberShape> {
    let flip = |low: Length, high: Length| -> Solved<(Length, Length)> {
        let (low, high) = reflect(low.to_exact(), high.to_exact(), plane)?;
        Ok((round_length(low)?, round_length(high)?))
    };
    let flip_one = |value: Length| -> Solved<Length> { Ok(flip(value, value)?.0) };
    // A point on the plane across `along`, its two coordinates in
    // `Axis::others` order.
    let flip_pair = |along: Axis, [a, b]: [Length; 2]| -> Solved<[Length; 2]> {
        let [first, second] = along.others();
        Ok([
            if first == axis { flip_one(a)? } else { a },
            if second == axis { flip_one(b)? } else { b },
        ])
    };
    let ends = |along: Axis, start: Length, end: Length| -> Solved<(Length, Length)> {
        if along == axis {
            flip(start, end)
        } else {
            Ok((start, end))
        }
    };
    Ok(match shape {
        solved::MemberShape::Box(bounds) => {
            let (low, high) = flip(bounds.min().get(axis), bounds.max().get(axis))?;
            solved::MemberShape::Box(solved::WorldBox::new(
                bounds.min().with(axis, low),
                bounds.max().with(axis, high),
            ))
        }
        solved::MemberShape::Cylinder {
            axis: along,
            centre,
            diameter,
            start,
            end,
        } => {
            let (start, end) = ends(*along, *start, *end)?;
            solved::MemberShape::Cylinder {
                axis: *along,
                centre: flip_pair(*along, *centre)?,
                diameter: *diameter,
                start,
                end,
            }
        }
        solved::MemberShape::Prism {
            axis: along,
            points,
            start,
            end,
        } => {
            let (start, end) = ends(*along, *start, *end)?;
            solved::MemberShape::Prism {
                axis: *along,
                points: points
                    .iter()
                    .map(|point| flip_pair(*along, *point))
                    .collect::<Solved<Vec<_>>>()?,
                start,
                end,
            }
        }
        solved::MemberShape::Bar {
            axis: along,
            from,
            to,
            thickness,
            start,
            end,
        } => {
            let (start, end) = ends(*along, *start, *end)?;
            solved::MemberShape::Bar {
                axis: *along,
                from: flip_pair(*along, *from)?,
                to: flip_pair(*along, *to)?,
                thickness: *thickness,
                start,
                end,
            }
        }
    })
}

fn reflect_box(bounds: &ExactBox, axis: Axis, plane: ExactLength) -> Solved<ExactBox> {
    let (low, high) = reflect(bounds.min(axis), bounds.max(axis), plane)?;
    Ok(bounds.with(axis, low, high))
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

    solver.warn_missing_max_length();

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
        &work,
    );
    // A mirrored joint that was made buys its source's fasteners again.
    for inst in &solver.model.joints {
        let Some(source) = inst.mirror_of else {
            continue;
        };
        if !joints.get(inst.id.index()).is_some_and(Option::is_some) {
            continue;
        }
        let copied: Vec<FastenerUse> = fasteners
            .iter()
            .filter(|f| f.joint == source)
            .map(|f| FastenerUse {
                joint: inst.id,
                ..f.clone()
            })
            .collect();
        fasteners.extend(copied);
    }

    // 6b. Spliced parts: their pieces and the splices between them.
    let splits = solver.make_pieces(&mut crate::pieces::Tables {
        geoms: &mut geoms,
        cuts: &mut cuts,
        memberships: &mut memberships,
        made: &mut made,
        joints: &mut joints,
    });

    // 6c. Parts wider than their stock: boards and the joints between them.
    let boards = solver.make_boards(&mut geoms, &mut cuts, &mut memberships, &mut made, &splits);

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
    let assemblies = solver.write_assemblies(&mechanisms).ok()?;
    let scenes = solver.write_scenes(&mechanisms).ok()?;
    let project = solver.write_project().ok()?;
    let joints: Vec<Joint> = joints.into_iter().collect::<Option<Vec<_>>>()?;
    if solver.diagnostics.iter().filter(|d| d.is_error()).count() > errors_before {
        return None;
    }
    let mut design = SolvedDesign {
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
        wall_rules: std::mem::take(&mut solver.wall_rules),
    };
    solver.rebind_pieces(&mut design, &splits);
    Some(design)
}

/// Which part each of a joint's roles names in one pair.
fn pair_roles(kind: &stave_model::defs::KindDef, pair: &PairWork) -> Vec<RoleBinding> {
    if pair.across {
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
    }
}

/// A cut a piece of hardware makes: its member's shape where it meets the
/// part, from the face the solver chose.
fn hardware_cut(geom: &PartGeom, cut: &HardwareCut, hardware: HardwareId) -> Solved<Cut> {
    member_cut(
        geom,
        MemberCut {
            source: CutSource::Hardware { hardware },
            owner: BodyOwner::Hardware { hardware },
            body: cut.body,
            name: cut.name.clone(),
            face: cut.face,
            shape: &cut.shape,
            kind: cut.kind.clone(),
        },
    )
}

/// One cut a member's shape makes in a part.
struct MemberCut<'a> {
    source: CutSource,
    owner: BodyOwner,
    body: BodyId,
    name: Option<solved::CutName>,
    /// The world face of the part it is cut from.
    face: stave_lang::placement::Face,
    /// The shape's world box.
    shape: &'a ExactBox,
    kind: solved::CutKind,
}

/// A cut a member makes: its shape where it meets the part (the
/// footprint), and the shape's whole box, in the part's frame.
fn member_cut(geom: &PartGeom, cut: MemberCut<'_>) -> Solved<Cut> {
    let face = geom.face_toward(cut.face.axis(), cut.face.is_positive());
    let mut region = *cut.shape;
    for axis in Axis::ALL {
        let low = region.min(axis).max(geom.cut_box.min(axis));
        let high = region.max(axis).min(geom.cut_box.max(axis));
        region = region.with(axis, low, high.max(low));
    }
    let local_box = |world: &ExactBox| -> Solved<LocalBox> {
        let [length, width, thickness] = geom.local_box(world)?;
        let corner = |pick: fn(&(ExactLength, ExactLength)) -> ExactLength| -> Solved<LocalPoint> {
            Ok(LocalPoint {
                length: round_length(pick(&length))?,
                width: round_length(pick(&width))?,
                thickness: round_length(pick(&thickness))?,
            })
        };
        Ok(LocalBox {
            min: corner(|p| p.0)?,
            max: corner(|p| p.1)?,
        })
    };
    Ok(Cut {
        source: cut.source,
        name: cut.name,
        face,
        place: CutPlace::Member {
            owner: cut.owner,
            body: cut.body,
            footprint: local_box(&region)?,
            shape: local_box(cut.shape)?,
        },
        kind: cut.kind,
    })
}

/// A joint's member for one pair in the solved design: one shape for
/// each shape of each copy, carried by the pair's first part.
fn joint_body(
    owner: BodyOwner,
    member: &LaidOut,
    carrier: PartId,
    item: Option<solved::MemberItem>,
) -> Solved<solved::Body> {
    let mut shapes = Vec::new();
    for copy in &member.copies {
        for shape in &copy.shapes {
            shapes.push(member_shape(&world_shape(shape, &copy.frame)?)?);
        }
    }
    let copies = u32::try_from(member.copies.len()).map_err(|_| Reported)?;
    solved::Body::new(
        solved::BodyRef {
            owner,
            body: member.body,
        },
        member.name.clone(),
        solved::BodyCarrier::Part(carrier),
        shapes,
    )
    .and_then(|body| body.laid_out(copies, item))
    .ok_or(Reported)
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
            if pair.making == Making::Withheld {
                // The parts don't meet as the kind needs: its rule has
                // said so, and nothing is cut, made or placed here.
                for part in &pair.parts {
                    if let Some(list) = memberships.get_mut(part.index())
                        && !list.contains(&joint)
                    {
                        list.push(joint);
                    }
                }
                pairs.push(JointPair {
                    roles: pair_roles(&kind, pair),
                    contact: pair.contact,
                    extensions: Vec::new(),
                    cut_order: Vec::new(),
                    insert: Vec::new(),
                    made_parts: Vec::new(),
                    requires: work.requires.get(pair_index).cloned().unwrap_or_default(),
                    members: Vec::new(),
                });
                continue;
            }
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
            // Members, laid out on the patch, and the cuts they make.
            let owner = BodyOwner::Joint {
                joint,
                pair: pair_index,
            };
            let place = self.joint_place(joint);
            let laid_out = self.lay_out_members(&kind, &env, &inst.name, place)?;
            for cut in &joint_def.member_cuts {
                let (Some(part), Some(member)) = (
                    pair.parts.get(cut.part.index()).copied(),
                    laid_out.iter().find(|member| member.body == cut.body),
                ) else {
                    return Err(Reported);
                };
                let kinds = cut
                    .operations
                    .iter()
                    .map(|operation| self.cut_kind(operation, &env))
                    .collect::<Solved<Vec<_>>>()?;
                let Some(geom) = geoms.get(part).cloned() else {
                    return Err(Reported);
                };
                for copy in &member.copies {
                    for shape in &copy.shapes {
                        let Some(face) = crossing_face(&shape.world, &geom.cut_box, &copy.frame)
                        else {
                            let part_name = self.part_name(part);
                            let (file, span) = place;
                            return Err(self.error(
                                Code::NoContact,
                                format!(
                                    "{}: its {} doesn't reach {part_name}",
                                    inst.name, member.name
                                ),
                                file,
                                span,
                                format!(
                                    "the kind cuts {part_name} for its {0}, so the {0} must go into {part_name}: check the {0}'s shape and where it is laid out",
                                    member.name
                                ),
                            ));
                        };
                        for cut_kind in &kinds {
                            work.member_cuts.push(WorldMemberCut {
                                pair: pair_index,
                                part,
                                body: cut.body,
                                name: cut.name.clone(),
                                face,
                                shape: shape.world,
                                kind: cut_kind.clone(),
                            });
                            let made = member_cut(
                                &geom,
                                MemberCut {
                                    source: CutSource::Joint {
                                        joint,
                                        pair: pair_index,
                                    },
                                    owner,
                                    body: cut.body,
                                    name: cut.name.clone(),
                                    face,
                                    shape: &shape.world,
                                    kind: cut_kind.clone(),
                                },
                            )?;
                            if let Some(list) = cuts.get_mut(part.index()) {
                                list.push(made);
                            }
                        }
                    }
                }
            }
            let carrier = pair.parts.first().copied().ok_or(Reported)?;
            // A member laid out no times (`count: 0`) is no body.
            let mut members = Vec::new();
            for member in laid_out.iter().filter(|member| !member.copies.is_empty()) {
                let item = match (&member.item, kind.bodies.get(member.body.index())) {
                    (Some(item), Some(body)) => {
                        let span = body.item.as_ref().map_or(body.span, |written| written.span);
                        Some(self.member_item(item, kind.decl.file, span)?)
                    }
                    (Some(_) | None, None) | (None, Some(_)) => None,
                };
                members.push(joint_body(owner, member, carrier, item)?);
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
            // Fasteners: those `uses:` counts, and every copy of a member
            // that says what it is.
            let mut counted: Vec<(stave_model::value::KindValue, u32, stave_lang::ast::Span)> =
                Vec::new();
            for uses in &joint_def.uses {
                let Value::Kind(fastener) = self.eval(&uses.fastener, &env)? else {
                    continue;
                };
                let count = match &uses.count {
                    Some(count) => self.eval_count(count, &env)?,
                    None => 1,
                };
                counted.push((fastener, count, uses.span));
            }
            for (member, body) in laid_out.iter().filter_map(|member| {
                let body = kind.bodies.get(member.body.index())?;
                Some((member, body))
            }) {
                if let (Some(item), Some(written)) = (&member.item, &body.item)
                    && !member.copies.is_empty()
                {
                    let count = u32::try_from(member.copies.len()).map_err(|_| Reported)?;
                    counted.push((item.clone(), count, written.span));
                }
            }
            for (fastener, count, span) in counted {
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
            pairs.push(JointPair {
                roles: pair_roles(&kind, pair),
                contact: pair.contact,
                extensions: pair.extensions.clone(),
                cut_order,
                insert,
                made_parts: made_ids,
                requires: work.requires.get(pair_index).cloned().unwrap_or_default(),
                members,
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
            origin: JointOrigin::Declared,
        })
    }

    /// What a joint's member is (`member pin : dowel_pin(...)`): the
    /// fastener kind's name and, when it says, the wood it is made of.
    fn member_item(
        &mut self,
        item: &stave_model::value::KindValue,
        file: stave_lang::FileId,
        span: stave_lang::ast::Span,
    ) -> Solved<solved::MemberItem> {
        let Some(def) = self.model.defs.kind(item.kind) else {
            return Err(Reported);
        };
        let kind = def.decl.name.clone();
        let says_wood = def.param("species").is_some() || def.field("species").is_some();
        let wood = if says_wood {
            match self.kind_entry(item, "species", file, span)? {
                Value::Word(stave_model::defs::Word::Species(species)) => Some(species),
                _ => None,
            }
        } else {
            None
        };
        Ok(solved::MemberItem { kind, wood })
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
        work: &[Option<JointWork>],
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
                            owner,
                            body,
                            footprint,
                            shape,
                        } => CutPlace::Member {
                            owner: match owner {
                                BodyOwner::Hardware { hardware } => BodyOwner::Hardware {
                                    hardware: *hardware_map.get(&hardware).unwrap_or(&hardware),
                                },
                                BodyOwner::Joint { joint, pair } => BodyOwner::Joint {
                                    joint: *joint_map.get(&joint).unwrap_or(&joint),
                                    pair,
                                },
                            },
                            body,
                            footprint,
                            shape,
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
            let mut unreflected = false;
            for (pair_index, pair) in copy.pairs.iter_mut().enumerate() {
                // The members are reflected and belong to the copy.
                let mut members = Vec::with_capacity(pair.members.len());
                for member in &pair.members {
                    let carrier = match member.carrier {
                        solved::BodyCarrier::Part(part) => {
                            solved::BodyCarrier::Part(map(self, part))
                        }
                        other => other,
                    };
                    let shapes = member
                        .shapes()
                        .iter()
                        .map(|shape| reflect_member_shape(shape, axis, plane))
                        .collect::<Solved<Vec<_>>>();
                    let body = shapes.ok().and_then(|shapes| {
                        solved::Body::new(
                            solved::BodyRef {
                                owner: BodyOwner::Joint {
                                    joint,
                                    pair: pair_index,
                                },
                                body: member.id.body,
                            },
                            member.name.clone(),
                            carrier,
                            shapes,
                        )
                        .and_then(|body| body.laid_out(member.copies(), member.item.clone()))
                    });
                    match body {
                        Some(body) => members.push(body),
                        None => unreflected = true,
                    }
                }
                pair.members = members;
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
            if failed || unreflected {
                continue;
            }
            // Its members' cuts in parts outside the assembly are made
            // again, reflected, in the part that occupies the reflected
            // place; those inside came with the mirrored parts' cuts.
            let source_cuts = work
                .get(source.index())
                .and_then(Option::as_ref)
                .map(|w| w.member_cuts.clone())
                .unwrap_or_default();
            for cut in source_cuts {
                if part_map.contains_key(&cut.part) {
                    continue;
                }
                let Ok(part) = self.rebind_part(cut.part, axis, plane, assembly) else {
                    continue;
                };
                let (Ok(shape), Some(geom)) =
                    (reflect_box(&cut.shape, axis, plane), geoms.get(part))
                else {
                    continue;
                };
                let face = if cut.face.axis() == axis {
                    cut.face.opposite()
                } else {
                    cut.face
                };
                let made = member_cut(
                    geom,
                    MemberCut {
                        source: CutSource::Joint {
                            joint,
                            pair: cut.pair,
                        },
                        owner: BodyOwner::Joint {
                            joint,
                            pair: cut.pair,
                        },
                        body: cut.body,
                        name: cut.name,
                        face,
                        shape: &shape,
                        kind: cut.kind,
                    },
                );
                if let (Ok(made), Some(list)) = (made, cuts.get_mut(part.index())) {
                    list.push(made);
                }
            }
            // Its kind's cuts in parts outside the assembly too.
            self.mirror_outside_cuts(
                &MirroredJoint {
                    joint,
                    source,
                    assembly,
                    axis,
                    plane,
                    part_map: &part_map,
                },
                &inst.name,
                geoms,
                cuts,
                memberships,
            );
            if let Some(entry) = joints.get_mut(joint.index()) {
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
            let mount_face = if source_placed.mount_face.axis() == axis {
                source_placed.mount_face.opposite()
            } else {
                source_placed.mount_face
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
                mechanism,
                own,
                members,
                ..source_placed
            };
            if let Some(entry) = placed.get_mut(hardware.index()) {
                *entry = Some(reflected);
            }
        }
    }
    /// The part that occupies the reflected place of an outside part.
    pub(crate) fn rebind_part(
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

    pub(crate) fn assembly_place(
        &self,
        assembly: AssemblyId,
    ) -> (stave_lang::FileId, stave_lang::ast::Span) {
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
            let (name, assembly, origin, stock, shape, grain, note, loose) =
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
                    let loose = match body.as_ref().and_then(|b| b.loose.clone()) {
                        Some(expr) => {
                            let env = self.part_env(source_body).ok_or(Reported)?;
                            self.eval_bool(&expr, &env)?
                        }
                        None => false,
                    };
                    (
                        inst.name.clone(),
                        inst.assembly,
                        origin,
                        stock,
                        shape,
                        grain,
                        note,
                        loose,
                    )
                } else {
                    let info = made.get(index - self.model.parts.len()).ok_or(Reported)?;
                    // A piece of a spliced part is cut as its part is.
                    let (grain, note, loose) = match info.origin {
                        PartOrigin::Piece { part, .. } => parts
                            .get(part.index())
                            .map_or((Grain::Length, None, false), |first: &solved::Part| {
                                (first.grain, first.note.clone(), first.loose)
                            }),
                        PartOrigin::Declared
                        | PartOrigin::Mirror { .. }
                        | PartOrigin::MirroredAssembly { .. }
                        | PartOrigin::Made { .. }
                        | PartOrigin::Boards { .. } => (Grain::Length, info.note.clone(), false),
                    };
                    (
                        info.name.clone(),
                        info.assembly,
                        info.origin,
                        info.stock,
                        Shape::Board,
                        grain,
                        note,
                        loose,
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
                loose,
                pieces: None,
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
            let members = members(inst.id, &hw.members, &placed_by, hw.piece)?;
            // Around every member; with none, the face it is mounted on.
            let bounds = match members
                .iter()
                .map(solved::Body::bounds)
                .reduce(|all, one| all.union(&one))
            {
                Some(bounds) => bounds,
                None => {
                    let piece = world_box(&self.drawn_box(hw.piece)?)?;
                    let at = piece.face(hw.mount_face);
                    let axis = hw.mount_face.axis();
                    solved::WorldBox::new(piece.min().with(axis, at), piece.max().with(axis, at))
                }
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
                bounds,
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
                members,
                own_mechanism: hw.own.clone(),
                settings: hw
                    .settings
                    .iter()
                    .map(|setting| -> Solved<solved::Setting> {
                        Ok(solved::Setting {
                            body: solved::BodyRef {
                                owner: BodyOwner::Hardware { hardware: inst.id },
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
            let root = inst.assembly.is_none()
                && inst.mirror_of.is_none()
                && self
                    .model
                    .defs
                    .region(inst.def)
                    .is_some_and(|def| crate::place::is_region_box(&def.value));
            out.push(Region {
                id,
                name: inst.name.clone(),
                assembly: inst.assembly,
                bounds: world_box(&value.bounds)?,
                root,
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
        mechanisms: &[Option<AssemblyMechanism>],
    ) -> Solved<Vec<Assembly>> {
        let mut out = Vec::new();
        for inst in &self.model.assemblies {
            let mechanism = mechanisms
                .get(inst.id.index())
                .and_then(Option::as_ref)
                .map(|m| m.mechanism.clone());
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
