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

use crate::i18n::{fill_in, tr, tr_noop};
use stave_geom::clearance::{box_centre, closest_for};
use stave_geom::mechanism::MechanismPath;

use stave_geom::collide::{CONTACT_TOLERANCE, WorldSolid, overlaps, touches};
use stave_geom::sequence::{Member, pair_members, pair_steps, trace, trace_carrying};
use stave_geom::solid::Solid;
use stave_lang::Code;
use stave_lang::units::Length;
use stave_model::defs::KindDef;
use stave_model::mechanism::{Holding, Mechanism, MechanismEntry, MemberIndex, MotionShape};
use stave_model::solved::{
    AssemblyId, Body, BodyCarrier, CutKind, CutPlace, CutSource, Depth, Hardware, HardwareBase,
    HardwareId, JointId, PairIndex, Part, PartId, RoleValue, SolvedDesign, StepMotion, WorldBox,
};
use stave_scene::focus::{
    Anchor, GapMark, MotionPlay, Plan, Play, StepPlay, amount_text, around, edge_point, hinge_view,
    key_poses, motion_zone,
};
use stave_scene::{Aabb, Item, Look, MotionRange, NodeId, Overlay, Posed, SceneEntry, SceneGraph};

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
        /// The pair.
        pair: PairIndex,
    },
    /// A piece of hardware.
    Hardware(HardwareId),
}

/// Why a name doesn't pick out something to focus on.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FindError {
    /// No joint or hardware has that name.
    #[error("{}", self.text())]
    NotFound {
        /// The name asked for.
        name: String,
        /// Some names that are there.
        names: String,
    },
    /// The joint has fewer pairs.
    #[error("{}", self.text())]
    NoPair {
        /// The joint.
        name: String,
        /// Its number of pairs.
        count: usize,
        /// The pair asked for.
        pair: usize,
    },
}

impl FindError {
    /// The error in the window's words (translated).
    fn text(&self) -> String {
        match self {
            FindError::NotFound { name, names } => fill_in(
                &tr("no joint or hardware is called '{name}'; try one of: {names}"),
                &[("name", name), ("names", names)],
            ),
            FindError::NoPair { name, count, pair } => fill_in(
                &tr("joint '{name}' has {count} pairs, numbered from 0; there is no pair {pair}"),
                &[("name", name), ("count", count), ("pair", pair)],
            ),
        }
    }
}

