//! Ghosts of a candidate edit: what a patch would add to the design or
//! change in it, drawn over the design as it is, with the candidate's
//! verdict so a view can tint them (docs/editing.md, "The edit loop").
//!
//! The base scene is never touched: [`Ghosts`] is built beside it, from
//! the solved base design and the solved design of the candidate, and
//! holds the candidate's own scene graph and which of its nodes are
//! ghosts. Dropping the [`Ghosts`] leaves the base scene exactly as it
//! was.
//!
//! Nodes are matched between the two designs by their path in the design
//! (`shelf.board`, `door_l.stile[hinge]`),
//! never by index: adding a part renumbers the parts after it.

use std::collections::BTreeMap;
use std::sync::Arc;

use stave_model::solved::{AssemblyId, Part, SolvedDesign};

use crate::focus::Look;
use crate::graph::{BuildOptions, Item, Node, NodeId, Posed, SceneGraph};

/// The verdict a ghost is drawn with: the editor's three verdicts
/// (docs/editing.md, "Verdicts and strict commit").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum GhostVerdict {
    /// The check has not finished.
    #[default]
    Checking,
    /// The candidate checks with no error.
    Fits,
    /// The candidate would cause errors.
    Refused,
}

impl GhostVerdict {
    /// The colour a view tints a ghost with, linear RGB: a cool grey
    /// while checking, green when it fits, red when it is refused.
    pub fn tint(self) -> [f32; 3] {
        match self {
            GhostVerdict::Checking => [0.42, 0.47, 0.55],
            GhostVerdict::Fits => [0.13, 0.55, 0.22],
            GhostVerdict::Refused => [0.80, 0.12, 0.10],
        }
    }
}

/// Why a node of the candidate is a ghost.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GhostChange {
    /// The base design has nothing at this path: a new part, a new piece
    /// of hardware, a new joint's pins.
    Added,
    /// The base design has it, at another size or in another place (or,
    /// for a part, cut from other stock); the ghost shows the new one.
    Changed,
}

/// One ghost: a node of the candidate's scene graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Ghost {
    /// The node, in [`Ghosts::graph`].
    pub node: NodeId,
    /// Added or changed.
    pub change: GhostChange,
    /// For a changed node: the base scene's node it stands for.
    pub replaces: Option<NodeId>,
}

/// What a candidate adds or changes, ready to draw over the base scene.
#[derive(Debug, Clone, PartialEq)]
pub struct Ghosts {
    /// The candidate's whole scene graph; only the ghosts' nodes are
    /// drawn from it.
    graph: SceneGraph,
    /// The ghosts, in the candidate graph's node order.
    ghosts: Vec<Ghost>,
    /// Base nodes the candidate has nothing at the path of.
    removed: Vec<NodeId>,
    /// The verdict they are drawn with.
    verdict: GhostVerdict,
    /// Each assembly of the candidate with the base assembly of the same
    /// name, for drawing the ghosts over the base scene.
    assemblies: Vec<(AssemblyId, AssemblyId)>,
}

/// The ghosts drawn over the base scene, as one scene a renderer draws
/// in one frame: the base scene's nodes first, in their own order (so a
/// base [`NodeId`] means the same node here), then every ghost's node.
/// What a ghost stands in for (a changed node's original, a removed
/// node) is still drawn, faded ([`GhostOverlay::looks`]).
#[derive(Debug, Clone, PartialEq)]
pub struct GhostOverlay {
    graph: SceneGraph,
    /// How many of `graph`'s nodes are the base scene's.
    base_count: usize,
    /// Per ghost node (from `base_count` on): its node in
    /// [`Ghosts::graph`].
    candidate: Vec<NodeId>,
    /// Per node of [`Ghosts::graph`]: the node here that draws it (its
    /// ghost, or the unchanged base node at its path).
    from_candidate: Vec<Option<NodeId>>,
    /// The base nodes a ghost stands in for, in node order.
    faded: Vec<NodeId>,
}

impl GhostOverlay {
    /// The base scene with the ghosts added after its nodes.
    pub fn graph(&self) -> &SceneGraph {
        &self.graph
    }

    /// The ghosts' nodes here, in order.
    pub fn ghost_nodes(&self) -> impl Iterator<Item = NodeId> + use<> {
        (self.base_count..self.graph.nodes.len()).map(NodeId)
    }

    /// `true` when `node` is a ghost's node.
    pub fn is_ghost(&self, node: NodeId) -> bool {
        node.0 >= self.base_count && node.0 < self.graph.nodes.len()
    }

    /// For a ghost's node here: the node it is in [`Ghosts::graph`].
    /// `None` for the base scene's nodes.
    pub fn candidate_node(&self, node: NodeId) -> Option<NodeId> {
        node.0
            .checked_sub(self.base_count)
            .and_then(|index| self.candidate.get(index).copied())
    }

