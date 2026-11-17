//! "How it comes apart" (Exploded.dc.html): the pulled-apart view in
//! words, read off the exploded poses themselves, so the words say what
//! the picture does.
//!
//! The explode runs in two stages ([`SceneGraph::pose_with`]): up to half
//! way each assembly moves out whole (drawers along their slides, doors off
//! their hinges), then the parts inside each assembly, and the carcass's
//! own parts, move out along the reverse of their joints' last insert
//! step. Each step here is one group of what moves in a stage: "Drawers
//! slide forward on their slides until they clear the carcass", "Carcass
//! comes apart: top lifts out of its rabbet, shelf slides out of its dado".

use std::collections::BTreeMap;
use std::path::Path;

use glam::Vec3;
use stave_geom::render;
use stave_lang::placement::Direction;
use stave_model::solved::{AssemblyId, HardwareBase, Motion, PartId, SolvedDesign, StepKind};
use stave_scene::{Item, Posed, SceneEntry, SceneGraph};

/// Moves shorter than this (millimetres) don't count.
const STILL: f32 = 0.5;

/// The most phrases one step lists before "and N more".
const MOST_PHRASES: usize = 5;

/// Which stage of the explode a step is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Stage {
    /// Assemblies move out whole.
    Assemblies,
    /// Parts come out of their joints.
    Parts,
}

impl Stage {
    /// Its number on the list: 1 or 2.
    pub fn number(self) -> u8 {
        match self {
            Stage::Assemblies => 1,
            Stage::Parts => 2,
        }
    }
}

/// One line of "How it comes apart".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// Its stage.
    pub stage: Stage,
    /// What moves, in bold: "Drawers", "Door frames", "Carcass".
    pub lead: String,
    /// What it does: "slide forward on their slides until they clear the
    /// carcass".
    pub text: String,
}

/// What kind of assembly a group is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Group {
    /// Assemblies that slide (drawers).
    Slides,
    /// Assemblies that turn (doors, lids).
    Turns,
    /// One assembly that doesn't move in use.
    Fixed(AssemblyId),
    /// The parts in no assembly.
    Carcass,
}

fn group_of(design: &SolvedDesign, assembly: AssemblyId) -> Group {
    match design
        .assemblies
        .get(assembly.index())
        .and_then(|a| a.motion)
    {
        Some(Motion::Travel { .. }) => Group::Slides,
        Some(Motion::Turn { .. } | Motion::Linkage { .. }) => Group::Turns,
        None => Group::Fixed(assembly),
    }
}

/// The top-level assembly a part is in (`None`: the carcass).
fn top_assembly(design: &SolvedDesign, part: PartId) -> Option<AssemblyId> {
    let mut current = design.part(part)?.assembly?;
    for _ in 0..design.assemblies.len() {
        match design
            .assemblies
            .get(current.index())
            .and_then(|a| a.parent)
        {
            Some(parent) => current = parent,
            None => break,
        }
    }
    Some(current)
}

/// The direction a move mostly goes.
fn direction_of(offset: Vec3) -> Direction {
    [
        Direction::Front,
        Direction::Back,
        Direction::Up,
        Direction::Down,
        Direction::Left,
        Direction::Right,
    ]
    .into_iter()
    .max_by(|a, b| {
        offset
            .dot(render::direction(*a))
            .total_cmp(&offset.dot(render::direction(*b)))
    })
    .unwrap_or(Direction::Front)
}

/// "forward", "up", "out to the left".
fn way(direction: Direction) -> &'static str {
    match direction {
        Direction::Front => "forward",
        Direction::Back => "back",
        Direction::Up => "up",
        Direction::Down => "down",
        Direction::Left => "out to the left",
        Direction::Right => "out to the right",
    }
}

