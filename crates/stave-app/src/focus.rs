//! Focus mode: one joint or one piece of hardware, shown so a beginner can
//! see how it goes together or how it works.
//!
//! Everything else fades to a ghost, the part that receives a joint is
//! drawn see-through (X-ray), and one scrub amount from 0 to 1 drives it:
//!
//! - **A joint pair** plays its own `insert:` steps, the same steps the
//!   assembly check runs backwards ([`stave_geom::sequence::trace`]): 0 is
//!   apart, 1 together, each step an equal share. The moving part's home is
//!   outlined, and the panel names the step and how far the part is from
//!   home.
//! - **A piece of hardware** drives its assembly's one motion (a door's
//!   angle, a drawer's travel), every member following; 0 is closed, 1
//!   fully open. The path of the door's front edge (or the drawer's front)
//!   is dotted, the kind's members are numbered, and the panel gives how
//!   far it opens and the smallest gap to the parts around it over its
//!   whole travel ([`stave_geom::clearance`]).
//!
//! The words come from the language: a joint kind's and a hardware kind's
//! doc comments, a kind's `insert:` steps as written, and a hardware kind's
//! documented `member`s.

use stave_geom::clearance::{box_centre, closest_for};
use stave_geom::collide::{CONTACT_TOLERANCE, WorldSolid, overlaps, touches};
use stave_geom::sequence::{Member, pair_members, pair_steps, trace};
use stave_geom::solid::Solid;
use stave_lang::Code;
use stave_lang::placement::Direction;
use stave_lang::units::Length;
use stave_model::defs::KindDef;
use stave_model::solved::{
    AssemblyId, CutKind, CutSource, Depth, Hardware, HardwareBase, HardwareData, HardwareId,
    JointId, Motion, Part, PartId, SolvedDesign, StepAmount, StepKind,
};
use stave_scene::focus::{
    Anchor, GapMark, MotionPlay, Plan, Play, StepPlay, amount_text, around, edge_point, hinge_view,
    key_poses, motion_zone,
};
use stave_scene::{Aabb, Item, Look, MotionRange, NodeId, Overlay, Posed, SceneEntry, SceneGraph};
use stave_schema::kinds::MemberName;

use crate::Opened;
use crate::session::{LineNumber, SourceLocation};
use crate::view::Row;

/// What focus mode shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Target {
    /// One pair of a joint.
    Joint {
        /// The joint.
        joint: JointId,
        /// The pair's index in the joint.
        pair: usize,
    },
    /// A piece of hardware.
    Hardware(HardwareId),
}

/// Why a name doesn't pick out something to focus on.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FindError {
    /// No joint or hardware has that name.
    #[error("no joint or hardware is called '{name}'; try one of: {names}")]
    NotFound {
        /// The name asked for.
        name: String,
        /// Some names that are there.
        names: String,
    },
    /// The joint has fewer pairs.
    #[error("joint '{name}' has {count} pairs, numbered from 0; there is no pair {pair}")]
    NoPair {
        /// The joint.
        name: String,
        /// Its number of pairs.
        count: usize,
        /// The pair asked for.
        pair: usize,
    },
}

/// The joint (and pair, default the first) or piece of hardware with this
/// name.
pub fn find(design: &SolvedDesign, name: &str, pair: Option<usize>) -> Result<Target, FindError> {
    if let Some(joint) = design.joints.iter().find(|j| j.name == name) {
        let index = pair.unwrap_or(0);
        if index >= joint.pairs.len() {
            return Err(FindError::NoPair {
                name: name.to_owned(),
                count: joint.pairs.len(),
                pair: index,
            });
        }
        return Ok(Target::Joint {
            joint: joint.id,
            pair: index,
        });
    }
    if let Some(hardware) = design.hardware.iter().find(|h| h.name == name) {
        return Ok(Target::Hardware(hardware.id));
    }
    let names: Vec<&str> = design
        .joints
        .iter()
        .map(|j| j.name.as_str())
        .chain(design.hardware.iter().map(|h| h.name.as_str()))
        .take(12)
        .collect();
    Err(FindError::NotFound {
        name: name.to_owned(),
        names: names.join(", "),
    })
}

/// What can be focused on from a selected part or piece of hardware: every
/// joint pair the part is in, or the hardware itself, each with a short
/// label ("rung[front] into leg[front_left] (socket)").
pub fn targets_of(design: &SolvedDesign, item: Item) -> Vec<(Target, String)> {
    match item {
        Item::Part(part) => design
            .joints
            .iter()
            .flat_map(|joint| {
                joint
                    .pairs
                    .iter()
                    .enumerate()
                    .filter(move |(_, pair)| {
                        pair.roles.iter().any(|r| r.part == part) || pair.made_parts.contains(&part)
                    })
                    .map(move |(index, _)| Target::Joint {
                        joint: joint.id,
                        pair: index,
                    })
            })
            .map(|target| (target, title(design, target)))
            .collect(),
        Item::Hardware(hardware) => {
            let target = Target::Hardware(hardware);
            vec![(target, title(design, target))]
        }
    }
}