/// The joint (and pair, default the first) or piece of hardware with this
/// name.
pub fn find(design: &SolvedDesign, name: &str, pair: Option<usize>) -> Result<Target, FindError> {
    if let Some(joint) = design.joints().iter().find(|j| j.name == name) {
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
            pair: PairIndex::from_usize(index),
        });
    }
    if let Some(hardware) = design.hardware().iter().find(|h| h.name == name) {
        return Ok(Target::Hardware(hardware.id));
    }
    let names: Vec<&str> = design
        .joints()
        .iter()
        .map(|j| j.name.as_str())
        .chain(design.hardware().iter().map(|h| h.name.as_str()))
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
            .joints()
            .iter()
            .flat_map(|joint| {
                joint
                    .indexed_pairs()
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
        // A joint's member goes together with its pair.
        Item::Member(member) => {
            let target = Target::Joint {
                joint: member.joint,
                pair: member.pair,
            };
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
            let Some(joint) = design.joint(joint) else {
                return String::new();
            };
            let Some(pair) = joint.pair(pair) else {
                return joint.name.clone();
            };
            let parts: Vec<&str> = pair.roles.iter().map(|r| name(r.part)).collect();
            match parts.as_slice() {
                [piece, into] => fill_in(
                    &tr("{piece} into {into} ({kind})"),
                    &[("piece", piece), ("into", into), ("kind", &joint.kind)],
                ),
                _ => format!("{} ({})", parts.join(", "), joint.kind),
            }
        }
        Target::Hardware(id) => design
            .hardware()
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
    pub name: String,
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

/// Why focus on a joint has no steps to play, so its step player has
/// nothing to scrub.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StillJoint {
    /// The kind has no `insert:` steps: its parts are brought together
    /// in place (a mitre, a glued cleat), nothing slides in.
    NoSteps,
    /// A step can't be played: the part it moves hits another on the way.
    Blocked {
        /// The step, counted from 1.
        step: usize,
        /// The part the step moves: `rung[front]`.
        moving: String,
        /// The part in its way: `leg[front_left]`.
        by: String,
    },
    /// The steps name a part the pair doesn't join, so they can't be
    /// traced.
    CantTrace,
}

impl StillJoint {
    /// In words, for the step player in place of its slider.
    pub fn text(&self) -> String {
        match self {
            StillJoint::NoSteps => {
                tr("These parts are brought together in place: nothing slides in to play.")
            }
            StillJoint::Blocked { step, moving, by } => fill_in(
                &tr("Step {step} can't be played: {by} is in the way of {moving}."),
                &[("step", step), ("by", by), ("moving", moving)],
            ),
            StillJoint::CantTrace => tr("Its steps can't be played."),
        }
    }
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
    /// What the panel shows before "More": the doc comment's first
    /// paragraph, at most two sentences. The same as `description` when
    /// that is short.
    pub summary: Option<String>,
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
    /// Why a joint has no steps to play; `None` when it plays them, and
    /// for hardware.
    still: Option<StillJoint>,
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

/// A kind's doc comment's first paragraph, at most two sentences.
fn summary_text(kind: &KindDef) -> Option<String> {
    let paragraph: Vec<&str> = kind
        .decl
        .docs
        .iter()
        .map(|line| line.trim())
        .skip_while(|line| line.is_empty())
        .take_while(|line| !line.is_empty())
        .collect();
    let text = paragraph.join(" ");
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() {
        return None;
    }
    let mut end = text.len();
    let mut found = 0;
    for (index, _) in text.match_indices(". ") {
        found += 1;
        if found == 2 {
            end = index + 1;
            break;
        }
    }
    Some(text.get(..end).unwrap_or(&text).trim().to_owned())
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
    step.source_text(&part, step.axis)
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

/// A cut for the panel: its label, from the cut's operation, and size.
/// The joint's own name heads the panel, so a dado's notch is not
/// relabelled by matching the kind's name.
pub(crate) fn cut_row(kind: &CutKind) -> Row {
    let millimetres = tenths;
    let mm = |length: Length| format!("{} mm", millimetres(length));
    let deep = |depth: &Depth| match depth {
        Depth::Length(length) => fill_in(&tr("{length} deep"), &[("length", &mm(*length))]),
        Depth::Through => tr("all the way through"),
    };
    let (label, value) = match kind {
        CutKind::Drill {
            diameter,
            depth,
            from_depth,
            tilt,
        } => {
            let mut value = fill_in(
                &tr("{diameter} across, {depth}"),
                &[("diameter", &mm(*diameter)), ("depth", &deep(depth))],
            );
            if *from_depth > Length::ZERO {
                value = fill_in(
                    &tr("{hole}, starting {start} below the face"),
                    &[("hole", &value), ("start", &mm(*from_depth))],
                );
            }
            if let Some(tilt) = tilt {
                value = fill_in(
                    &tr("{hole}, slanted {angle}"),
                    &[("hole", &value), ("angle", &tilt.angle.to_string())],
                );
            }
            (tr_noop("Hole"), value)
        }
        CutKind::Pocket {
            width,
            thickness,
            depth,
        } => (
            tr_noop("Mortise"),
            fill_in(
                &tr("{thickness} × {width} mm, {depth}"),
                &[
                    ("thickness", &millimetres(*thickness)),
                    ("width", &millimetres(*width)),
                    ("depth", &deep(depth)),
                ],
            ),
        ),
        CutKind::Tenon {
            length,
            width,
            thickness,
        } => (
            tr_noop("Tenon"),
            fill_in(
                &tr("{length} long, {thickness} × {width} mm"),
                &[
                    ("length", &mm(*length)),
                    ("thickness", &millimetres(*thickness)),
                    ("width", &millimetres(*width)),
                ],
            ),
        ),
        CutKind::Spigot { length, diameter } => (
            tr_noop("Round tenon"),
            fill_in(
                &tr("{diameter} across, {length} long"),
                &[("diameter", &mm(*diameter)), ("length", &mm(*length))],
            ),
        ),
        CutKind::Notch { depth } => (tr_noop("Notch"), deep(depth)),
        CutKind::Slot { width, depth, .. } => (
            tr_noop("Slot"),
            fill_in(
                &tr("{width} wide, {depth}"),
                &[("width", &mm(*width)), ("depth", &deep(depth))],
            ),
        ),
        CutKind::Round { diameter, .. } => (
            tr_noop("Round section"),
            fill_in(&tr("{length} across"), &[("length", &mm(*diameter))]),
        ),
        CutKind::DovetailSocket { .. } => (tr_noop("Dovetail socket"), tr("angled sides")),
        CutKind::DovetailTail { .. } => (tr_noop("Dovetail tail"), tr("angled sides")),
        CutKind::Outline { .. } => (tr_noop("Outline"), tr("cut to a drawn shape")),
        CutKind::Diagonal => (
            tr_noop("Mitre"),
            tr("cut on the diagonal, outside corner to inside corner"),
        ),
        CutKind::Chamfer { size } => (
            tr_noop("Chamfer"),
            fill_in(&tr("{size} back along each face"), &[("size", &mm(*size))]),
        ),
        CutKind::Roundover { radius } => (
            tr_noop("Roundover"),
            fill_in(&tr("{radius} radius"), &[("radius", &mm(*radius))]),
        ),
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

/// Every node of a joint pair's members (a dowel's pins).
fn pair_member_nodes(graph: &SceneGraph, joint: JointId, pair: PairIndex) -> Vec<NodeId> {
    graph
        .node_ids()
        .filter(|id| {
            graph.node(*id).is_some_and(|node| match node.item {
                Item::Member(member) => member.joint == joint && member.pair == pair,
                Item::Part(_) | Item::Hardware(_) => false,
            })
        })
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
        Target::Hardware(id) => hardware_focus(opened, design, graph, design.hardware_piece(id)?)?,
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
    pair_index: PairIndex,
) -> Option<Focus> {
    let joint = design.joint(joint_id)?;
    let pair = joint.pair(pair_index)?;
    let target = Target::Joint {
        joint: joint_id,
        pair: pair_index,
    };
    let kind = kind_def(opened, joint.kind_id);
    let members = pair_members(pair);
    let parts: Vec<&Part> = members.iter().filter_map(|id| design.part(*id)).collect();
    let solids: Vec<Solid> = parts.iter().map(|part| Solid::of(part, design)).collect();
    let (play, still) = match pair_steps(design, pair, &members, None) {
        _ if pair.insert.is_empty() => (Play::Still, Some(StillJoint::NoSteps)),
        Some(steps) if !steps.is_empty() && parts.len() == members.len() => {
            let sequence: Vec<Member<'_>> = parts
                .iter()
                .zip(&solids)
                .map(|(part, solid)| Member { part, solid })
                .collect();
            // The pair's members (its pins) move with the part that
            // carries them, so the part comes off until they are out of
            // their holes. A pair whose members would block a step (none
            // of the standard kinds) plays its parts' own steps.
            let carried: Vec<Vec<&Body>> = members
                .iter()
                .map(|part| {
                    pair.members
                        .iter()
                        .filter(|body| body.carried_by() == Some(*part))
                        .collect()
                })
                .collect();
            match trace_carrying(&sequence, &carried, &steps).or_else(|_| trace(&sequence, &steps))
            {
                Ok(traced) => (
                    Play::Steps(StepPlay {
                        members: members.clone(),
                        traced,
                        texts: pair.insert.iter().map(|s| step_text(design, s)).collect(),
                    }),
                    None,
                ),
                Err(blocked) => {
                    let name = |part: Option<PartId>| part.map(|part| part_name(design, part));
                    let still = match (
                        name(pair.insert.get(blocked.step).map(|s| s.part)),
                        name(members.get(blocked.by).copied()),
                    ) {
                        (Some(moving), Some(by)) => StillJoint::Blocked {
                            step: blocked.step.saturating_add(1),
                            moving,
                            by,
                        },
                        (None, _) | (_, None) => StillJoint::CantTrace,
                    };
                    (Play::Still, Some(still))
                }
            }
        }
        Some(_) | None => (Play::Still, Some(StillJoint::CantTrace)),
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
                label: tr_noop("Goes in"),
                value: piece.clone(),
            });
            rows.push(Row {
                label: tr_noop("Receives it"),
                value: into.clone(),
            });
        }
        _ => {
            for (role, part) in &roles {
                rows.push(Row {
                    label: tr_noop("Part"),
                    value: fill_in(
                        &tr("{part} (the {role})"),
                        &[("part", part), ("role", role)],
                    ),
                });
            }
        }
    }
    if !pair.made_parts.is_empty() {
        rows.push(Row {
            label: tr_noop("Made by the joint"),
            value: pair
                .made_parts
                .iter()
                .map(|p| part_name(design, *p))
                .collect::<Vec<_>>()
                .join(", "),
        });
    }
    // The members it places (a dowel's pins): how many, what, how big.
    for body in &pair.members {
        let what = body
            .item
            .as_ref()
            .map(|item| format!(" ({})", crate::view::item_text(item)))
            .unwrap_or_default();
        rows.push(Row {
            label: tr_noop("Places"),
            value: fill_in(
                &tr("{count} × {name}{what}, {each}"),
                &[
                    ("count", &body.copies()),
                    ("name", &body.name.replace('_', " ")),
                    ("what", &what),
                    ("each", &crate::view::copy_text(body)),
                ],
            ),
        });
    }
    // The cuts, the same cut in one part counted once (a hole for each
    // pin is "4 × Hole").
    let mut cuts: Vec<(Row, usize)> = Vec::new();
    for part in &parts {
        for cut in &part.cuts {
            if cut.source
                == (CutSource::Joint {
                    joint: joint_id,
                    pair: pair_index,
                })
            {
                let mut row = cut_row(&cut.kind);
                let per_member = matches!(cut.place, CutPlace::Member { .. });
                if parts.len() > 2 || per_member || matches!(cut.kind, CutKind::Notch { .. }) {
                    row.value = fill_in(
                        &tr("{cut}, in {part}"),
                        &[("cut", &row.value), ("part", &part.name)],
                    );
                }
                match cuts.iter_mut().find(|(seen, _)| *seen == row) {
                    Some((_, count)) => *count = count.saturating_add(1),
                    None => cuts.push((row, 1)),
                }
            }
        }
    }
    rows.extend(cuts.into_iter().map(|(row, count)| {
        if count > 1 {
            Row {
                value: format!("{count} × {}", row.value),
                ..row
            }
        } else {
            row
        }
    }));
    rows.push(Row {
        label: tr_noop("Fit"),
        value: fit_text(&parts, &solids),
    });

    // A part that turns about the up axis (the chidori's lock) is best
    // seen from above, cut through the node: its flats lining up with the
    // slots it turns into.
    // A joint with members (a dowel's pins) is framed on them: they are
    // what shows how it goes together.
    let pins = pair_member_nodes(graph, joint_id, pair_index);
    let zone = graph
        .members_zone(&pins)
        .or_else(|| graph.joint_zone(&moving, &receiving));
    let turning = pair.insert.iter().find(|step| {
        matches!(step.motion, StepMotion::Rotate { .. })
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
        context: fill_in(
            &tr("{project} · Joint focus · {title}"),
            &[("project", &design.project.title), ("title", &title)],
        ),
        title,
        heading,
        summary: description.clone(),
        description,
        rows,
        insert,
        receives: receiving.first().map(|id| part_name(design, *id)),
        members: Vec::new(),
        // A joint with nothing to play looks the same at every pose.
        key_poses: if still.is_some() {
            Vec::new()
        } else {
            vec![
                KeyPose {
                    progress: 0.0,
                    caption: tr("apart"),
                },
                KeyPose {
                    progress: 0.5,
                    caption: tr("half way"),
                },
                KeyPose {
                    progress: 1.0,
                    caption: tr("together"),
                },
            ]
        },
        closest: None,
        // The pair's members (its pins) are drawn as they are and move
        // with the part that carries them.
        focus_nodes: part_nodes(graph, &members)
            .into_iter()
            .chain(pins)
            .collect(),
        xray_nodes: part_nodes(graph, &receiving),
        zone,
        view: None,
        play,
        still,
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
        Some(depth) if depth > CONTACT_TOLERANCE => fill_in(
            &tr("wood overlaps by {depth} mm"),
            &[("depth", &tenths(depth))],
        ),
        Some(_) | None if all_touch => fits_words(),
        Some(_) | None => tr("a part doesn't touch the others"),
    }
}

/// What the Fit row says when every part touches and none overlaps: the
/// window shows it as good.
pub fn fits_words() -> String {
    tr("touches, nothing overlaps")
}

/// What the swing or slide check says when nothing is hit: the window
/// shows it as good.
pub fn clear_words() -> String {
    tr("clear all the way")
}

/// A joint kind's `insert:` steps as written, and the file and line.
fn insert_text(opened: &Opened, kind: &KindDef) -> Option<InsertText> {
    let steps = &kind.joint()?.insert;
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
        if seen > design.assemblies().len() {
            break;
        }
        assembly = design.assembly(id)?.parent;
    }
    None
}

/// The parts inside an assembly, its sub-assemblies' included.
fn assembly_parts(design: &SolvedDesign, assembly: AssemblyId) -> Vec<PartId> {
    design
        .parts()
        .iter()
        .filter(|part| {
            let mut current = part.assembly;
            let mut steps = 0;
            while let Some(id) = current {
                if id == assembly {
                    return true;
                }
                steps += 1;
                if steps > design.assemblies().len() {
                    break;
                }
                current = design.assembly(id).and_then(|a| a.parent);
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
    let mut context: Vec<PartId> = vec![hardware.piece];
    let outside_parts: Vec<PartId> = hardware
        .assembly
        .map(|assembly| assembly_parts(design, assembly))
        .unwrap_or_default();
    context.extend(side_of(design, hardware, &outside_parts));
    let mut focus_nodes = hardware_nodes.clone();
    focus_nodes.extend(part_nodes(graph, &context));
    let xray_nodes = part_nodes(graph, &[hardware.piece]);

    // The members, numbered as the kind lists them, each by its own name
    // and doc comment.
    let mut members = Vec::new();
    let mut anchors = Vec::new();
    if let Some(kind) = kind {
        for (index, member) in kind.bodies.iter().enumerate() {
            let number = u8::try_from(index + 1).unwrap_or(u8::MAX);
            members.push(MemberRow {
                number,
                name: member.name.as_str().to_owned(),
                caption: member
                    .docs
                    .join(" ")
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" "),
            });
            let nodes = member_nodes(graph, &hardware_nodes, index);
            if !nodes.is_empty() {
                anchors.push((number, Anchor::Nodes(nodes, None)));
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
            let mechanism = design
                .assemblies()
                .get(assembly.index())
                .and_then(|a| a.mechanism.as_ref());
            let motion = mechanism.and_then(|m| MechanismPath::new(m).ok());
            let swing = mechanism.and_then(Mechanism::swing);
            let toward = match mechanism.map(Mechanism::shape) {
                Some(MotionShape::Travel { toward, .. }) => Some(toward),
                Some(MotionShape::Turn { .. } | MotionShape::General) | None => None,
            };
            rows.push(Row {
                label: if range.turns() {
                    tr_noop("Opens to")
                } else {
                    tr_noop("Pulls out")
                },
                value: amount_text(range, range.full()),
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
                .parts()
                .iter()
                .filter(|p| !inside.contains(&p.id))
                .collect();
            let fixed: Vec<WorldSolid> = fixed_parts.iter().map(|p| solid(p)).collect();
            let mut gap_mark = None;
            // Where a gap runs along the hardware, it is shown level with it.
            let prefer = Some(box_centre(&hardware.bounds));
            if let Some(motion) = &motion {
                // To the side the hinge or slide is on, then to anything.
                let side = side_of(design, hardware, &inside)
                    .and_then(|on| fixed_parts.iter().position(|p| p.id == on));
                let on = side.and_then(|index| fixed_parts.get(index)).map(|p| p.id);
                if let Some(index) = side
                    && let Some(side) = fixed.get(index)
                    && let Some(found) =
                        closest_for(&moving, std::slice::from_ref(side), motion, prefer)
                {
                    rows.push(Row {
                        label: tr_noop("Closest to the side"),
                        value: gap_text(found.gap),
                    });
                    gap_mark = GapMark::new(graph, assembly, &found, &gap_text(found.gap));
                }
                if let Some(found) = closest_for(&moving, &fixed, motion, prefer)
                    && let Some(part) = fixed_parts.get(found.fixed)
                {
                    if Some(part.id) != on {
                        rows.push(Row {
                            label: tr_noop("Closest to anything"),
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
                label: if range.turns() {
                    tr_noop("Swing check")
                } else {
                    tr_noop("Slide check")
                },
                value: match hit {
                    Some((diagnostic, _)) => diagnostic.message.clone(),
                    None => clear_words(),
                },
            });
            let rest = graph.rest();
            let path = match (
                graph.bounds_of(&rest, &part_nodes(graph, &inside)),
                graph.bounds_of(&rest, &hardware_nodes),
            ) {
                (Some(door), Some(level)) => {
                    let point = edge_point(door, level, swing, toward);
                    graph.path_of(assembly, point, 96)
                }
                _ => Vec::new(),
            };
            if !path.is_empty() {
                rows.push(Row {
                    label: tr_noop("Dotted line"),
                    value: if range.turns() {
                        tr("the path of the door's front edge: solid where it has been")
                    } else {
                        tr("the path of the drawer's front: solid where it has been")
                    },
                });
            }
            match (toward, swing) {
                (Some(toward), _) => {
                    plan = Plan::slide(graph, &hardware_nodes, toward, range.full());
                }
                (None, Some((free, _))) => {
                    plan = Plan::of(graph, &hardware_nodes, &path, pinned_held(hardware));
                    view = Some(hinge_view(free));
                }
                (None, None) => {}
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
        None => Play::Fixed(anchors),
    };
    // The base's words; the kind's doc comment, shown under it, says which
    // hinge, slide, pull, support or fitting it is.
    let heading = match hardware.base {
        HardwareBase::Hinge => tr("How this hinge works"),
        HardwareBase::Slide => tr("How this drawer slide works"),
        HardwareBase::Pull => tr("How this pull is fixed"),
        HardwareBase::Support => tr("What this shelf rests on"),
        HardwareBase::Fitting => tr("How this fitting is fixed"),
    };
    let title = title(design, target);
    Some(Focus {
        target,
        context: fill_in(
            &tr("{project} · Hardware · {title}"),
            &[("project", &design.project.title), ("title", &title)],
        ),
        title,
        heading,
        description: kind.and_then(doc_text),
        summary: kind.and_then(summary_text),
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
        still: None,
        plan,
        prefers_plan: false,
    })
}

/// The part a piece of hardware is fixed to outside its assembly: the
/// first of its roles bound to a part outside it (a hinge's `on`), or else
/// the part outside it that a member held by a region lies against (a
/// slide's cabinet member against the side).
fn side_of(design: &SolvedDesign, hardware: &Hardware, inside: &[PartId]) -> Option<PartId> {
    let role = hardware.roles.iter().find_map(|(_, value)| match value {
        RoleValue::Part(part) if *part != hardware.piece && !inside.contains(part) => Some(*part),
        RoleValue::Part(_) | RoleValue::Region(_) => None,
    });
    if role.is_some() {
        return role;
    }
    let held: Vec<WorldBox> = hardware
        .members
        .iter()
        .filter(|member| matches!(member.carrier, BodyCarrier::Region(_)))
        .map(Body::bounds)
        .collect();
    let touching = |a: &WorldBox, b: &WorldBox| {
        stave_lang::placement::Axis::ALL.into_iter().all(|axis| {
            a.min().get(axis) <= b.max().get(axis) && b.min().get(axis) <= a.max().get(axis)
        })
    };
    design
        .parts()
        .iter()
        .filter(|p| !inside.contains(&p.id))
        .find(|p| held.iter().any(|member| touching(&p.cut_box(), member)))
        .map(|p| p.id)
}

/// The nodes of one member of a piece of hardware (by its index among its
/// kind's members with a body): where its number goes.
fn member_nodes(graph: &SceneGraph, nodes: &[NodeId], body: usize) -> Vec<NodeId> {
    nodes
        .iter()
        .copied()
        .filter(|id| {
            graph
                .node(*id)
                .and_then(|node| node.member)
                .is_some_and(|(member, _)| member.body.index() == body)
        })
        .collect()
}

/// The world box around the held members a piece of hardware's mechanism
/// pins its moving members to (an arm the links turn on, a fixed leaf):
/// the view from above is cut just above them, so it shows what turns on
/// them.
fn pinned_held(hardware: &Hardware) -> Option<WorldBox> {
    let mechanism = hardware.own_mechanism.as_ref()?;
    let held = |index: MemberIndex| {
        mechanism
            .members()
            .get(mechanism.piece_of(index).index())
            .is_some_and(|member| member.holding == Holding::Held)
    };
    mechanism
        .entries()
        .iter()
        .flat_map(MechanismEntry::members)
        .filter(|index| held(*index))
        .filter_map(|index| mechanism.members().get(index.index()))
        .filter_map(|member| hardware.members.iter().find(|b| b.id.body == member.body))
        .map(Body::bounds)
        .reduce(|all, one| all.union(&one))
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
    pub fn plan_caption(&self) -> Option<String> {
        self.plan.as_ref()?;
        Some(match self.target {
            Target::Hardware(_) if self.motion().is_some_and(|(_, range)| !range.turns()) => {
                tr("Looking down on the slide, cut through its middle")
            }
            Target::Hardware(_) => tr("Looking down on the hinge, through the cabinet top"),
            Target::Joint { .. } => tr("Looking down on the joint, cut through its middle"),
        })
    }

    /// A flat view from above, for a hinge or a slide.
    pub fn plan(&self) -> Option<&Plan> {
        self.plan.as_ref()
    }

    /// Why a joint has no steps to play: `None` when it plays them, and
    /// for hardware.
    pub fn still_joint(&self) -> Option<&StillJoint> {
        self.still.as_ref()
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
            status: self.still.as_ref().map_or(played.status, StillJoint::text),
        }
    }
}
