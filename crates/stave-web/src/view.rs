//! What the viewer shows of an opened project, without the browser: the
//! camera, the sliders, the exploded view, picking, and everything a frame
//! needs to be drawn.

use std::collections::BTreeSet;

use serde_json::json;
use stave_model::solved::{PartId, RoleValue, SolvedDesign};
use stave_scene::{Aabb, Item, Look, NodeId, Orbit, Posed, SceneGraph};

use crate::project::Project;

/// How strongly a ghost takes its verdict's colour (as the window draws
/// an edit's ghost).
const GHOST_TINT: f32 = 0.62;

/// A slider the page set that isn't there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("there is no motion {index}: the design has {count}")]
pub struct NoSuchMotion {
    /// The slider asked for.
    pub index: usize,
    /// How many motions the design has.
    pub count: usize,
}

/// What a click found: a part, a piece of hardware or a joint's piece,
/// and what it is joined to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picked {
    /// Its name as written: `end_l`.
    pub name: String,
    /// What it is, in words: `Part cut from walnut 12`, `Euro hinge`.
    pub what: String,
    /// The names of what it is joined to or fitted on, sorted.
    pub joined: Vec<String>,
    /// It is something the suggestion adds or changes (a ghost).
    pub suggested: bool,
    /// The suggestion takes it out or changes it: it is drawn faded under
    /// the ghosts.
    pub replaced: bool,
}

impl Picked {
    /// As JSON for the page:
    /// `{"name": …, "what": …, "joined": […], "suggested": false,
    /// "replaced": false}`.
    pub fn to_json(&self) -> String {
        json!({
            "name": self.name,
            "what": self.what,
            "joined": self.joined,
            "suggested": self.suggested,
            "replaced": self.replaced,
        })
        .to_string()
    }
}

/// Everything one frame is drawn with, per node of the scene in
/// [`NodeId`] order where it is a list.
#[derive(Debug, Clone, PartialEq)]
pub struct Drawing {
    /// Where every node is.
    pub posed: Posed,
    /// The camera.
    pub orbit: Orbit,
    /// How each node is drawn: a ghost see-through, what it stands in for
    /// faded.
    pub looks: Vec<Look>,
    /// A ghost's verdict colour; zero alpha for every other node.
    pub tints: Vec<[f32; 4]>,
    /// What is selected (the last part clicked).
    pub selection: Vec<NodeId>,
    /// What a problem is about, tinted red.
    pub problem_nodes: Vec<NodeId>,
    /// The wood problems share, drawn in red, when the design is drawn as
    /// it was checked (at rest, not pulled apart).
    pub problem_volumes: Vec<Aabb>,
}

/// An opened project as the viewer shows it.
#[derive(Debug)]
pub struct View {
    project: Project,
    orbit: Orbit,
    width: u32,
    height: u32,
    /// Per motion, how far open, from 0 (closed) to 1 (fully open).
    amounts: Vec<f32>,
    /// How far pulled apart, from 0 to 1.
    explode: f32,
    selection: Option<NodeId>,
    camera: Camera,
}

/// Whether the camera is still where the view framed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Camera {
    /// Framed to show the whole design: a new size frames it again.
    Framed,
    /// Turned or zoomed by hand: a new size leaves it.
    Moved,
}

/// `value` clamped to `0..=1`, with anything that isn't a number taken
/// as 0.
fn fraction(value: f32) -> f32 {
    if value.is_nan() {
        0.0
    } else {
        value.clamp(0.0, 1.0)
    }
}

/// A size in pixels as a float, at least 1; sizes beyond 65535 pixels
/// (never a canvas's) count as 65535.
fn pixels(size: u32) -> f32 {
    f32::from(u16::try_from(size.max(1)).unwrap_or(u16::MAX))
}

fn aspect(width: u32, height: u32) -> f32 {
    stave_render::aspect(width.max(1), height.max(1))
}

/// The box a view frames: the drawn design at rest, or a small box about
/// the origin when nothing placed.
fn bounds(project: &Project) -> Aabb {
    project
        .graph()
        .and_then(|graph| graph.bounds(&graph.rest()))
        .unwrap_or(Aabb {
            min: glam::Vec3::splat(-1.0),
            max: glam::Vec3::ONE,
        })
}

impl View {
    /// Shows `project` in a view `width` by `height` pixels, framed to
    /// show all of it, everything closed and together.
    pub fn new(project: Project, width: u32, height: u32) -> View {
        let amounts = vec![0.0; project.motions().len()];
        let orbit = Orbit::framing(bounds(&project), aspect(width, height));
        View {
            project,
            orbit,
            width,
            height,
            amounts,
            explode: 0.0,
            selection: None,
            camera: Camera::Framed,
        }
    }