/// A target's short title: "rung[front] into leg[front_left] (socket)",
/// "door_l.hinges[upper] (euro_hinge)".
pub fn title(design: &SolvedDesign, target: Target) -> String {
    let name = |id: PartId| design.part(id).map_or("?", |p| p.name.as_str());
    match target {
        Target::Joint { joint, pair } => {
            let Some(joint) = design.joints.get(joint.index()) else {
                return String::new();
            };
            let Some(pair) = joint.pairs.get(pair) else {
                return joint.name.clone();
            };
            let parts: Vec<&str> = pair.roles.iter().map(|r| name(r.part)).collect();
            match parts.as_slice() {
                [piece, into] => format!("{piece} into {into} ({})", joint.kind),
                _ => format!("{} ({})", parts.join(", "), joint.kind),
            }
        }
        Target::Hardware(id) => design
            .hardware
            .get(id.index())
            .map_or_else(String::new, |h| format!("{} ({})", h.name, h.kind)),
    }
}

/// How focus mode draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    /// Draw the receiving part see-through.
    pub xray: bool,
    /// Tint the faces cuts made.
    pub tint_cut_faces: bool,
    /// Fade everything else to ghosts.
    pub fade_others: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            xray: true,
            tint_cut_faces: true,
            fade_others: true,
        }
    }
}

/// A numbered, captioned member of a piece of hardware.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberRow {
    /// Its number on the drawing, from 1.
    pub number: u8,
    /// Its name: `plate`.
    pub name: &'static str,
    /// Its doc comment.
    pub caption: String,
}

/// A joint kind's `insert:` as written, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InsertText {
    /// `insert: [push(piece, along: length)]`
    pub text: String,
    /// `std/joinery/socket.stave:9`
    pub source: String,
    /// Each step as written: `push(piece, along: length)`.
    pub steps: Vec<String>,
    /// The link to the kind: `std:joinery/socket` for the standard
    /// library, the file's name for a project's own kind.
    pub link: String,
    /// The file and line, for the link.
    pub location: Option<SourceLocation>,
}

/// A pose to show as a small picture beside the main view.
#[derive(Debug, Clone, PartialEq)]
pub struct KeyPose {
    /// The scrub amount, 0 to 1.
    pub progress: f32,
    /// "0°", "45°", "110°", or "250 mm".
    pub caption: String,
}

/// A joint pair or a piece of hardware in focus.
#[derive(Debug, Clone, PartialEq)]
pub struct Focus {
    /// What it is.
    pub target: Target,
    /// "Stool · Joint focus · rung[front] into leg[front_left] (socket)".
    pub context: String,
    /// "rung[front] into leg[front_left] (socket)".
    pub title: String,
    /// The panel's heading: "Socket", "How a cup hinge works".
    pub heading: String,
    /// The kind's doc comment: its first sentence for a joint, all of it
    /// for hardware.
    pub description: Option<String>,
    /// Labelled values: Goes in, Receives it, the cuts, Fit; or Opens to,
    /// Closest to, Swing check.
    pub rows: Vec<Row>,
    /// A joint kind's `insert:` as written.
    pub insert: Option<InsertText>,
    /// The part that receives a joint (`leg[front_left]`), for "see into
    /// it" beside X-ray.
    pub receives: Option<String>,
    /// A hardware kind's members.
    pub members: Vec<MemberRow>,
    /// Small views beside the main one.
    pub key_poses: Vec<KeyPose>,
    /// The smallest gap over a motion.
    pub closest: Option<(String, Length)>,
    /// The nodes drawn as they are.
    focus_nodes: Vec<NodeId>,
    /// The nodes drawn see-through when X-ray is on.
    xray_nodes: Vec<NodeId>,
    /// How it moves.
    play: Play,
    /// A flat view from above, for a hinge.
    plan: Option<Plan>,
    /// What to frame in the 3D view.
    zone: Option<Aabb>,
    /// Focus opens on the view from above (a joint that turns upright).
    prefers_plan: bool,
    /// The direction to look from in 3D (yaw and pitch), when focus has
    /// a better one than the viewer's: a hinge from inside the cabinet.
    view: Option<(f32, f32)>,
}

