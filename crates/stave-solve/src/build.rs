//! Putting it together: runs every step in order and writes the
//! [`SolvedDesign`].

use std::collections::{BTreeMap, HashMap};

use stave_lang::ast::Span;
use stave_lang::placement::{Axis, PartAxis, PerAxis, PerPartAxis};
use stave_lang::units::{ExactLength, Length};
use stave_lang::{Code, Diagnostic, FileId, Label};
use stave_model::defs::{BodyId, DefId, StockField, StockSectionDef, Unusable, Word};
use stave_model::ids::{
    AssemblyId, HardwareId, JointId, PairIndex, PartId, RegionId, SceneId, StockId,
};
use stave_model::instance::Ctx;
use stave_model::mechanism::MemberIndex;
use stave_model::solved::{
    self, Assembly, BodyOwner, Cut, CutPlace, CutSource, DesignDraft, FastenerUse, Hardware,
    HardwareBase, Joint, JointOrigin, JointPair, LocalBox, LocalPoint, PartOrigin, Point,
    PoseValue, ProjectInfo, Region, RequireResult, RoleBinding, Scene, SceneItem, ScenePose, Shape,
    Size3, SolvedDesign, Station, Stock, StockSection, Tool,
};
use stave_model::value::{ExactBox, Value};
use stave_schema::kinds::BaseField;
use stave_schema::words::{Grain, MotionWord, StockForm};

use crate::eval::{Env, midpoint};
use crate::hardware::{
    HardwareCut, MemberCarrier, ORIGIN, Placed, PlacedMember, RoleBound, WorldShape, crossing_face,
    world_shape,
};
use crate::joints::{CutAt, GeomTable, JointWork, Making, PairWork, PartGeom, round_length};
use crate::mechanism::AssemblyMechanism;
use crate::members::{LaidOut, WorldMemberCut};
use crate::mirror_cuts::MirroredJoint;
use crate::place::reflect;
use crate::{Failure, Fault, Solved, Solver};

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

fn point(values: PerAxis<ExactLength>) -> Solved<Point> {
    let PerAxis { across, up, deep } = values;
    Ok(Point {
        across: round_length(across)?,
        up: round_length(up)?,
        deep: round_length(deep)?,
    })
}

fn world_box(bounds: &ExactBox) -> Solved<solved::WorldBox> {
    let low = point(PerAxis::from_fn(|axis| bounds.min(axis)))?;
    let high = point(PerAxis::from_fn(|axis| bounds.max(axis)))?;
    Ok(solved::WorldBox::new(low, high))
}

/// A world point reflected across the plane at `plane` on `axis`.
fn reflect_point(
    point: PerAxis<ExactLength>,
    axis: Axis,
    plane: ExactLength,
) -> Solved<PerAxis<ExactLength>> {
    let mut out = point;
    let twice = plane.checked_add(plane).ok_or(Fault::Overflow)?;
    out[axis] = twice.checked_sub(point[axis]).ok_or(Fault::Overflow)?;
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
            .ok_or(Fault::Internal("the role a mirrored member is with"))?;
        match roles.get(index).map(|(_, bound)| *bound) {
            Some(RoleBound::Part(part)) => Ok(MemberCarrier::Part(part)),
            Some(RoleBound::Region(region)) => Ok(MemberCarrier::Region(region)),
            None => Err(Fault::Internal("the role a mirrored member is with").into()),
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
            WorldShape::Slanted {
                axis: along,
                toward,
                angle,
                diameter,
                length,
                bounds,
            } => WorldShape::Slanted {
                axis: *along,
                toward: crate::hardware::slant_reflected(*along, *toward, axis),
                angle: *angle,
                diameter: *diameter,
                length: *length,
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
                body: member.body,
                name: member.name.clone(),
            })
        })
        .collect()
}