    /// The node here that draws a node of [`Ghosts::graph`]: its ghost,
    /// or the base node at its path when the candidate left it as it was.
    pub fn of_candidate(&self, node: NodeId) -> Option<NodeId> {
        self.from_candidate.get(node.0).copied().flatten()
    }

    /// The base nodes a ghost stands in for (changed or removed), in node
    /// order.
    pub fn faded(&self) -> &[NodeId] {
        &self.faded
    }

    /// How each node looks: what a ghost stands in for faded
    /// ([`Look::Ghost`]), the ghosts see-through ([`Look::XRay`], for a
    /// view to tint by the verdict), the rest as usual.
    pub fn looks(&self) -> Vec<Look> {
        self.graph
            .node_ids()
            .map(|id| {
                if self.is_ghost(id) {
                    Look::XRay
                } else if self.faded.binary_search(&id).is_ok() {
                    Look::Ghost
                } else {
                    Look::Normal
                }
            })
            .collect()
    }
}

/// Which list of nodes a node is in, for matching by path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum KindOfItem {
    Part,
    Hardware,
    Member,
}

/// A node's path in the design: what it is, its name, and which of the
/// nodes with that name it is (a name drawn by several nodes counts them
/// in order).
type NodePath<'n> = (KindOfItem, &'n str, usize);

fn kind_of(item: Item) -> KindOfItem {
    match item {
        Item::Part(_) => KindOfItem::Part,
        Item::Hardware(_) => KindOfItem::Hardware,
        Item::Member(_) => KindOfItem::Member,
    }
}

/// Every node's path, by path.
fn paths(graph: &SceneGraph) -> BTreeMap<NodePath<'_>, NodeId> {
    let mut seen: BTreeMap<(KindOfItem, &str), usize> = BTreeMap::new();
    let mut paths = BTreeMap::new();
    for id in graph.node_ids() {
        let Some(node) = graph.node(id) else {
            continue;
        };
        let kind = kind_of(node.item);
        let count = seen.entry((kind, node.name.as_str())).or_insert(0);
        paths.insert((kind, node.name.as_str(), *count), id);
        *count += 1;
    }
    paths
}

/// `true` when a part has the same size, place, shape and stock in both
/// designs. Its cuts are not compared: a part that only gains a new cut for
/// a new shelf is not a ghost.
fn same_part(base: &Part, candidate: &Part) -> bool {
    base.drawn == candidate.drawn
        && base.cut == candidate.cut
        && base.drawn_offset == candidate.drawn_offset
        && base.pose == candidate.pose
        && base.shape == candidate.shape
        && base.stock == candidate.stock
}

/// `true` when a node draws the same thing in both scenes.
fn same_node(
    base: &SolvedDesign,
    base_node: &Node,
    candidate: &SolvedDesign,
    candidate_node: &Node,
) -> bool {
    match (base_node.item, candidate_node.item) {
        (Item::Part(old), Item::Part(new)) => match (base.part(old), candidate.part(new)) {
            (Some(old), Some(new)) => same_part(old, new),
            (Some(_) | None, Some(_) | None) => false,
        },
        // Hardware and joint members are drawn from their exact shapes,
        // so the same shapes in the same place draw the same mesh.
        (Item::Hardware(_) | Item::Member(_), _) | (Item::Part(_), _) => {
            (Arc::ptr_eq(&base_node.local, &candidate_node.local)
                || base_node.local == candidate_node.local)
                && base_node.rest == candidate_node.rest
                && base_node.material == candidate_node.material
        }
    }
}

impl Ghosts {
    /// The ghosts of a candidate: every node of the candidate's scene that
    /// the base scene has nothing at the path of (added), or has at
    /// another size or place (changed, drawn at its new size), with the
    /// verdict `verdict`. `base_graph` is the base design's scene graph,
    /// which is only read.
    pub fn new(
        base: &SolvedDesign,
        base_graph: &SceneGraph,
        candidate: &SolvedDesign,
        options: BuildOptions,
        verdict: GhostVerdict,
    ) -> Ghosts {
        let graph = SceneGraph::build(candidate, options);
        let base_paths = paths(base_graph);
        let candidate_paths = paths(&graph);
        let mut ghosts: Vec<Ghost> = candidate_paths
            .iter()
            .filter_map(|(path, id)| {
                let node = graph.node(*id)?;
                match base_paths.get(path) {
                    None => Some(Ghost {
                        node: *id,
                        change: GhostChange::Added,
                        replaces: None,
                    }),
                    Some(base_id) => {
                        let base_node = base_graph.node(*base_id)?;
                        (!same_node(base, base_node, candidate, node)).then_some(Ghost {
                            node: *id,
                            change: GhostChange::Changed,
                            replaces: Some(*base_id),
                        })
                    }
                }
            })
            .collect();
        ghosts.sort_by_key(|ghost| ghost.node);
        let mut removed: Vec<NodeId> = base_paths
            .iter()
            .filter(|(path, _)| !candidate_paths.contains_key(*path))
            .map(|(_, id)| *id)
            .collect();
        removed.sort();
        let assemblies = candidate
            .assemblies()
            .iter()
            .filter_map(|assembly| {
                let same = base
                    .assemblies()
                    .iter()
                    .find(|old| old.name == assembly.name)?;
                Some((assembly.id, same.id))
            })
            .collect();
        Ghosts {
            graph,
            ghosts,
            removed,
            verdict,
            assemblies,
        }
    }