/// One frame of focus mode.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    /// Where everything is.
    pub posed: Posed,
    /// How each node looks.
    pub looks: Vec<Look>,
    /// Lines and labels over the view.
    pub overlay: Overlay,
    /// "Step 1 of 1 · push(rung[front], along: length)".
    pub step: Option<String>,
    /// "rung[front] is 6 mm from home", "door_l is open 45°".
    pub status: String,
}

/// The first sentence of a doc comment.
fn first_sentence(text: &str) -> String {
    let end = text.find(". ").map_or(text.len(), |index| index + 1);
    text.get(..end).unwrap_or(text).trim().to_owned()
}

/// A kind's doc comments as one paragraph.
fn doc_text(kind: &KindDef) -> Option<String> {
    let text = kind.decl.docs.join(" ");
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    (!text.is_empty()).then_some(text)
}

fn part_name(design: &SolvedDesign, id: PartId) -> String {
    design
        .part(id)
        .map_or_else(|| "?".to_owned(), |p| p.name.clone())
}

/// A gap for a beginner: millimetres rounded down to a tenth.
fn gap_text(gap: Length) -> String {
    let tenths = gap.nm().div_euclid(100_000);
    format!("{}.{} mm", tenths.div_euclid(10), tenths.rem_euclid(10))
}

/// A step as the source writes it, with its part named:
/// `push(rung[front], along: length)`.
fn step_text(design: &SolvedDesign, step: &stave_model::solved::AssemblyStep) -> String {
    let part = part_name(design, step.part);
    let by = match step.by {
        Some(StepAmount::Length(length)) => format!(", by: {length}"),
        Some(StepAmount::Angle(angle)) => format!(", by: {angle}"),
        None => String::new(),
    };
    let angle = step
        .angle
        .map(|angle| format!(", angle: {angle}"))
        .unwrap_or_default();
    let axis = step.axis.word();
    match step.kind {
        StepKind::Push => format!("push({part}, along: {axis}{by}{angle})"),
        StepKind::Drive => format!("drive({part}, along: {axis}{by}{angle})"),
        StepKind::Rotate => format!("rotate({part}{by}, about: {axis})"),
    }
}

/// A length for the panel: millimetres to the nearest tenth, without a
/// trailing `.0` (`6.3`, `22`).
fn tenths(length: Length) -> String {
    let tenths = length.nm().saturating_add(50_000).div_euclid(100_000);
    let (whole, tenth) = (tenths.div_euclid(10), tenths.rem_euclid(10));
    if tenth == 0 {
        whole.to_string()
    } else {
        format!("{whole}.{tenth}")
    }
}

/// A cut for the panel: its label and size.
pub(crate) fn cut_row(joint_kind: &str, kind: &CutKind) -> Row {
    let millimetres = tenths;
    let mm = |length: Length| format!("{} mm", millimetres(length));
    let deep = |depth: &Depth| match depth {
        Depth::Length(length) => format!("{} deep", mm(*length)),
        Depth::Through => "all the way through".to_owned(),
    };
    let (label, value) = match kind {
        CutKind::Drill {
            diameter,
            depth,
            count,
        } => {
            let holes = if *count > 1 {
                format!("{count} holes, ")
            } else {
                String::new()
            };
            (
                "Hole",
                format!("{holes}{} across, {}", mm(*diameter), deep(depth)),
            )
        }
        CutKind::Pocket {
            width,
            thickness,
            depth,
        } => (
            "Mortise",
            format!(
                "{} × {} mm, {}",
                millimetres(*thickness),
                millimetres(*width),
                deep(depth)
            ),
        ),
        CutKind::Tenon {
            length,
            width,
            thickness,
        } => (
            "Tenon",
            format!(
                "{} long, {} × {} mm",
                mm(*length),
                millimetres(*thickness),
                millimetres(*width)
            ),
        ),
        CutKind::Spigot { length, diameter } => (
            "Round tenon",
            format!("{} across, {} long", mm(*diameter), mm(*length)),
        ),
        CutKind::Notch { depth } => (
            match joint_kind.rsplit('.').next().unwrap_or(joint_kind) {
                "dado" => "Dado",
                "rabbet" => "Rabbet",
                "groove" => "Groove",
                _ => "Notch",
            },
            deep(depth),
        ),
        CutKind::Slot { width, depth, .. } => {
            ("Slot", format!("{} wide, {}", mm(*width), deep(depth)))
        }
        CutKind::Round { diameter, .. } => ("Round section", format!("{} across", mm(*diameter))),
        CutKind::DovetailSocket { .. } => ("Dovetail socket", "angled sides".to_owned()),
        CutKind::DovetailTail { .. } => ("Dovetail tail", "angled sides".to_owned()),
        CutKind::Outline { .. } => ("Outline", "cut to a drawn shape".to_owned()),
    };
    Row { label, value }
}