/// Names as the parts tree groups them: `rail[bottom, top]`, `foot[4]`
/// (more than three of one name), then "a, b and c".
fn names_text(names: &[String]) -> String {
    let mut grouped: Vec<(String, Vec<String>)> = Vec::new();
    for name in names {
        let (base, index) = match name.split_once('[') {
            Some((base, rest)) if rest.ends_with(']') => {
                (base.to_owned(), Some(rest.trim_end_matches(']').to_owned()))
            }
            Some(_) | None => (name.clone(), None),
        };
        match (
            index,
            grouped
                .iter_mut()
                .find(|(b, i)| *b == base && !i.is_empty()),
        ) {
            (Some(index), Some((_, indices))) => indices.push(index),
            (Some(index), None) => grouped.push((base, vec![index])),
            (None, _) => grouped.push((base, Vec::new())),
        }
    }
    let names: Vec<String> = grouped
        .into_iter()
        .map(|(base, indices)| match indices.len() {
            0 => base,
            1..=3 => format!("{base}[{}]", indices.join(", ")),
            count => format!("{base}[{count}]"),
        })
        .collect();
    match names.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// Each part's move in each stage, from the poses at 0, ½ and 1.
struct Moves {
    first: Vec<Vec3>,
    second: Vec<Vec3>,
}

fn moves(design: &SolvedDesign, graph: &SceneGraph, scene: &SceneEntry) -> Moves {
    let rest = graph.pose_with(scene, &[], 0.0);
    let half = graph.pose_with(scene, &[], 0.5);
    let full = graph.pose_with(scene, &[], 1.0);
    let mut first = vec![Vec3::ZERO; design.parts.len()];
    let mut second = vec![Vec3::ZERO; design.parts.len()];
    let at = |posed: &Posed, id| posed.shown(id).map(|m| m.w_axis.truncate());
    for id in graph.node_ids() {
        let Some(Item::Part(part)) = graph.node(id).map(|node| node.item) else {
            continue;
        };
        let (Some(a), Some(b), Some(c)) = (at(&rest, id), at(&half, id), at(&full, id)) else {
            continue;
        };
        if let Some(slot) = first.get_mut(part.index()) {
            *slot = b - a;
        }
        if let Some(slot) = second.get_mut(part.index()) {
            *slot = c - b;
        }
    }
    Moves { first, second }
}

/// The steps of "How it comes apart" for a scene, in order: the
/// assemblies (drawers, then doors, then the others), then their parts and
/// the carcass's. Empty when nothing moves.
pub fn steps(design: &SolvedDesign, graph: &SceneGraph, scene: &SceneEntry) -> Vec<Step> {
    let moves = moves(design, graph, scene);
    let moved = |list: &[Vec3], part: PartId| {
        list.get(part.index())
            .copied()
            .filter(|offset| offset.length() > STILL)
    };
    let mut steps = Vec::new();

    // Stage 1: the top-level assemblies that move out whole.
    let mut first: BTreeMap<Group, Vec<(AssemblyId, Vec3)>> = BTreeMap::new();
    for assembly in design.assemblies.iter().filter(|a| a.parent.is_none()) {
        let offset = assembly
            .parts
            .iter()
            .find_map(|part| moved(&moves.first, *part))
            .or_else(|| {
                // A door's parts may all be in sub-assemblies.
                design
                    .parts
                    .iter()
                    .filter(|p| top_assembly(design, p.id) == Some(assembly.id))
                    .find_map(|p| moved(&moves.first, p.id))
            });
        if let Some(offset) = offset {
            first
                .entry(group_of(design, assembly.id))
                .or_default()
                .push((assembly.id, offset));
        }
    }
    for (group, members) in &first {
        if let Some(step) = first_step(design, *group, members) {
            steps.push(step);
        }
    }

    // Stage 2: the parts coming out of their joints, by group.
    let mut second: BTreeMap<Group, Vec<(PartId, Vec3)>> = BTreeMap::new();
    for part in &design.parts {
        let Some(offset) = moved(&moves.second, part.id) else {
            continue;
        };
        let group = top_assembly(design, part.id)
            .map_or(Group::Carcass, |assembly| group_of(design, assembly));
        second.entry(group).or_default().push((part.id, offset));
    }
    for (group, parts) in &second {
        let count = first.get(group).map_or(1, Vec::len);
        steps.push(second_step(design, *group, parts, count));
    }
    steps
}

fn assembly_name(design: &SolvedDesign, id: AssemblyId) -> String {
    design
        .assemblies
        .get(id.index())
        .map_or_else(|| "?".to_owned(), |a| a.name.clone())
}

fn first_step(design: &SolvedDesign, group: Group, members: &[(AssemblyId, Vec3)]) -> Option<Step> {
    let (first_id, offset) = *members.first()?;
    let many = members.len() > 1;
    let direction = way(direction_of(offset));
    let hardware = |base: HardwareBase| {
        members.iter().any(|(id, _)| {
            design.hardware.iter().any(|h| {
                h.base == base
                    && h.assembly.is_some_and(|a| {
                        a == *id
                            || design.assemblies.get(a.index()).and_then(|x| x.parent) == Some(*id)
                    })
            })
        })
    };
    let (lead, text) = match group {
        Group::Slides => {
            let on = if hardware(HardwareBase::Slide) {
                if many {
                    " on their slides"
                } else {
                    " on its slides"
                }
            } else {
                ""
            };
            if many {
                (
                    "Drawers".to_owned(),
                    format!("slide {direction}{on} until they clear the carcass"),
                )
            } else {
                (
                    assembly_name(design, first_id),
                    format!("slides {direction}{on} until it clears the carcass"),
                )
            }
        }
        Group::Turns => {
            let butt = design.hardware.iter().any(|h| {
                h.is_butt_hinge()
                    && h.assembly
                        .is_some_and(|a| members.iter().any(|(id, _)| *id == a))
            });
            let hinges = hardware(HardwareBase::Hinge);
            let (verb, own) = if many {
                ("come", "their")
            } else {
                ("comes", "its")
            };
            let mut text = if hinges {
                format!("{verb} {direction} off {own} hinges")
            } else {
                format!("{verb} away {direction}")
            };
            if hinges {
                text.push_str(if butt {
                    ". One leaf goes with the door, the other stays on the frame"
                } else {
                    ". Cups go with the door, plates stay on the side"
                });
            }
            let lead = if many {
                "Doors".to_owned()
            } else {
                assembly_name(design, first_id)
            };
            (lead, text)
        }
        Group::Fixed(id) => (
            assembly_name(design, id),
            if direction == "up" {
                "lifts off the joint that holds it".to_owned()
            } else {
                format!("comes away {direction}, off the joint that holds it")
            },
        ),
        Group::Carcass => return None,
    };
    Some(Step {
        stage: Stage::Assemblies,
        lead,
        text,
    })
}

/// A part's name inside its top-level assembly: `rail[top]` for
/// `door_l.rail[top]`.
fn relative_name(design: &SolvedDesign, part: PartId) -> String {
    let Some(found) = design.part(part) else {
        return "?".to_owned();
    };
    match top_assembly(design, part).and_then(|a| design.assemblies.get(a.index())) {
        Some(assembly) => found
            .name
            .strip_prefix(&format!("{}.", assembly.name))
            .unwrap_or(&found.name)
            .to_owned(),
        None => found.name.clone(),
    }
}

/// The joint a part comes out of: its kind, when the part is the one the
/// joint's last push moves.
fn joint_of(design: &SolvedDesign, part: PartId) -> Option<String> {
    design.joints.iter().find_map(|joint| {
        joint.pairs.iter().find_map(|pair| {
            let step = pair
                .insert
                .iter()
                .rev()
                .find(|step| !matches!(step.kind, StepKind::Rotate))?;
            (step.part == part).then(|| {
                joint
                    .kind
                    .rsplit('.')
                    .next()
                    .unwrap_or(&joint.kind)
                    .replace('_', " ")
            })
        })
    })
}

/// "its dado", "their rabbets", "its half lap joint".
fn housing(kind: &str, many: bool) -> String {
    let own = if many { "their" } else { "its" };
    let plural = if many { "s" } else { "" };
    match kind {
        "dado" | "rabbet" | "groove" | "socket" | "mortise" | "housing" => {
            format!("{own} {kind}{plural}")
        }
        _ => format!("{own} {kind} joint{plural}"),
    }
}

/// A phrase's verb ("slide", "move") and the joint kind its parts come out of.
type PhraseKey = (&'static str, Option<String>);

fn second_step(
    design: &SolvedDesign,
    group: Group,
    parts: &[(PartId, Vec3)],
    count: usize,
) -> Step {
    // Phrases by (verb, joint kind), each naming its parts once.
    let mut phrases: Vec<(PhraseKey, Vec<String>)> = Vec::new();
    for (part, offset) in parts {
        let name = relative_name(design, *part);
        let kind = joint_of(design, *part);
        let verb = match (kind.is_some(), direction_of(*offset)) {
            (false, _) => "move",
            (true, Direction::Up) => "lift",
            (true, Direction::Down) => "drop",
            (true, Direction::Front | Direction::Back | Direction::Left | Direction::Right) => {
                "slide"
            }
        };
        let key = (verb, kind);
        match phrases.iter_mut().find(|(k, _)| *k == key) {
            Some((_, names)) => {
                if !names.contains(&name) {
                    names.push(name);
                }
            }
            None => phrases.push((key, vec![name])),
        }
    }
    let mut texts: Vec<String> = phrases
        .iter()
        .map(|((verb, kind), names)| {
            let many = names.len() > 1;
            let who = names_text(names);
            match kind {
                Some(kind) => format!(
                    "{who} {verb}{} out of {}",
                    if many { "" } else { "s" },
                    housing(kind, many)
                ),
                None if many => format!("{who} move apart"),
                None => format!("{who} moves away"),
            }
        })
        .collect();
    if texts.len() > MOST_PHRASES {
        let more = texts.len() - MOST_PHRASES;
        texts.truncate(MOST_PHRASES);
        texts.push(format!(
            "and {more} more {}",
            if more == 1 {
                "group of parts"
            } else {
                "groups of parts"
            }
        ));
    }
    let many = count > 1;
    let lead = match group {
        Group::Slides => if many {
            "Drawer boxes"
        } else {
            "The drawer box"
        }
        .to_owned(),
        Group::Turns => if many {
            "Door frames"
        } else {
            "The door frame"
        }
        .to_owned(),
        Group::Fixed(id) => assembly_name(design, id),
        Group::Carcass => "Carcass".to_owned(),
    };
    let verb = if many { "come" } else { "comes" };
    Step {
        stage: Stage::Parts,
        lead,
        text: format!("{verb} apart: {}", texts.join(", ")),
    }
}

/// Why the view couldn't be saved.
#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    /// The glTF couldn't be made.
    #[error("can't make the glTF file: {0}")]
    Export(#[from] stave_scene::export::ExportError),
    /// The file couldn't be written.
    #[error("can't write '{path}': {source}")]
    Write {
        /// Where.
        path: String,
        /// Why.
        source: std::io::Error,
    },
}

/// Saves the view as posed (pulled apart, opened) as a binary glTF file,
/// as `stave export` does.
pub fn save_glb(graph: &SceneGraph, posed: &Posed, path: &Path) -> Result<(), SaveError> {
    let bytes = graph.to_glb(posed)?;
    std::fs::write(path, bytes).map_err(|source| SaveError::Write {
        path: path.display().to_string(),
        source,
    })
}