/// A member's shape in the solved design, to the nearest nanometre.
fn member_shape(shape: &WorldShape) -> Solved<solved::MemberShape> {
    let pair = |axis: Axis, point: PerAxis<ExactLength>| -> Solved<[Length; 2]> {
        let [first, second] = axis.others();
        Ok([round_length(point[first])?, round_length(point[second])?])
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
                round_length(midpoint(bounds.min(on), bounds.max(on)).ok_or(Fault::Overflow)?)
            };
            let diameter = bounds
                .max(first)
                .checked_sub(bounds.min(first))
                .ok_or(Fault::Overflow)?;
            let (start, end) = ends(*axis, bounds)?;
            solved::MemberShape::Cylinder {
                axis: *axis,
                centre: [centre(first)?, centre(second)?],
                diameter: round_length(diameter)?,
                start,
                end,
            }
        }
        WorldShape::Slanted {
            axis,
            toward,
            angle,
            diameter,
            length,
            bounds,
        } => {
            let centre = |on: Axis| -> Solved<Length> {
                round_length(midpoint(bounds.min(on), bounds.max(on)).ok_or(Fault::Overflow)?)
            };
            let mut middle = solved::Point::default();
            for on in Axis::ALL {
                middle = middle.with(on, centre(on)?);
            }
            solved::MemberShape::Slanted {
                centre: middle,
                axis: *axis,
                toward: *toward,
                angle: *angle,
                diameter: round_length(*diameter)?,
                length: round_length(*length)?,
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
/// A placed member that its kind's mechanism places is the member of its
/// assembly's mechanism it became; with no mechanism moving it (hardware
/// outside a moving assembly) it stays with the piece.
fn members(
    hardware: HardwareId,
    placed: Vec<PlacedMember>,
    placed_by: &BTreeMap<(HardwareId, BodyId), MemberIndex>,
    piece: PartId,
) -> Solved<Vec<solved::Body>> {
    placed
        .into_iter()
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
                member.name,
                carrier,
                shapes,
            )
            .ok_or_else(|| Fault::Internal("a member's shapes").into())
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
        solved::MemberShape::Slanted {
            centre,
            axis: along,
            toward,
            angle,
            diameter,
            length,
        } => solved::MemberShape::Slanted {
            centre: centre.with(axis, flip_one(centre.get(axis))?),
            axis: *along,
            toward: crate::hardware::slant_reflected(*along, *toward, axis),
            angle: *angle,
            diameter: *diameter,
            length: *length,
        },
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
    // Each is worked out for what it reports; a failure is settled (made
    // sure to reach the user) where the item is declared.
    for (index, inst) in solver.model.params.iter().enumerate() {
        let result = solver.param_value(stave_model::ids::ParamId::from_usize(index));
        let (file, span) = solver.place_of(DefId::Param(inst.def));
        let _ = solver.settled(result, file, span);
    }
    for (index, inst) in solver.model.lets.iter().enumerate() {
        let result = solver.let_value(stave_model::ids::LetId::from_usize(index));
        let (file, span) = solver.place_of(DefId::Let(inst.def));
        let _ = solver.settled(result, file, span);
    }
    for index in 0..part_count {
        let part = PartId::from_usize(index);
        let result = solver.drawn_box(part);
        let (file, span) = solver.part_place(part);
        let _ = solver.settled(result, file, span);
    }
    for (index, inst) in solver.model.regions.iter().enumerate() {
        let result = solver.region_value(RegionId::from_usize(index));
        let (file, span) = solver.place_of(DefId::Region(inst.def));
        let _ = solver.settled(result, file, span);
    }
    // Every profile of the project, used or not, so stations that go
    // backwards are reported once, at the profile.
    for index in 0..solver.model.defs.profiles.len() {
        let profile = stave_model::ids::ProfileId::from_usize(index);
        let in_project = solver
            .model
            .defs
            .decl(DefId::Profile(profile))
            .is_some_and(|decl| {
                solver.module_ctx(DefId::Profile(profile)).is_some()
                    && decl.scope == stave_model::defs::Scope::Module(solver.model.root)
            });
        if in_project {
            let result = solver.profile_stations(profile);
            let (file, span) = solver.place_of(DefId::Profile(profile));
            let _ = solver.settled(result, file, span);
        }
    }

    // Every size of every stock of the project, used or not, so a size of
    // 0mm or less is reported once, at the stock.
    for index in 0..solver.model.defs.stocks.len() {
        let stock = StockId::from_usize(index);
        let in_project = solver
            .model
            .defs
            .decl(DefId::Stock(stock))
            .is_some_and(|decl| {
                solver.module_ctx(DefId::Stock(stock)).is_some()
                    && decl.scope == stave_model::defs::Scope::Module(solver.model.root)
            });
        if !in_project {
            continue;
        }
        for field in [
            StockField::Thickness,
            StockField::Diameter,
            StockField::Width,
            StockField::MaxWidth,
            StockField::MaxLength,
        ] {
            let result = solver.stock_length(stock, field);
            let (file, span) = solver.place_of(DefId::Stock(stock));
            let _ = solver.settled(result, file, span);
        }
        // Its allowances, and `sold:` against its thickness (grammar,
        // "stock").
        let result = solver.stock_allowances(stock);
        let (file, span) = solver.place_of(DefId::Stock(stock));
        let _ = solver.settled(result, file, span);
        // Its `max_held_width:` (grammar, "Grain and wood movement").
        let result = solver.max_held_width(stock);
        let _ = solver.settled(result, file, span);
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
            let result = solver.work_joint(joint);
            let (file, span) = solver.joint_place(joint);
            solver.settled(result, file, span)
        });
    }
    // 3. Geometry of every declared part with what joints add.
    let mut geoms = GeomTable::default();
    for index in 0..part_count {
        let part = PartId::from_usize(index);
        let result = solver.part_geom(part);
        let (file, span) = solver.part_place(part);
        if let Some(geom) = solver.settled(result, file, span) {
            let checked = solver.check_max_length(part, &geom);
            let _ = solver.settled(checked, file, span);
            let checked = solver.check_cut_thickness(part, &geom);
            let _ = solver.settled(checked, file, span);
            let checked = solver.check_turned_blank(part);
            let _ = solver.settled(checked, file, span);
            geoms.set(part, geom);
        }
    }

    solver.warn_missing_stock_limits(&geoms);

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
        let result = solver.finish_joint(
            joint,
            joint_work,
            &mut geoms,
            &mut cuts,
            &mut memberships,
            &mut made,
            &mut fasteners,
        );
        let (file, span) = solver.joint_place(joint);
        if let Some(solved) = solver.settled(result, file, span)
            && let Some(entry) = joints.get_mut(index)
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
        let result = solver.place_hardware(hardware);
        let (file, span) = solver.hardware_place(hardware);
        if let Some(hw) = solver.settled(result, file, span) {
            // Every cut the hardware makes is in its part, or the failure
            // is reported and the design withheld: a cut list never
            // silently misses a bore.
            for cut in &hw.cuts {
                let made = match (geoms.get(cut.part), cuts.get_mut(cut.part.index())) {
                    (Some(geom), Some(list)) => {
                        hardware_cut(geom, cut, hardware).map(|cut| list.push(cut))
                    }
                    // The part failed, and that was reported.
                    _ => Err(Fault::Earlier.into()),
                };
                let _ = solver.settled(made, file, span);
            }
            if let Some(entry) = placed.get_mut(index) {
                *entry = Some(hw);
            }
        }
    }

    // 5b. Edge profiles: each part's chamfers and roundovers, before the
    // mirrored assemblies copy their parts' cuts and splices hand them out.
    solver.make_edge_profiles(&geoms, &mut cuts);
    // 5b'. Shaped parts: each part's taper or arch (grammar, "Shaped
    // parts"), cut from the part like an outline.
    solver.make_shapes(&geoms, &mut cuts);
    // 5c. Edge banding (grammar, "Edge banding"): checked here, taken off
    // only in the cut list.
    let mut banding = solver.make_banding(&geoms);

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
        let Some(inst) = solver.model.requires.get(index) else {
            continue;
        };
        if inst.mirror_of.is_some() {
            continue;
        }
        let def = match solver.model.defs.sound_require(inst.def) {
            Ok(def) => def,
            Err(Unusable::Missing) => continue,
            Err(Unusable::Poisoned) => {
                let _ = solver.skip();
                continue;
            }
        };
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

    // 9. Write everything out. A failure here is settled at the project,
    // so a design withheld is always explained.
    let (file, span) = (solver.project_file, solver.project_span);
    let parts = solver.write_parts(&geoms, cuts, memberships, made, boards, &mut banding);
    let parts = solver.settled(parts, file, span)?;
    let hardware = solver.write_hardware(placed, &mechanisms);
    let hardware = solver.settled(hardware, file, span)?;
    let regions = solver.write_regions();
    let regions = solver.settled(regions, file, span)?;
    let assemblies = solver.write_assemblies(&mechanisms);
    let assemblies = solver.settled(assemblies, file, span)?;
    let scenes = solver.write_scenes(&mechanisms);
    let scenes = solver.settled(scenes, file, span)?;
    let project = solver.write_project();
    let project = solver.settled(project, file, span)?;
    // A joint left out failed, and that failure was settled where it
    // happened.
    let joints: Vec<Joint> = joints.into_iter().collect::<Option<Vec<_>>>()?;
    if solver.diagnostics.iter().filter(|d| d.is_error()).count() > errors_before || solver.skipped
    {
        return None;
    }
    let mut draft = DesignDraft {
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
            let mut overruns = std::mem::take(&mut solver.overruns);
            overruns.sort_by_key(|o| o.part);
            overruns.dedup();
            overruns
        },
    };
    solver.rebind_pieces(&mut draft, splits);
    let design = SolvedDesign::new(draft)
        .map_err(|_| Failure::Fault(Fault::Internal("a solved design's tables")));
    solver.settled(design, file, span)
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
            slant: cut.slant,
            kind: cut.kind.clone(),
            wall: cut.wall,
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
    /// The shape's slant, when it is a slanted cylinder.
    slant: Option<crate::hardware::WorldSlant>,
    kind: solved::CutKind,
    /// The least wall its operation asks for.
    wall: Option<stave_lang::units::Length>,
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
        let [length, width, thickness] = geom.local_box(world)?.into_array();
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
    // A drill follows the slant of its member: going deeper into the
    // part, it leans toward the slant's direction, or away from it when
    // the hole goes in toward the negative end of the axis.
    let kind = match (cut.kind, cut.slant) {
        (
            solved::CutKind::Drill {
                diameter,
                depth,
                from_depth,
                tilt: _,
            },
            Some(slant),
        ) => {
            let inward_positive = !cut.face.is_positive();
            let lean = if inward_positive {
                slant.toward
            } else {
                slant.toward.opposite()
            };
            solved::CutKind::Drill {
                diameter,
                depth,
                from_depth,
                tilt: Some(solved::Tilt {
                    angle: slant.angle,
                    toward: geom.face_toward(lean.axis(), lean.is_positive()),
                }),
            }
        }
        (kind, _) => kind,
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
        kind,
        wall: cut.wall,
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
    let copies = u32::try_from(member.copies.len()).map_err(|_| Fault::Overflow)?;
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
    .ok_or_else(|| Fault::Internal("a member's shapes").into())
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
        let Some(inst) = self.model.joints.get(joint.index()) else {
            return Err(Fault::Internal("a joint").into());
        };
        let kind_id = work.binding.kind;
        let kind = self.sound(self.model.defs.sound_kind(kind_id), "a kind's declaration")?;
        let Some(ctx) = self.module_ctx(DefId::Kind(kind_id)) else {
            return Err(Fault::Internal("the file an item is declared in").into());
        };
        // The resolver gives every joint, splice and method a joint kind.
        let Some(joint_def) = kind.joint() else {
            return Err(Fault::Internal("a joint kind's entries").into());
        };
        let pair_count = work.pairs.len();
        let mut pairs = Vec::new();
        let mut note = None;
        let mut fastener_totals: Vec<(stave_model::ids::KindId, Length, Length, u32)> = Vec::new();
        for (pair_index, pair) in work.pairs.iter_mut().enumerate() {
            let pair_index = PairIndex::from_usize(pair_index);
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
                    roles: pair_roles(kind, pair),
                    contact: pair.contact,
                    extensions: Vec::new(),
                    cut_order: Vec::new(),
                    insert: Vec::new(),
                    made_parts: Vec::new(),
                    requires: work
                        .requires
                        .get(pair_index.index())
                        .cloned()
                        .unwrap_or_default(),
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
                let at = self.joint_place(joint);
                let placed = self.place_made_part(made_def, pair, geoms, &env, &name, at)?;
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
            pair.env.made = made_ids.clone().into();
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
                let (file, span) = self.joint_place(joint);
                let at = CutAt {
                    owner: &inst.name,
                    file,
                    span,
                };
                for (part, face, place, cut_kind, wall) in
                    self.pair_cuts(cut, pair, geoms, &env, at)?
                {
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
                            wall,
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
            let laid_out = self.lay_out_members(kind, &env, &inst.name, place)?;
            for cut in &joint_def.member_cuts {
                let (Some(part), Some(member)) = (
                    pair.parts.get(cut.part.index()).copied(),
                    laid_out.iter().find(|member| member.body == cut.body),
                ) else {
                    return Err(Fault::Internal("a part").into());
                };
                let mut kinds = Vec::new();
                let mut walls = Vec::new();
                for operation in &cut.operations {
                    let kind = self.cut_kind(operation, &env)?;
                    walls.push(self.cut_wall(operation, &env)?);
                    let at = CutAt {
                        owner: &inst.name,
                        file: place.0,
                        span: place.1,
                    };
                    self.check_cut_size(at, cut.name.as_ref(), operation, &kind, part)?;
                    kinds.push(kind);
                }
                let Some(geom) = geoms.get(part).cloned() else {
                    return Err(Fault::Earlier.into());
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
                            ).into());
                        };
                        let slant = crate::hardware::WorldSlant::of(&crate::hardware::world_shape(
                            shape,
                            &copy.frame,
                        )?);
                        self.check_slanted_cut(&crate::hardware::SlantedCut {
                            owner: &inst.name,
                            member: &member.name,
                            part,
                            face,
                            slant,
                            kinds: &kinds,
                            file: place.0,
                            span: place.1,
                        })?;
                        for (cut_kind, wall) in kinds.iter().zip(&walls) {
                            work.member_cuts.push(WorldMemberCut {
                                pair: pair_index,
                                part,
                                body: cut.body,
                                name: cut.name.clone(),
                                face,
                                shape: shape.world,
                                slant,
                                kind: cut_kind.clone(),
                                wall: *wall,
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
                                    slant,
                                    kind: cut_kind.clone(),
                                    wall: *wall,
                                },
                            )?;
                            if let Some(list) = cuts.get_mut(part.index()) {
                                list.push(made);
                            }
                        }
                    }
                }
            }
            let carrier = pair.parts.first().copied().ok_or(Fault::Earlier)?;
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
                    if let Some(part) = pair.parts.get(role.index())
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
                    let count = u32::try_from(member.copies.len()).map_err(|_| Fault::Overflow)?;
                    counted.push((item.clone(), count, written.span));
                }
            }
            for (fastener, count, span) in counted {
                let file = kind.decl.file;
                let diameter = match self.base_field(&fastener, BaseField::Diameter, file, span)? {
                    Some(Value::Length(l)) => round_length(l)?,
                    _ => return Err(Fault::Earlier.into()),
                };
                let length = match self.base_field(&fastener, BaseField::Length, file, span)? {
                    Some(Value::Length(l)) => round_length(l)?,
                    _ => return Err(Fault::Earlier.into()),
                };
                match fastener_totals
                    .iter_mut()
                    .find(|(k, d, l, _)| *k == fastener.kind && *d == diameter && *l == length)
                {
                    Some(entry) => entry.3 = entry.3.saturating_add(count),
                    None => fastener_totals.push((fastener.kind, diameter, length, count)),
                }
            }
            if note.is_none()
                && let Some(text) = &joint_def.note
            {
                note = Some(self.format_text(text, &env)?);
            }
            pairs.push(JointPair {
                roles: pair_roles(kind, pair),
                contact: pair.contact,
                extensions: std::mem::take(&mut pair.extensions),
                cut_order,
                insert,
                made_parts: made_ids,
                requires: work
                    .requires
                    .get(pair_index.index())
                    .cloned()
                    .unwrap_or_default(),
                members,
            });
        }
        for (kind, diameter, length, count) in fastener_totals {
            let name = self.def_name(DefId::Kind(kind));
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
            .filter_map(|r| {
                self.model
                    .defs
                    .decl(DefId::Recipe(*r))
                    .map(|decl| decl.name.clone())
            })
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
            unmatched: std::mem::take(&mut work.unmatched),
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
        let def = self.sound(
            self.model.defs.sound_kind(item.kind),
            "a kind's declaration",
        )?;
        let kind = def.decl.name.clone();
        let wood = match self.base_field(item, BaseField::Species, file, span)? {
            Some(Value::Word(stave_model::defs::Word::Species(species))) => Some(species),
            _ => None,
        };
        Ok(solved::MemberItem { kind, wood })
    }

    /// A kind's name, qualified with its module unless it is from the
    /// prelude or the project.
    fn kind_name(&self, kind: stave_model::ids::KindId) -> String {
        let Some(decl) = self.model.defs.decl(DefId::Kind(kind)) else {
            return String::new();
        };
        match decl.scope {
            stave_model::defs::Scope::Module(module)
                if module != self.model.root && !self.is_prelude(module) =>
            {
                let module_name = self
                    .model
                    .defs
                    .modules
                    .get(module.index())
                    .map_or("", |m| m.name.as_str());
                format!("{module_name}.{}", decl.name)
            }
            stave_model::defs::Scope::Module(_) | stave_model::defs::Scope::Component(_) => {
                decl.name.clone()
            }
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
                        CutSource::Edges { part } => CutSource::Edges {
                            part: *part_map.get(&part).unwrap_or(&part),
                        },
                        CutSource::Shape { part } => CutSource::Shape {
                            part: *part_map.get(&part).unwrap_or(&part),
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
                        edge @ CutPlace::Edge { .. } => edge,
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
            let Some(Some(mut copy)) = joints.get(source.index()).cloned() else {
                continue;
            };
            let Some(inst) = self.model.joints.get(joint.index()) else {
                continue;
            };
            let Some(assembly) = inst.assembly else {
                continue;
            };
            // Every failure from here is settled at the joint: a mirrored
            // joint left out withholds the design, and says why.
            let (file, span) = self.joint_place(joint);
            let plane = self.assembly_mirror_plane(assembly);
            let Some((axis, plane)) = self.settled(plane, file, span) else {
                continue;
            };
            copy.id = joint;
            let source_name = std::mem::replace(&mut copy.name, inst.name.clone());
            copy.assembly = Some(assembly);
            let mut failed = false;
            let mut map = |this: &mut Self, part: PartId| -> PartId {
                if let Some(mapped) = part_map.get(&part) {
                    return *mapped;
                }
                match this.rebind_part(part, axis, plane, assembly) {
                    Ok(found) => found,
                    Err(failure) => {
                        let _ = this.settle(failure, file, span);
                        failed = true;
                        part
                    }
                }
            };
            let mut unreflected = false;
            for (pair_index, pair) in copy.pairs.iter_mut().enumerate() {
                let pair_index = PairIndex::from_usize(pair_index);
                // The members are reflected and belong to the copy.
                let mut members = Vec::with_capacity(pair.members.len());
                for member in &pair.members {
                    let carrier = match member.carrier {
                        solved::BodyCarrier::Part(part) => {
                            solved::BodyCarrier::Part(map(self, part))
                        }
                        other @ (solved::BodyCarrier::Region(_) | solved::BodyCarrier::Link(_)) => {
                            other
                        }
                    };
                    let shapes = member
                        .shapes()
                        .iter()
                        .map(|shape| reflect_member_shape(shape, axis, plane))
                        .collect::<Solved<Vec<_>>>();
                    let body = shapes.and_then(|shapes| {
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
                        .ok_or_else(|| Fault::Internal("a mirrored member's shapes").into())
                    });
                    match self.settled(body, file, span) {
                        Some(body) => members.push(body),
                        None => unreflected = true,
                    }
                }
                pair.members = members;
                for require in &mut pair.requires {
                    require.name = require.name.replacen(&source_name, &inst.name, 1);
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
                .map_or(&[][..], |w| w.member_cuts.as_slice());
            for cut in source_cuts {
                if part_map.contains_key(&cut.part) {
                    continue;
                }
                let rebound = self.rebind_part(cut.part, axis, plane, assembly);
                let Some(part) = self.settled(rebound, file, span) else {
                    continue;
                };
                let shape = reflect_box(&cut.shape, axis, plane);
                let Some(shape) = self.settled(shape, file, span) else {
                    continue;
                };
                let Some(geom) = geoms.get(part) else {
                    // The part failed, and that was reported.
                    let _ = self.settle(Fault::Earlier.into(), file, span);
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
                        name: cut.name.clone(),
                        face,
                        shape: &shape,
                        slant: cut.slant.map(|slant| slant.reflected(axis)),
                        kind: cut.kind.clone(),
                        wall: cut.wall,
                    },
                );
                let made = made.and_then(|made| match cuts.get_mut(part.index()) {
                    Some(list) => {
                        list.push(made);
                        Ok(())
                    }
                    None => Err(Fault::Internal("the cuts of a part").into()),
                });
                let _ = self.settled(made, file, span);
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
            let Some(inst) = self.model.hardware.get(hardware.index()) else {
                continue;
            };
            let Some(assembly) = inst.assembly else {
                continue;
            };
            // Every failure from here is settled at the hardware: hardware
            // left out withholds the design, and says why.
            let (file, span) = self.hardware_place(hardware);
            let plane = self.assembly_mirror_plane(assembly);
            let Some((axis, plane)) = self.settled(plane, file, span) else {
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
                match self.settled(rebound, file, span) {
                    Some(rebound) => roles.push((role.clone(), rebound)),
                    None => {
                        unbound = true;
                        break;
                    }
                }
            }
            if unbound {
                continue;
            }
            let Some((_, RoleBound::Part(piece))) = roles.first().cloned() else {
                let _ = self.settle(Fault::Earlier.into(), file, span);
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
                    None => {
                        let _ = self.settle(Fault::Overflow.into(), file, span);
                        continue;
                    }
                },
                None => None,
            };
            let own = match &mechanism {
                Some(reflected) => {
                    let checked =
                        self.check_mechanism(hardware, &roles, reflected, &source_placed.kind_name);
                    match self.settled(checked, file, span) {
                        Some(own) => own,
                        None => continue,
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
                    None => {
                        let rebound = self.rebind_part(cut.part, axis, plane, assembly);
                        match self.settled(rebound, file, span) {
                            Some(found) => (found, true),
                            None => {
                                unbound = true;
                                break;
                            }
                        }
                    }
                };
                let shape = reflect_box(&cut.shape, axis, plane);
                let Some(shape) = self.settled(shape, file, span) else {
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
                    slant: cut.slant.map(|slant| slant.reflected(axis)),
                    ..cut.clone()
                };
                if outside {
                    // The cut is in its part, or the failure is reported
                    // and the design withheld.
                    let made = match (geoms.get(part), cuts.get_mut(part.index())) {
                        (Some(geom), Some(list)) => {
                            hardware_cut(geom, &reflected_cut, hardware).map(|cut| list.push(cut))
                        }
                        // The part failed, and that was reported.
                        _ => Err(Fault::Earlier.into()),
                    };
                    let _ = self.settled(made, file, span);
                }
                reflected_cuts.push(reflected_cut);
            }
            if unbound {
                continue;
            }
            let members = reflect_members(
                &source_placed.members,
                &source_placed.roles,
                &roles,
                axis,
                plane,
            );
            let Some(members) = self.settled(members, file, span) else {
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
            // A part whose box fails was settled when every part was
            // placed (step 1 of `build`); it is no counterpart.
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
        ).into())
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
        Err(self
            .error(
                Code::Mirror,
                format!("{name} has no counterpart on the other side of the mirror"),
                file,
                span,
                "make the design symmetric about the mirror plane",
            )
            .into())
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
        let def = self.sound(self.model.defs.sound_stock(stock), "a stock's declaration")?;
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
            section: match &def.section {
                Some(StockSectionDef::Board { width, .. }) => StockSection::Board {
                    thickness: length(self, F::Thickness)?.ok_or(Fault::Earlier)?,
                    width: match width {
                        Some(_) => length(self, F::Width)?,
                        None => None,
                    },
                },
                Some(StockSectionDef::Dowel { .. }) => StockSection::Dowel {
                    diameter: length(self, F::Diameter)?.ok_or(Fault::Earlier)?,
                },
                None => return Err(Fault::Earlier.into()),
            },
            max_width: length(self, F::MaxWidth)?,
            max_length: length(self, F::MaxLength)?,
            max_held_width: self.max_held_width(stock)?,
            allowances: self.stock_allowances(stock)?,
        })
    }

    fn write_parts(
        &mut self,
        geoms: &GeomTable,
        cuts: Vec<Vec<Cut>>,
        memberships: Vec<Vec<JointId>>,
        made: Vec<MadeInfo>,
        mut boards: Vec<Option<solved::Boards>>,
        banding: &mut [Option<solved::Banding>],
    ) -> Solved<Vec<solved::Part>> {
        let mut parts = Vec::new();
        let mut cuts = cuts.into_iter();
        let mut memberships = memberships.into_iter();
        let total = self.model.parts.len() + made.len();
        // Made parts follow the declared ones, in order.
        let mut made = made.into_iter();
        for index in 0..total {
            let id = PartId::from_usize(index);
            let part_cuts = cuts.next().unwrap_or_default();
            let part_joints = memberships.next().unwrap_or_default();
            let Some(geom) = geoms.get(id).cloned() else {
                return Err(Fault::Earlier.into());
            };
            let (name, assembly, origin, stock, shape, grain, note, loose) =
                if let Some(inst) = self.model.parts.get(index) {
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
                            let def = self.sound(
                                self.model.defs.sound_profile(profile),
                                "a profile's declaration",
                            )?;
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
                                    .sound_stock(stock)
                                    .is_ok_and(|s| s.form == StockForm::Dowel) =>
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
                    let body = self.part_body(source_body);
                    let grain = body.map_or(Grain::Length, |b| b.grain);
                    let note = match body.and_then(|b| b.note.as_ref()) {
                        Some(text) => {
                            let env = self
                                .part_env(source_body)
                                .ok_or(Fault::Internal("where a part is declared"))?;
                            Some(self.format_text(text, &env)?)
                        }
                        None => None,
                    };
                    let loose = match body.and_then(|b| b.loose.as_ref()) {
                        Some(expr) => {
                            let env = self
                                .part_env(source_body)
                                .ok_or(Fault::Internal("where a part is declared"))?;
                            self.eval_bool(expr, &env)?
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
                    let info = made.next().ok_or(Fault::Internal("a made part"))?;
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
                        | PartOrigin::Boards { .. } => (Grain::Length, info.note, false),
                    };
                    (
                        info.name,
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
            let size = |values: PerPartAxis<ExactLength>| -> Solved<Size3> {
                let [length, width, thickness] = values.into_array();
                Ok(Size3 {
                    length: round_length(length)?,
                    width: round_length(width)?,
                    thickness: round_length(thickness)?,
                })
            };
            let cut_values = PerPartAxis::try_from_fn(|axis| geom.cut_size(axis))?;
            let mut origin_values = ORIGIN;
            for axis in PartAxis::ALL {
                let direction = geom.orientation.direction(axis);
                let world = direction.axis();
                let value = if direction.is_positive() {
                    geom.cut_box.min(world)
                } else {
                    geom.cut_box.max(world)
                };
                origin_values[world] = value;
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
                banding: banding.get_mut(index).and_then(Option::take),
            });
        }
        Ok(parts)
    }

    fn write_hardware(
        &mut self,
        placed: Vec<Option<Placed>>,
        mechanisms: &[Option<AssemblyMechanism>],
    ) -> Solved<Vec<Hardware>> {
        let placed_by: BTreeMap<(HardwareId, BodyId), MemberIndex> = mechanisms
            .iter()
            .flatten()
            .flat_map(|m| m.bodies.iter().map(|(key, index)| (*key, *index)))
            .collect();
        let mut out = Vec::new();
        for (index, entry) in placed.into_iter().enumerate() {
            let Some(hw) = entry else {
                return Err(Fault::Earlier.into());
            };
            let Some(inst) = self.model.hardware.get(index) else {
                return Err(Fault::Internal("a piece of hardware").into());
            };
            let base = HardwareBase::from_base(hw.base)
                .ok_or(Fault::Internal("a piece of hardware's base"))?;
            let members = members(inst.id, hw.members, &placed_by, hw.piece)?;
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
                kind: hw.kind_name,
                kind_id: hw.kind,
                base,
                assembly: inst.assembly,
                piece: hw.piece,
                mount_face: hw.mount_face,
                bounds,
                model: hw.model,
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
                fasteners: hw.fasteners,
                members,
                own_mechanism: hw.own,
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
                    .into_iter()
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
                            Value::Bool(value) => solved::EntryValue::Bool(value),
                            Value::Text(text) => solved::EntryValue::Text(text),
                            Value::Count(_)
                            | Value::Ratio(_)
                            | Value::Part(_)
                            | Value::Parts(_)
                            | Value::Region(_)
                            | Value::Regions(_)
                            | Value::Stock(_)
                            | Value::Profile(_)
                            | Value::Anchor(_)
                            | Value::Word(_)
                            | Value::Mount(_)
                            | Value::Turned(_)
                            | Value::Shaped(_)
                            | Value::Boards(_)
                            | Value::Splices(_)
                            | Value::Envelope(_)
                            | Value::Model(_)
                            | Value::Spacing(_)
                            | Value::Kind(_)
                            | Value::Tool(_)
                            | Value::Recipe(_)
                            | Value::Assembly(_)
                            | Value::Assemblies(_)
                            | Value::Joint(_)
                            | Value::Hardware(_)
                            | Value::List(_)
                            | Value::Patch(_) => return None,
                        };
                        Some((solved::EntryName::new(&name)?, value))
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
                return Err(Fault::Internal("a region").into());
            };
            let root = inst.assembly.is_none()
                && inst.mirror_of.is_none()
                && self
                    .model
                    .defs
                    .sound_region(inst.def)
                    .is_ok_and(|def| crate::place::is_region_box(&def.value));
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
            let component = self.def_name(DefId::Component(inst.component));
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

    /// The parts, assemblies and hardware a scene's `show:` or `hide:`
    /// names. The resolver refuses other types; a value that still gets
    /// here (its type was unknown until solved) is a `type-mismatch`, never
    /// dropped, and every variant is listed so a new one must be decided.
    fn scene_items(
        &mut self,
        value: &Value,
        out: &mut Vec<SceneItem>,
        field: &str,
        file: FileId,
        span: Span,
    ) -> Solved<()> {
        match value {
            Value::Part(part) => out.push(SceneItem::Part(*part)),
            Value::Parts(parts) => out.extend(parts.iter().map(|p| SceneItem::Part(*p))),
            Value::Assembly(assembly) => out.push(SceneItem::Assembly(*assembly)),
            Value::Assemblies(list) => out.extend(list.iter().map(|a| SceneItem::Assembly(*a))),
            Value::Hardware(hardware) => out.push(SceneItem::Hardware(*hardware)),
            Value::List(items) => {
                for item in items {
                    self.scene_items(item, out, field, file, span)?;
                }
            }
            Value::Length(_)
            | Value::Angle(_)
            | Value::Count(_)
            | Value::Ratio(_)
            | Value::Bool(_)
            | Value::Text(_)
            | Value::Region(_)
            | Value::Regions(_)
            | Value::Stock(_)
            | Value::Profile(_)
            | Value::Anchor(_)
            | Value::Word(_)
            | Value::Mount(_)
            | Value::Turned(_)
            | Value::Shaped(_)
            | Value::Boards(_)
            | Value::Splices(_)
            | Value::Envelope(_)
            | Value::Model(_)
            | Value::Spacing(_)
            | Value::Kind(_)
            | Value::Tool(_)
            | Value::Recipe(_)
            | Value::Joint(_)
            | Value::Patch(_) => {
                return Err(self
                    .error(
                        Code::TypeMismatch,
                        format!(
                            "'{field}' needs parts, assemblies or hardware, found {}",
                            value.description()
                        ),
                        file,
                        span,
                        format!(
                            "name the parts, assemblies or hardware to {field}, or a list of them"
                        ),
                    )
                    .into());
            }
        }
        Ok(())
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
            .ok_or(Fault::Internal("the project's own items"))?;
        let model = self.model;
        for &scene in &model.scenes {
            // A scene with a name or type error is already reported.
            let def = match self.model.defs.sound_scene(scene) {
                Ok(def) => def,
                Err(Unusable::Missing) => continue,
                Err(Unusable::Poisoned) => {
                    let _ = self.skip();
                    continue;
                }
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
                    return Err(self
                        .error(
                            Code::TypeMismatch,
                            "'pose' needs an assembly",
                            def.decl.file,
                            pose.target.span,
                            "name an assembly with hinges or slides",
                        )
                        .into());
                };
                let value = match self.eval(&pose.value, &env)? {
                    Value::Word(Word::Motion(MotionWord::Open)) => PoseValue::Open,
                    Value::Word(Word::Motion(MotionWord::Closed)) => PoseValue::Closed,
                    Value::Angle(angle) => {
                        PoseValue::Angle(angle.to_nearest_angle().ok_or(Fault::Overflow)?)
                    }
                    Value::Length(length) => PoseValue::Travel(round_length(length)?),
                    Value::Count(_)
                    | Value::Ratio(_)
                    | Value::Bool(_)
                    | Value::Text(_)
                    | Value::Part(_)
                    | Value::Parts(_)
                    | Value::Region(_)
                    | Value::Regions(_)
                    | Value::Stock(_)
                    | Value::Profile(_)
                    | Value::Anchor(_)
                    | Value::Word(_)
                    | Value::Mount(_)
                    | Value::Turned(_)
                    | Value::Shaped(_)
                    | Value::Boards(_)
                    | Value::Splices(_)
                    | Value::Envelope(_)
                    | Value::Model(_)
                    | Value::Spacing(_)
                    | Value::Kind(_)
                    | Value::Tool(_)
                    | Value::Recipe(_)
                    | Value::Assembly(_)
                    | Value::Assemblies(_)
                    | Value::Joint(_)
                    | Value::Hardware(_)
                    | Value::List(_)
                    | Value::Patch(_) => return Err(Fault::Earlier.into()),
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
                    self.scene_items(&value, &mut items, "show", def.decl.file, expr.span)?;
                    Some(items)
                }
                None => None,
            };
            let hide = match &def.hide {
                Some(expr) => {
                    let value = self.eval(expr, &env)?;
                    let mut items = Vec::new();
                    self.scene_items(&value, &mut items, "hide", def.decl.file, expr.span)?;
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
        let Some(project) = self.model.defs.project(self.model.root) else {
            return Err(Fault::Internal("the project's declaration").into());
        };
        let root = self
            .model
            .module_instance(self.model.root)
            .ok_or(Fault::Internal("the project's own items"))?;
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
                    return Err(self
                        .error(
                            Code::OutOfRange,
                            format!("'resolution' must be more than 0mm, found {value}"),
                            project.file,
                            expr.span,
                            "write a positive length, such as 0.5mm",
                        )
                        .into());
                }
                value
            }
            None => Length::from_nm(stave_schema::fields::DEFAULT_RESOLUTION_NM),
        };
        let tools = match &project.tools {
            Some(list) => {
                let mut tools = Vec::new();
                for tool in list {
                    if self.model.defs.decl(DefId::Kind(*tool)).is_none() {
                        continue;
                    }
                    let kind_value =
                        stave_model::value::KindValue::new(*tool, self.kind_name(*tool));
                    let title = match self.base_field(
                        &kind_value,
                        BaseField::Title,
                        project.file,
                        project.span,
                    )? {
                        Some(Value::Text(text)) => Some(text),
                        _ => None,
                    };
                    let powered = self.base_field(
                        &kind_value,
                        BaseField::Powered,
                        project.file,
                        project.span,
                    )? == Some(Value::Bool(true));
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
            .filter_map(|r| {
                self.model
                    .defs
                    .decl(DefId::Recipe(*r))
                    .map(|decl| decl.name.clone())
            })
            .collect();
        let allowances = self.project_allowances()?;
        Ok(ProjectInfo {
            title: project.title.clone(),
            goal: project.goal.clone(),
            finish: project.finish.clone(),
            units: project.units,
            resolution,
            style: project.style.clone(),
            tools,
            methods,
            kerf: allowances.kerf,
            rough_length: allowances.rough_length,
            rough_width: allowances.rough_width,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use stave_lang::Code;
    use stave_model::Model;
    use stave_model::ids::PartId;
    use stave_model::value::Value;
    use stave_modules::{MemoryFiles, ModuleSet};

    use crate::Solver;

    const PROJECT: &str = "stave 1

project \"Test\" {
  units: mm
}

stock pine_19 { species: pine, form: solid, thickness: 19mm, max_length: 2400mm }

part base : pine_19 {
  length: 600mm
  width:  300mm
  place {
    ground
  }
}
";

    fn model() -> Model {
        let files = MemoryFiles::new().with("/project/project.stave", PROJECT);
        let modules = ModuleSet::load_with(Path::new("/project"), &files).unwrap();
        let (defs, diagnostics) = stave_model::resolve(&modules);
        assert!(diagnostics.iter().all(|d| !d.is_error()), "{diagnostics:?}");
        let (model, diagnostics) = stave_model::instance::instantiate(defs, modules.root);
        assert!(diagnostics.iter().all(|d| !d.is_error()), "{diagnostics:?}");
        model
    }

    /// A value the resolver could not type (it was unknown until solved)
    /// and that is not a scene item is a `type-mismatch`, not an empty
    /// scene.
    #[test]
    fn a_scene_shows_only_parts_assemblies_and_hardware() {
        let model = model();
        let project = model.defs.project(model.root).unwrap();
        let mut solver = Solver::new(&model, project.file, project.span);
        let (file, span) = (project.file, project.span);
        let part = PartId::from_usize(0);
        let mut items = Vec::new();
        let shown = Value::List(vec![Value::Part(part), Value::Parts(vec![part])]);
        assert!(
            solver
                .scene_items(&shown, &mut items, "show", file, span)
                .is_ok()
        );
        assert_eq!(items.len(), 2);
        assert!(solver.diagnostics.is_empty());

        let wrong = Value::List(vec![Value::Part(part), Value::Bool(true)]);
        assert!(
            solver
                .scene_items(&wrong, &mut items, "hide", file, span)
                .is_err()
        );
        let [diagnostic] = solver.diagnostics.as_slice() else {
            panic!("{:?}", solver.diagnostics);
        };
        assert_eq!(diagnostic.code, Code::TypeMismatch);
        assert_eq!(
            diagnostic.message,
            "'hide' needs parts, assemblies or hardware, found true or false"
        );
        assert!(diagnostic.help.is_some());
    }
}