/// Every node of the parts given.
fn part_nodes(graph: &SceneGraph, parts: &[PartId]) -> Vec<NodeId> {
    parts
        .iter()
        .flat_map(|part| graph.nodes_of(Item::Part(*part)))
        .collect()
}

/// Opens focus on a target. `None` when the target isn't in the design.
pub fn open(
    opened: &Opened,
    design: &SolvedDesign,
    graph: &SceneGraph,
    target: Target,
) -> Option<Focus> {
    let focus = match target {
        Target::Joint { joint, pair } => joint_focus(opened, design, graph, joint, pair)?,
        Target::Hardware(id) => {
            hardware_focus(opened, design, graph, design.hardware.get(id.index())?)?
        }
    };
    Some(focus)
}

fn kind_def(opened: &Opened, kind: stave_model::solved::KindId) -> Option<&KindDef> {
    opened.model.as_ref()?.defs.kind(kind)
}

fn joint_focus(
    opened: &Opened,
    design: &SolvedDesign,
    graph: &SceneGraph,
    joint_id: JointId,
    pair_index: usize,
) -> Option<Focus> {
    let joint = design.joints.get(joint_id.index())?;
    let pair = joint.pairs.get(pair_index)?;
    let target = Target::Joint {
        joint: joint_id,
        pair: pair_index,
    };
    let kind = kind_def(opened, joint.kind_id);
    let members = pair_members(pair);
    let parts: Vec<&Part> = members.iter().filter_map(|id| design.part(*id)).collect();
    let solids: Vec<Solid> = parts.iter().map(|part| Solid::of(part, design)).collect();
    let play = match pair_steps(design, pair, &members, None) {
        Some(steps) if !steps.is_empty() && parts.len() == members.len() => {
            let sequence: Vec<Member<'_>> = parts
                .iter()
                .zip(&solids)
                .map(|(part, solid)| Member { part, solid })
                .collect();
            match trace(&sequence, &steps) {
                Ok(traced) => Play::Steps(StepPlay {
                    members: members.clone(),
                    traced,
                    texts: pair.insert.iter().map(|s| step_text(design, s)).collect(),
                }),
                Err(_) => Play::Still,
            }
        }
        Some(_) | None => Play::Still,
    };
    // The parts that never move receive the others: drawn see-through.
    let moving: Vec<PartId> = pair.insert.iter().map(|s| s.part).collect();
    let receiving: Vec<PartId> = members
        .iter()
        .copied()
        .filter(|p| !moving.contains(p))
        .collect();

    let mut rows = Vec::new();
    let roles: Vec<(&str, String)> = pair
        .roles
        .iter()
        .map(|r| (r.role.as_str(), part_name(design, r.part)))
        .collect();
    match roles.as_slice() {
        [(_, piece), (_, into)] => {
            rows.push(Row {
                label: "Goes in",
                value: piece.clone(),
            });
            rows.push(Row {
                label: "Receives it",
                value: into.clone(),
            });
        }
        _ => {
            for (role, part) in &roles {
                rows.push(Row {
                    label: "Part",
                    value: format!("{part} (the {role})"),
                });
            }
        }
    }
    if !pair.made_parts.is_empty() {
        rows.push(Row {
            label: "Made by the joint",
            value: pair
                .made_parts
                .iter()
                .map(|p| part_name(design, *p))
                .collect::<Vec<_>>()
                .join(", "),
        });
    }
    for part in &parts {
        for cut in &part.cuts {
            if cut.source
                == (CutSource::Joint {
                    joint: joint_id,
                    pair: pair_index,
                })
            {
                let mut row = cut_row(&joint.kind, &cut.kind);
                if parts.len() > 2 || matches!(cut.kind, CutKind::Notch { .. }) {
                    row.value = format!("{}, in {}", row.value, part.name);
                }
                rows.push(row);
            }
        }
    }
    rows.push(Row {
        label: "Fit",
        value: fit_text(&parts, &solids),
    });

    // A part that turns about the up axis (the chidori's lock) is best
    // seen from above, cut through the node: its flats lining up with the
    // slots it turns into.
    let zone = graph.joint_zone(&moving, &receiving);
    let turning = pair.insert.iter().find(|step| {
        step.kind == StepKind::Rotate
            && design.part(step.part).is_some_and(|part| {
                part.pose.orientation.direction(step.axis).axis() == stave_lang::placement::Axis::Up
            })
    });
    let plan = turning.and_then(|step| Plan::joint(graph, step.part, &receiving));
    let insert = kind.and_then(|kind| insert_text(opened, kind));
    let description = kind.and_then(doc_text).map(|text| first_sentence(&text));
    let title = title(design, target);
    let heading = {
        let bare = joint.kind.rsplit('.').next().unwrap_or(&joint.kind);
        let mut chars = bare.chars();
        chars.next().map_or_else(String::new, |first| {
            first
                .to_uppercase()
                .chain(chars)
                .collect::<String>()
                .replace('_', " ")
        })
    };
    Some(Focus {
        target,
        context: format!("{} · Joint focus · {title}", design.project.title),
        title,
        heading,
        description,
        rows,
        insert,
        receives: receiving.first().map(|id| part_name(design, *id)),
        members: Vec::new(),
        key_poses: vec![
            KeyPose {
                progress: 0.0,
                caption: "apart".to_owned(),
            },
            KeyPose {
                progress: 0.5,
                caption: "half way".to_owned(),
            },
            KeyPose {
                progress: 1.0,
                caption: "together".to_owned(),
            },
        ],
        closest: None,
        focus_nodes: part_nodes(graph, &members),
        xray_nodes: part_nodes(graph, &receiving),
        zone,
        view: None,
        play,
        plan,
        prefers_plan: plan.is_some(),
    })
}