    /// The project shown.
    pub fn project(&self) -> &Project {
        &self.project
    }

    /// The camera.
    pub fn orbit(&self) -> Orbit {
        self.orbit
    }

    /// The view's size in pixels.
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Follows the canvas's new size in pixels.
    /// While the camera is where the view framed it, it is framed again
    /// for the new shape.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
        match self.camera {
            Camera::Framed => {
                self.orbit = self
                    .orbit
                    .framed(bounds(&self.project), aspect(width, height));
            }
            Camera::Moved => {}
        }
    }

    /// Turns the design by a drag of `(dx, dy)` pixels.
    pub fn orbit_by(&mut self, dx: f32, dy: f32) {
        self.orbit = self.orbit.orbited(dx, dy);
        self.camera = Camera::Moved;
    }

    /// Closer (`steps > 0`) or further away.
    pub fn zoom(&mut self, steps: f32) {
        self.orbit = self.orbit.zoomed(steps);
        self.camera = Camera::Moved;
    }

    /// Opens motion `index` (a door, a drawer; in
    /// [`Project::motions`] order) to `amount`, from 0 (closed) to 1
    /// (fully open).
    pub fn set_pose(&mut self, index: usize, amount: f32) -> Result<(), NoSuchMotion> {
        let count = self.amounts.len();
        let slot = self
            .amounts
            .get_mut(index)
            .ok_or(NoSuchMotion { index, count })?;
        *slot = fraction(amount);
        Ok(())
    }

    /// Pulls the design apart by `amount`, from 0 (as built) to 1 (fully
    /// exploded).
    pub fn set_explode(&mut self, amount: f32) {
        self.explode = fraction(amount);
    }

    /// Where every node is: the sliders' motions and the exploded view
    /// applied. `None` when nothing placed.
    pub fn posed(&self) -> Option<Posed> {
        let graph = self.project.graph()?;
        let overrides: Vec<_> = self
            .project
            .motions()
            .iter()
            .zip(&self.amounts)
            .map(|(motion, amount)| (motion.assembly, amount * motion.range.full()))
            .collect();
        Some(match graph.scenes.first() {
            Some(rest) => graph.pose_with(rest, &overrides, self.explode),
            None => graph.rest(),
        })
    }

    /// What is under the pixel `(x, y)` (from the top left), selecting it;
    /// `None`, and nothing selected, when the click missed.
    pub fn pick(&mut self, x: f32, y: f32) -> Option<Picked> {
        self.selection = None;
        let graph = self.project.graph()?;
        let design = self.project.design()?;
        let posed = self.posed()?;
        let ray = self
            .orbit
            .ray(x, y, pixels(self.width), pixels(self.height));
        let hit = graph.pick(&posed, ray)?;
        let node = graph.node(hit.node)?;
        // A ghost's item is the suggestion's.
        let (design, item) = self
            .project
            .item_of(hit.node)
            .unwrap_or((design, node.item));
        self.selection = Some(hit.node);
        Some(Picked {
            name: node.name.clone(),
            what: what(design, item),
            joined: joined_to(design, item),
            suggested: self.project.ghost_nodes().contains(&hit.node),
            replaced: self.project.faded_nodes().contains(&hit.node),
        })
    }

    /// Everything the next frame is drawn with; `None` when nothing
    /// placed.
    pub fn drawing(&self) -> Option<Drawing> {
        let graph = self.project.graph()?;
        let posed = self.posed()?;
        let count = graph.nodes.len();
        let mut looks = self.project.looks();
        looks.resize(count, Look::Normal);
        let mut tints = vec![[0.0; 4]; count];
        if let Some([red, green, blue]) = self.project.ghost_tint() {
            for node in self.project.ghost_nodes() {
                if let Some(tint) = tints.get_mut(node.index()) {
                    *tint = [red, green, blue, GHOST_TINT];
                }
            }
        }
        let as_checked = self.explode == 0.0 && self.amounts.iter().all(|amount| *amount == 0.0);
        let highlight = self.project.highlight();
        Some(Drawing {
            posed,
            orbit: self.orbit,
            looks,
            tints,
            selection: self.selection.into_iter().collect(),
            problem_nodes: highlight.nodes.clone(),
            problem_volumes: if as_checked {
                highlight.volumes.clone()
            } else {
                Vec::new()
            },
        })
    }

    /// The scene drawn, when something placed.
    pub fn graph(&self) -> Option<&SceneGraph> {
        self.project.graph()
    }

    /// What the page shows beside the view, as JSON: the title, the
    /// problems, the suggestion's verdict and a slider per motion.
    ///
    /// ```text
    /// {"title": "Box", "placed": true, "errors": 0,
    ///  "suggestion": {"verdict": "fits", "errors": 0} | null,
    ///  "motions": [{"name": "lid", "turns": true, "full": 1.57}]}
    /// ```
    pub fn summary_json(&self) -> String {
        let project = &self.project;
        let suggestion = project.verdict().map(|verdict| {
            json!({
                "verdict": verdict.word(),
                "errors": project.suggestion_error_count().unwrap_or(0),
            })
        });
        let motions: Vec<_> = project
            .motions()
            .iter()
            .map(|motion| {
                json!({
                    "name": motion.name,
                    "turns": motion.range.turns(),
                    "full": motion.range.full(),
                })
            })
            .collect();
        json!({
            "title": project.title(),
            "placed": project.graph().is_some(),
            "errors": project.error_count(),
            "suggestion": suggestion,
            "motions": motions,
        })
        .to_string()
    }
}