    /// The ghosts over the base scene `base_graph` (the scene they were
    /// made against), as one scene to draw in one frame. A ghost moves
    /// with the base assembly of the same name.
    pub fn over(&self, base_graph: &SceneGraph) -> GhostOverlay {
        let nodes: Vec<NodeId> = self.ghosts.iter().map(|ghost| ghost.node).collect();
        let graph = base_graph.with_nodes_of(&self.graph, &nodes, |id| {
            self.assemblies
                .iter()
                .find(|(candidate, _)| *candidate == id)
                .map(|(_, base)| *base)
        });
        let base_count = base_graph.nodes.len();
        let base_paths = paths(base_graph);
        let mut from_candidate = vec![None; self.graph.nodes.len()];
        for (path, id) in paths(&self.graph) {
            if let Some(slot) = from_candidate.get_mut(id.0) {
                *slot = base_paths.get(&path).copied();
            }
        }
        for (index, id) in nodes.iter().enumerate() {
            if let Some(slot) = from_candidate.get_mut(id.0) {
                *slot = Some(NodeId(base_count + index));
            }
        }
        GhostOverlay {
            graph,
            base_count,
            candidate: nodes,
            from_candidate,
            faded: self.stands_for(),
        }
    }

    /// The candidate's scene graph. Only [`Ghosts::ghosts`]' nodes are
    /// ghosts; the rest is the base scene again.
    pub fn graph(&self) -> &SceneGraph {
        &self.graph
    }

    /// The ghosts, in the candidate graph's node order.
    pub fn ghosts(&self) -> &[Ghost] {
        &self.ghosts
    }

    /// Each ghost with the node it draws.
    pub fn nodes(&self) -> impl Iterator<Item = (&Ghost, &Node)> {
        self.ghosts
            .iter()
            .filter_map(|ghost| Some((ghost, self.graph.node(ghost.node)?)))
    }

    /// The base scene's nodes the candidate has nothing at the path of.
    pub fn removed(&self) -> &[NodeId] {
        &self.removed
    }

    /// The base scene's nodes a ghost stands in for: the changed ones and
    /// the removed ones, in node order. A view draws them faded under the
    /// ghosts.
    pub fn stands_for(&self) -> Vec<NodeId> {
        let mut nodes: Vec<NodeId> = self
            .ghosts
            .iter()
            .filter_map(|ghost| ghost.replaces)
            .chain(self.removed.iter().copied())
            .collect();
        nodes.sort();
        nodes.dedup();
        nodes
    }

    /// The verdict the ghosts are drawn with.
    pub fn verdict(&self) -> GhostVerdict {
        self.verdict
    }

    /// The same ghosts with another verdict (the slow tier has finished).
    pub fn set_verdict(&mut self, verdict: GhostVerdict) {
        self.verdict = verdict;
    }

    /// The looks of the base scene's nodes under the ghosts: the nodes a
    /// ghost stands in for faded, the rest as usual.
    pub fn base_looks(&self, base_graph: &SceneGraph) -> Vec<Look> {
        let faded = self.stands_for();
        base_graph
            .node_ids()
            .map(|id| {
                if faded.binary_search(&id).is_ok() {
                    Look::Ghost
                } else {
                    Look::Normal
                }
            })
            .collect()
    }

    /// The ghosts posed for the base scene named `scene` (the candidate's
    /// scene of that name; at rest when it has none), pulled apart by
    /// `explode` as [`SceneGraph::pose_with`] does: every node of the
    /// candidate graph that is not a ghost is hidden.
    pub fn pose(&self, scene: &str, explode: f32) -> Posed {
        let mut posed = match self.graph.scene_named(scene) {
            Some(entry) => self.graph.pose_with(entry, &[], explode),
            None => self.graph.rest(),
        };
        let mut ghost = vec![false; self.graph.nodes.len()];
        for g in &self.ghosts {
            if let Some(slot) = ghost.get_mut(g.node.index()) {
                *slot = true;
            }
        }
        for (node, ghost) in self.graph.node_ids().zip(ghost) {
            if !ghost {
                posed.set_shown(node, false);
            }
        }
        posed
    }
}