/// "touches, nothing overlaps", or what is wrong.
fn fit_text(parts: &[&Part], solids: &[Solid]) -> String {
    let worlds: Vec<WorldSolid> = parts
        .iter()
        .zip(solids)
        .map(|(part, solid)| WorldSolid::of_part(part, solid))
        .collect();
    let mut deepest: Option<Length> = None;
    let mut all_touch = true;
    for (i, a) in worlds.iter().enumerate() {
        for b in worlds.iter().skip(i + 1) {
            for overlap in overlaps(a, b) {
                if deepest.is_none_or(|d| overlap.depth > d) {
                    deepest = Some(overlap.depth);
                }
            }
        }
        all_touch &= worlds
            .iter()
            .enumerate()
            .any(|(j, b)| j != i && touches(a, b));
    }
    match deepest {
        Some(depth) if depth > CONTACT_TOLERANCE => {
            format!("wood overlaps by {} mm", tenths(depth))
        }
        Some(_) | None if all_touch => "touches, nothing overlaps".to_owned(),
        Some(_) | None => "a part doesn't touch the others".to_owned(),
    }
}

/// A joint kind's `insert:` steps as written, and the file and line.
fn insert_text(opened: &Opened, kind: &KindDef) -> Option<InsertText> {
    let steps = &kind.joint.as_ref()?.insert;
    let first = steps.first()?;
    let file = opened.sources.get(kind.decl.file)?;
    let texts: Vec<&str> = steps
        .iter()
        .filter_map(|step| {
            let start = usize::try_from(step.span.start()).ok()?;
            let end = usize::try_from(step.span.end()).ok()?;
            file.text().get(start..end)
        })
        .collect();
    let line = file.line_column(first.span.start()).line;
    let path = file.path();
    let link = match path.strip_prefix("std/") {
        Some(inside) => format!("std:{}", inside.strip_suffix(".stave").unwrap_or(inside)),
        None => std::path::Path::new(path).file_name().map_or_else(
            || path.to_owned(),
            |name| format!("{}:{line}", name.to_string_lossy()),
        ),
    };
    let location = LineNumber::from_usize(line).map(|line| SourceLocation {
        path: path.to_owned(),
        line,
    });
    Some(InsertText {
        text: format!("insert: [{}]", texts.join(", ")),
        source: format!("{path}:{line}"),
        steps: texts.iter().map(|text| (*text).to_owned()).collect(),
        link,
        location,
    })
}

/// The innermost assembly around the hardware that moves.
fn moving_assembly(
    design: &SolvedDesign,
    graph: &SceneGraph,
    hardware: &Hardware,
) -> Option<(AssemblyId, MotionRange)> {
    let mut assembly = hardware.assembly;
    let mut seen = 0;
    while let Some(id) = assembly {
        if let Some(range) = graph.motion_range(id) {
            return Some((id, range));
        }
        seen += 1;
        if seen > design.assemblies.len() {
            break;
        }
        assembly = design.assemblies.get(id.index())?.parent;
    }
    None
}

/// The parts inside an assembly, its sub-assemblies' included.
fn assembly_parts(design: &SolvedDesign, assembly: AssemblyId) -> Vec<PartId> {
    design
        .parts
        .iter()
        .filter(|part| {
            let mut current = part.assembly;
            let mut steps = 0;
            while let Some(id) = current {
                if id == assembly {
                    return true;
                }
                steps += 1;
                if steps > design.assemblies.len() {
                    break;
                }
                current = design.assemblies.get(id.index()).and_then(|a| a.parent);
            }
            false
        })
        .map(|part| part.id)
        .collect()
}