/// A name as written, in words: `walnut_12` is `walnut 12`, and a kind
/// from a library (`hw.euro_hinge`) is named by its last part.
fn in_words(name: &str) -> String {
    name.rsplit('.')
        .next()
        .unwrap_or(name)
        .replace('_', " ")
        .trim()
        .to_owned()
}

/// `text` with its first letter a capital.
fn capitalised(text: &str) -> String {
    let mut letters = text.chars();
    match letters.next() {
        Some(first) => first.to_uppercase().chain(letters).collect(),
        None => String::new(),
    }
}

/// What an item is, in words: `Part cut from walnut 12`, `Euro hinge`,
/// `Piece of the dado joint shelf l`.
fn what(design: &SolvedDesign, item: Item) -> String {
    match item {
        Item::Part(id) => design.part(id).map_or_else(
            || "Part".to_owned(),
            |part| format!("Part cut from {}", in_words(&part.stock.name)),
        ),
        Item::Hardware(id) => design.hardware_piece(id).map_or_else(
            || "Hardware".to_owned(),
            |hardware| capitalised(&in_words(&hardware.kind)),
        ),
        Item::Member(member) => design.joint(member.joint).map_or_else(
            || "Piece of a joint".to_owned(),
            |joint| {
                format!(
                    "Piece of the {} joint {}",
                    in_words(&joint.kind),
                    in_words(&joint.name)
                )
            },
        ),
    }
}

/// The names of the parts and hardware an item is joined to or fitted
/// on, sorted.
fn joined_to(design: &SolvedDesign, item: Item) -> Vec<String> {
    let mut parts: BTreeSet<PartId> = BTreeSet::new();
    let mut hardware = BTreeSet::new();
    match item {
        Item::Part(part) => {
            for joint in design.joints() {
                for pair in &joint.pairs {
                    if pair.roles.iter().any(|role| role.part == part) {
                        parts.extend(pair.roles.iter().map(|role| role.part));
                    }
                }
            }
            parts.remove(&part);
            for piece in design.hardware() {
                let on_it = piece.piece == part
                    || piece
                        .roles
                        .iter()
                        .any(|(_, value)| *value == RoleValue::Part(part));
                if on_it {
                    hardware.insert(piece.name.clone());
                }
            }
        }
        Item::Hardware(id) => {
            if let Some(piece) = design.hardware_piece(id) {
                parts.insert(piece.piece);
                for (_, value) in &piece.roles {
                    match value {
                        RoleValue::Part(part) => {
                            parts.insert(*part);
                        }
                        RoleValue::Region(_) => {}
                    }
                }
            }
        }
        Item::Member(member) => {
            if let Some(pair) = design
                .joint(member.joint)
                .and_then(|joint| joint.pair(member.pair))
            {
                parts.extend(pair.roles.iter().map(|role| role.part));
            }
        }
    }
    let mut names: BTreeSet<String> = parts
        .into_iter()
        .filter_map(|id| design.part(id).map(|part| part.name.clone()))
        .collect();
    names.extend(hardware);
    names.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_put_in_words() {
        assert_eq!(in_words("walnut_12"), "walnut 12");
        assert_eq!(in_words("hw.euro_hinge"), "euro hinge");
        assert_eq!(capitalised(&in_words("euro_hinge")), "Euro hinge");
        assert_eq!(capitalised(""), "");
    }

    #[test]
    fn fractions_are_clamped() {
        assert_eq!(fraction(-1.0), 0.0);
        assert_eq!(fraction(0.25), 0.25);
        assert_eq!(fraction(7.0), 1.0);
        assert_eq!(fraction(f32::NAN), 0.0);
    }
}