fn hardware_focus(
    opened: &Opened,
    design: &SolvedDesign,
    graph: &SceneGraph,
    hardware: &Hardware,
) -> Option<Focus> {
    let target = Target::Hardware(hardware.id);
    let kind = kind_def(opened, hardware.kind_id);
    let hardware_nodes = graph.nodes_of(Item::Hardware(hardware.id));
    let on = match hardware.data {
        HardwareData::Hinge { on, .. } | HardwareData::ButtHinge { on, .. } => Some(on),
        HardwareData::Slide { .. } | HardwareData::Pull { .. } => None,
    };
    let mut context: Vec<PartId> = vec![hardware.piece];
    let outside_parts: Vec<PartId> = hardware
        .assembly
        .map(|assembly| assembly_parts(design, assembly))
        .unwrap_or_default();
    context.extend(side_of(design, hardware, &outside_parts, on));
    let mut focus_nodes = hardware_nodes.clone();
    focus_nodes.extend(part_nodes(graph, &context));
    let xray_nodes = part_nodes(graph, &[hardware.piece]);

    // The members, numbered as the kind lists them.
    let mut members = Vec::new();
    let mut anchors = Vec::new();
    if let Some(kind) = kind {
        for (index, member) in kind.members.iter().enumerate() {
            let number = u8::try_from(index + 1).unwrap_or(u8::MAX);
            members.push(MemberRow {
                number,
                name: member.name.word(),
                caption: member
                    .docs
                    .join(" ")
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" "),
            });
            if let Some(anchor) = member_anchor(graph, hardware, member.name, &hardware_nodes) {
                anchors.push((number, anchor));
            }
        }
    }

    let mut rows = Vec::new();
    let mut key_pose_rows = Vec::new();
    let mut closest = None;
    let mut plan = None;
    let mut view = None;
    let mut zone = None;
    // A pull is about how it is fixed, not about the door it is on.
    let moves =
        moving_assembly(design, graph, hardware).filter(|_| hardware.base != HardwareBase::Pull);
    let play = match moves {
        Some((assembly, range)) => {
            let motion = design
                .assemblies
                .get(assembly.index())
                .and_then(|a| a.motion);
            rows.push(Row {
                label: if range.turns { "Opens to" } else { "Pulls out" },
                value: amount_text(range, range.full),
            });
            key_pose_rows = key_poses(range)
                .into_iter()
                .map(|(progress, caption)| KeyPose { progress, caption })
                .collect();
            // The smallest gap to anything outside the assembly.
            let inside = assembly_parts(design, assembly);
            let solid = |part: &Part| WorldSolid::of_part(part, &Solid::of(part, design));
            let moving: Vec<WorldSolid> = inside
                .iter()
                .filter_map(|id| design.part(*id))
                .map(solid)
                .collect();
            let fixed_parts: Vec<&Part> = design
                .parts
                .iter()
                .filter(|p| !inside.contains(&p.id))
                .collect();
            let fixed: Vec<WorldSolid> = fixed_parts.iter().map(|p| solid(p)).collect();
            let mut gap_mark = None;
            // Where a gap runs along the hardware, it is shown level with it.
            let prefer = Some(box_centre(&hardware.bounds));
            if let Some(motion) = &motion {
                // To the side the hinge or slide is on, then to anything.
                let side = side_of(design, hardware, &inside, on)
                    .and_then(|on| fixed_parts.iter().position(|p| p.id == on));
                let on = side.and_then(|index| fixed_parts.get(index)).map(|p| p.id);
                if let Some(index) = side
                    && let Some(side) = fixed.get(index)
                    && let Some(found) =
                        closest_for(&moving, std::slice::from_ref(side), motion, prefer)
                {
                    rows.push(Row {
                        label: "Closest to the side",
                        value: gap_text(found.gap),
                    });
                    gap_mark = GapMark::new(graph, assembly, &found, &gap_text(found.gap));
                }
                if let Some(found) = closest_for(&moving, &fixed, motion, prefer)
                    && let Some(part) = fixed_parts.get(found.fixed)
                {
                    if Some(part.id) != on {
                        rows.push(Row {
                            label: "Closest to anything",
                            value: format!("{}: {}", part.name, gap_text(found.gap)),
                        });
                        if gap_mark.is_none() {
                            gap_mark = GapMark::new(graph, assembly, &found, &gap_text(found.gap));
                        }
                    }
                    closest = Some((part.name.clone(), found.gap));
                }
            }
            // The motion check's verdict.
            let hit = opened.contacts.iter().find(|(diagnostic, contact)| {
                diagnostic.code == Code::Motion
                    && contact.motion.is_some_and(|m| m.assembly == assembly)
            });
            rows.push(Row {
                label: if range.turns {
                    "Swing check"
                } else {
                    "Slide check"
                },
                value: match hit {
                    Some((diagnostic, _)) => diagnostic.message.clone(),
                    None => "clear all the way".to_owned(),
                },
            });
            let rest = graph.rest();
            let path = match (
                graph.bounds_of(&rest, &part_nodes(graph, &inside)),
                graph.bounds_of(&rest, &hardware_nodes),
            ) {
                (Some(door), Some(level)) => {
                    let toward = match motion {
                        Some(Motion::Travel { toward, .. }) => Some(toward),
                        Some(Motion::Turn { .. } | Motion::Linkage { .. }) | None => None,
                    };
                    let point =
                        edge_point(door, level, motion.as_ref().and_then(Motion::swing), toward);
                    graph.path_of(assembly, point, 96)
                }
                _ => Vec::new(),
            };
            if !path.is_empty() {
                rows.push(Row {
                    label: "Dotted line",
                    value: if range.turns {
                        "the path of the door's front edge: solid where it has been"
                    } else {
                        "the path of the drawer's front: solid where it has been"
                    }
                    .to_owned(),
                });
            }
            match motion {
                Some(Motion::Travel { toward, .. }) => {
                    plan = Plan::slide(graph, &hardware_nodes, toward, range.full);
                }
                Some(Motion::Turn { .. } | Motion::Linkage { .. }) => {
                    let arm = hardware.mechanism.as_ref().map(|m| m.arm.bounds);
                    plan = Plan::of(graph, &hardware_nodes, &path, arm);
                    view = motion
                        .as_ref()
                        .and_then(Motion::swing)
                        .map(|(free, _)| hinge_view(free));
                }
                None => {}
            }
            zone = motion_zone(graph, &hardware_nodes, assembly, &path, gap_mark.as_ref());
            Play::Motion(MotionPlay {
                assembly,
                range,
                path,
                anchors,
                gap: gap_mark,
            })
        }
        None => Play::Fixed(anchors.clone()),
    };
    let heading = match hardware.base {
        HardwareBase::Hinge if hardware.is_butt_hinge() => "How a butt hinge works",
        HardwareBase::Hinge => "How a cup hinge works",
        HardwareBase::Slide => "How a drawer slide works",
        HardwareBase::Pull => "How a pull is fixed",
    };
    let title = title(design, target);
    Some(Focus {
        target,
        context: format!("{} · Hardware · {title}", design.project.title),
        title,
        heading: heading.to_owned(),
        description: kind.and_then(doc_text),
        rows,
        insert: None,
        receives: None,
        members,
        key_poses: key_pose_rows,
        closest,
        focus_nodes,
        xray_nodes,
        zone: zone.or_else(|| around(graph, &hardware_nodes)),
        view,
        play,
        plan,
        prefers_plan: false,
    })
}

/// The part a hinge or slide is fixed to outside its assembly: a hinge's
/// `on`, or the part a slide's cabinet member lies against.
fn side_of(
    design: &SolvedDesign,
    hardware: &Hardware,
    inside: &[PartId],
    on: Option<PartId>,
) -> Option<PartId> {
    if on.is_some() {
        return on;
    }
    let member = hardware.fixed.as_ref()?.bounds;
    let touching = |a: &stave_model::solved::WorldBox, b: &stave_model::solved::WorldBox| {
        stave_lang::placement::Axis::ALL.into_iter().all(|axis| {
            a.min().get(axis) <= b.max().get(axis) && b.min().get(axis) <= a.max().get(axis)
        })
    };
    design
        .parts
        .iter()
        .filter(|p| !inside.contains(&p.id))
        .find(|p| touching(&p.cut_box(), &member))
        .map(|p| p.id)
}

/// Where a member's number goes.
fn member_anchor(
    graph: &SceneGraph,
    hardware: &Hardware,
    member: MemberName,
    nodes: &[NodeId],
) -> Option<Anchor> {
    let suffix_of = |id: &NodeId| {
        graph
            .node(*id)
            .and_then(|node| node.name.strip_prefix(hardware.name.as_str()))
            .map(str::trim)
    };
    let pick = |test: &dyn Fn(&str) -> bool| -> Vec<NodeId> {
        nodes
            .iter()
            .copied()
            .filter(|id| suffix_of(id).is_some_and(test))
            .collect()
    };
    Some(match member {
        MemberName::Cup | MemberName::Leaves => Anchor::Nodes(pick(&|rest| rest.is_empty()), None),
        // A slide's drawer member is long: its number goes near its front.
        MemberName::DrawerMember => {
            Anchor::Nodes(pick(&|rest| rest.is_empty()), Some(Direction::Front))
        }
        // Seen from above the arm lies over the plate: the plate's number
        // goes at its back end, the arm's at its front.
        MemberName::Plate => Anchor::Box(hardware.fixed.as_ref()?.bounds, Some(Direction::Back)),
        MemberName::Arm => Anchor::Box(
            hardware.mechanism.as_ref()?.arm.bounds,
            Some(Direction::Front),
        ),
        MemberName::Links => Anchor::Nodes(pick(&|rest| rest.ends_with("link")), None),
        // A slide's cabinet member is long: its number goes near its
        // front, where the view from above looks.
        MemberName::CabinetMember => {
            Anchor::Box(hardware.fixed.as_ref()?.bounds, Some(Direction::Front))
        }
        MemberName::Pin => Anchor::Nodes(
            pick(&|rest| !rest.is_empty() && !rest.ends_with("link")),
            None,
        ),
        // A pull stands off its face: what you hold is at the outer end,
        // the screws at the end against the door.
        MemberName::Knob | MemberName::Bar => {
            Anchor::Box(hardware.bounds, Some(hardware.mount_face.direction()))
        }
        MemberName::Screw | MemberName::Screws => Anchor::Box(
            hardware.bounds,
            Some(hardware.mount_face.direction().opposite()),
        ),
    })
}

impl Focus {
    /// The scrub amount's meaning: `true` when 0 is apart and 1 together
    /// (a joint), `false` when 0 is closed and 1 fully open (hardware).
    pub fn assembles(&self) -> bool {
        matches!(self.target, Target::Joint { .. })
    }

    /// The nodes to frame: the item and the parts it joins or is mounted
    /// on.
    pub fn nodes(&self) -> &[NodeId] {
        &self.focus_nodes
    }

    /// The box the 3D view frames: where a joint's parts meet, or the
    /// hardware and the path its door or drawer follows.
    pub fn zone(&self) -> Option<Aabb> {
        self.zone
    }

    /// The direction to look from in 3D (an orbit's yaw and pitch), when
    /// focus has one: a hinge is seen from inside the cabinet.
    pub fn view(&self) -> Option<(f32, f32)> {
        self.view
    }

    /// Whether focus should open looking from above: a joint whose part
    /// turns about the up axis reads best that way.
    pub fn prefers_plan(&self) -> bool {
        self.prefers_plan
    }

    /// What the view from above shows, in words.
    pub fn plan_caption(&self) -> Option<&'static str> {
        self.plan.as_ref()?;
        Some(match self.target {
            Target::Hardware(_) if self.heading.contains("slide") => {
                "Looking down on the slide, cut through its middle"
            }
            Target::Hardware(_) => "Looking down on the hinge, through the cabinet top",
            Target::Joint { .. } => "Looking down on the joint, cut through its middle",
        })
    }

    /// A flat view from above, for a hinge or a slide.
    pub fn plan(&self) -> Option<&Plan> {
        self.plan.as_ref()
    }

    /// The number of steps a joint plays (0 for hardware).
    pub fn step_count(&self) -> usize {
        match &self.play {
            Play::Steps(play) => play.traced.len(),
            Play::Motion(_) | Play::Still | Play::Fixed(_) => 0,
        }
    }

    /// Where along the motion (0 to 1) it comes closest to the parts
    /// around it, when that is marked: the one pose its gap is drawn in.
    pub fn closest_progress(&self) -> Option<f32> {
        match &self.play {
            Play::Motion(play) => play.closest_progress(),
            Play::Steps(_) | Play::Still | Play::Fixed(_) => None,
        }
    }

    /// The assembly a piece of hardware moves, and how far.
    pub fn motion(&self) -> Option<(AssemblyId, MotionRange)> {
        match &self.play {
            Play::Motion(play) => Some((play.assembly, play.range)),
            Play::Steps(_) | Play::Still | Play::Fixed(_) => None,
        }
    }

    /// The design posed at `progress` (0 to 1: apart to together for a
    /// joint, closed to open for hardware) in a scene, with every node's
    /// look and the overlay.
    pub fn frame(
        &self,
        graph: &SceneGraph,
        scene: &SceneEntry,
        progress: f32,
        options: &Options,
    ) -> Frame {
        let xray: &[NodeId] = if options.xray { &self.xray_nodes } else { &[] };
        let looks = graph.looks(&self.focus_nodes, xray, options.fade_others);
        let played = self.play.frame(graph, scene, progress);
        Frame {
            posed: played.posed,
            looks,
            overlay: played.overlay,
            step: played.step,
            status: played.status,
        }
    }
}
