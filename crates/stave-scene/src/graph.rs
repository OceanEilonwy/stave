//! Nodes, materials and scene poses.

use glam::{Mat4, Quat, Vec3};
use stave_geom::linkage::LinkagePath;
use stave_geom::solid::Solid;
use stave_geom::{Aabb, Extent, Geometry, render};
use stave_lang::placement::PartAxis;
use stave_model::solved::{
    AssemblyId, HardwareBase, HardwareData, HardwareId, Motion, Part, PartId, PoseValue, SceneId,
    SceneItem, SolvedDesign, StepKind,
};
use stave_schema::species::{Species, Srgb};
use stave_schema::words::StockForm;

/// What a node draws: a part or a piece of hardware. This is what picking
/// returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Item {
    /// A part.
    Part(PartId),
    /// A piece of hardware (drawn as its envelope).
    Hardware(HardwareId),
}

/// The index of a node in [`SceneGraph::nodes`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(usize);

impl NodeId {
    /// The index into [`SceneGraph::nodes`].
    pub fn index(self) -> usize {
        self.0
    }
}

/// Wood: the species' colour and how its figure is drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Wood {
    /// The species.
    pub species: Species,
    /// Its colour, linear RGB.
    pub colour: [f32; 3],
    /// Where the tree's centre lies, in the vertex grain coordinates
    /// (the second and third: across the grain). Growth rings are
    /// circles about it.
    pub pith: [f32; 2],
    /// How strongly the grain shows: 1 for solid wood, less for sheet
    /// goods, 0 for MDF.
    pub figure: f32,
}

/// How a node is shaded.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Material {
    /// A wooden part.
    Wood(Wood),
    /// Hardware: brushed metal.
    Metal,
}

impl Material {
    /// The base colour, linear RGB.
    pub fn base_colour(&self) -> [f32; 3] {
        match self {
            Material::Wood(wood) => wood.colour,
            Material::Metal => METAL_COLOUR,
        }
    }
}

/// Brushed steel, linear RGB.
const METAL_COLOUR: [f32; 3] = [0.56, 0.58, 0.61];

/// One drawable thing.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    /// The part or hardware it draws.
    pub item: Item,
    /// Its name as written: `door_l.stile[hinge]`.
    pub name: String,
    /// Its mesh and outline at rest, in world coordinates.
    pub geometry: Geometry,
    /// How it is shaded.
    pub material: Material,
    /// The assemblies it is inside, outermost first.
    pub assemblies: Vec<AssemblyId>,
    /// A cup hinge's link: it turns about its arm pin as the hinge's
    /// assembly opens, on top of what its own assemblies do.
    pub follows: Option<LinkFollower>,
}

/// How a cup hinge's link moves: with the linkage of its hinge, at the
/// amount its door's assembly is opened.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinkFollower {
    /// The assembly the hinge opens (the door).
    pub assembly: AssemblyId,
    /// The hinge's linkage.
    path: LinkagePath,
    /// Which link (0 or 1).
    link: usize,
}

impl LinkFollower {
    /// The link's transform with its assembly opened by `amount` radians.
    fn transform(&self, amount: f32) -> Mat4 {
        let theta = f64::from(amount).clamp(0.0, self.path.opening());
        self.path
            .pose(theta)
            .and_then(|pose| pose.links.get(self.link).copied())
            .map_or(Mat4::IDENTITY, render::transform)
    }
}

/// A motion with its amounts in render units.
#[derive(Debug, Clone, Copy, PartialEq)]
enum RenderMotion {
    Turn {
        through: Vec3,
        /// Unit axis; turning by a positive angle opens.
        axis: Vec3,
        /// Radians.
        opening: f32,
    },
    /// A cup hinge's four-bar linkage.
    Linkage {
        path: LinkagePath,
        /// Radians.
        opening: f32,
    },
    Travel {
        toward: Vec3,
        /// Millimetres.
        travel: f32,
    },
}

impl RenderMotion {
    fn from_motion(motion: &Motion) -> RenderMotion {
        match *motion {
            Motion::Turn {
                pivot,
                opening_angle,
            } => {
                // Turning about free × swing carries the free side toward
                // the swing side.
                let free = render::direction(pivot.free_side);
                let swing = render::direction(pivot.swing_toward);
                RenderMotion::Turn {
                    through: render::point(pivot.through),
                    axis: free.cross(swing).normalize_or(render::axis(pivot.axis)),
                    opening: render::radians(opening_angle),
                }
            }
            Motion::Linkage { linkage } => RenderMotion::Linkage {
                path: LinkagePath::new(&linkage),
                opening: render::radians(linkage.opening_angle()),
            },
            Motion::Travel { toward, travel } => RenderMotion::Travel {
                toward: render::direction(toward),
                travel: render::mm(travel),
            },
        }
    }

    /// The amount a pose value means for this motion (radians or mm).
    fn amount(&self, value: PoseValue) -> f32 {
        match (self, value) {
            (_, PoseValue::Closed) => 0.0,
            (
                RenderMotion::Turn { opening, .. } | RenderMotion::Linkage { opening, .. },
                PoseValue::Open,
            ) => *opening,
            (RenderMotion::Travel { travel, .. }, PoseValue::Open) => *travel,
            (RenderMotion::Turn { .. } | RenderMotion::Linkage { .. }, PoseValue::Angle(angle)) => {
                render::radians(angle)
            }
            (RenderMotion::Travel { .. }, PoseValue::Travel(length)) => render::mm(length),
            // A value of the other kind is a type error the checker
            // reports; it doesn't move anything.
            (RenderMotion::Turn { .. } | RenderMotion::Linkage { .. }, PoseValue::Travel(_))
            | (RenderMotion::Travel { .. }, PoseValue::Angle(_)) => 0.0,
        }
    }

    fn transform(&self, amount: f32) -> Mat4 {
        match *self {
            RenderMotion::Turn { through, axis, .. } => {
                Mat4::from_translation(through)
                    * Mat4::from_quat(Quat::from_axis_angle(axis, amount))
                    * Mat4::from_translation(-through)
            }
            RenderMotion::Linkage { path, .. } => {
                let theta = f64::from(amount).clamp(0.0, path.opening());
                path.coupler(theta)
                    .map_or(Mat4::IDENTITY, render::transform)
            }
            RenderMotion::Travel { toward, .. } => Mat4::from_translation(toward * amount),
        }
    }
}

/// A scene of the design, ready to pose.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneEntry {
    /// Its id in the design.
    pub id: SceneId,
    /// Its name (`rest`, `all_open`).
    pub name: String,
    /// Its title, or the name when it has none.
    pub title: String,
    /// Motion amounts per assembly (radians or millimetres).
    moves: Vec<(AssemblyId, f32)>,
    show: Option<Vec<SceneItem>>,
    hide: Vec<SceneItem>,
    /// The scene asks for the parts to be pulled apart (not drawn yet).
    pub explode: bool,
    /// The scene asks for a section view (not drawn yet).
    pub section: bool,
}

/// Options for building a scene graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BuildOptions {
    /// Which box boards are drawn with.
    pub extent: Extent,
}

/// The whole design, ready to draw.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneGraph {
    /// The project's title.
    pub title: String,
    /// Parts first (in part order), then hardware: each piece's member on
    /// its piece, followed by its fixed member when it has one.
    pub nodes: Vec<Node>,
    /// The scenes; the first is the built-in `rest`.
    pub scenes: Vec<SceneEntry>,
    /// Each assembly's motion, by [`AssemblyId`].
    motions: Vec<Option<RenderMotion>>,
    /// Per node: how far its assembly moves out in an exploded view, the
    /// first stage (millimetres, render frame).
    explode_assembly: Vec<Vec3>,
    /// Per node: how far it then moves out within its assembly, the second
    /// stage.
    explode_part: Vec<Vec3>,
}

/// What a viewer shows for a problem: nodes to tint and the shared
/// volume to draw in red.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Highlight {
    /// The nodes involved.
    pub nodes: Vec<NodeId>,
    /// The shared volumes, in the render frame, already posed.
    pub volumes: Vec<Aabb>,
}

/// The range of an assembly's motion, for a slider.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionRange {
    /// `true` for a hinge (radians), `false` for a slide (millimetres).
    pub turns: bool,
    /// Fully open.
    pub full: f32,
}

/// Where every node is in one scene, and whether it is shown.
#[derive(Debug, Clone, PartialEq)]
pub struct Posed {
    /// Per node: rest to posed.
    pub transforms: Vec<Mat4>,
    /// Per node: shown in this scene.
    pub visible: Vec<bool>,
}

impl Posed {
    /// The pose of one node, if it is shown.
    pub fn shown(&self, node: NodeId) -> Option<Mat4> {
        match self.visible.get(node.0) {
            Some(true) => self.transforms.get(node.0).copied(),
            Some(false) | None => None,
        }
    }
}

/// sRGB to linear, per channel.
fn linear(colour: Srgb) -> [f32; 3] {
    [colour.red, colour.green, colour.blue].map(|channel| {
        let c = f32::from(channel) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    })
}

/// A small deterministic hash of a part id, in `0..1`, so neighbouring
/// boards don't share the same figure.
fn jitter(id: PartId, salt: u32) -> f32 {
    let mut x = id.get().wrapping_mul(0x9E37_79B9) ^ salt.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    f32::from(u16::try_from(x & 0xFFFF).unwrap_or(0)) / 65535.0
}

fn wood(part: &Part) -> Wood {
    let species = part.stock.species;
    let figure = match part.stock.form {
        StockForm::Solid | StockForm::Dowel => 1.0,
        StockForm::Plywood => 0.55,
        StockForm::Mdf => 0.0,
    };
    // Across-grain sizes, in grain coordinates (second, third).
    let width = render::mm(part.cut.width);
    let thickness = render::mm(part.cut.thickness);
    let (across, through) = match part.stock.form {
        StockForm::Dowel => (width, thickness),
        StockForm::Solid | StockForm::Plywood | StockForm::Mdf => match part.grain {
            stave_schema::words::Grain::Length => (width, thickness),
            stave_schema::words::Grain::Width => (render::mm(part.cut.length), thickness),
        },
    };
    let round = !matches!(part.shape, stave_model::solved::Shape::Board);
    let pith = if round {
        // Turned from the middle of a blank: rings about the axis, a
        // little off centre.
        [
            across * (0.5 + 0.08 * (jitter(part.id, 1) - 0.5)),
            through * (0.5 + 0.08 * (jitter(part.id, 2) - 0.5)),
        ]
    } else {
        // A flat-sawn board: the tree's centre well below the board, off
        // to one side, so the faces show long arches and the ends show
        // rings.
        [
            across * (jitter(part.id, 3) * 1.6 - 0.3),
            -(150.0 + 250.0 * jitter(part.id, 4)) - through,
        ]
    };
    Wood {
        species,
        colour: linear(species.colour()),
        pith,
        figure,
    }
}

impl SceneGraph {
    /// Builds the graph of a design.
    pub fn build(design: &SolvedDesign, options: BuildOptions) -> SceneGraph {
        let assemblies_of = |mut assembly: Option<AssemblyId>| {
            let mut path = Vec::new();
            while let Some(id) = assembly {
                if path.contains(&id) {
                    break;
                }
                path.push(id);
                assembly = design.assemblies.get(id.index()).and_then(|a| a.parent);
            }
            path.reverse();
            path
        };
        let mut nodes = Vec::with_capacity(design.parts.len() + design.hardware.len());
        for part in &design.parts {
            let geometry = match options.extent {
                Extent::Cut => stave_geom::cut_part(part, &Solid::of(part, design)),
                Extent::Drawn => stave_geom::part(part, Extent::Drawn),
            };
            let material = Material::Wood(wood(part));
            nodes.push(Node {
                item: Item::Part(part.id),
                name: part.name.clone(),
                geometry,
                material,
                assemblies: assemblies_of(part.assembly),
                follows: None,
            });
        }
        for hardware in &design.hardware {
            nodes.push(Node {
                item: Item::Hardware(hardware.id),
                name: hardware.name.clone(),
                geometry: stave_geom::hardware(hardware),
                material: Material::Metal,
                assemblies: assemblies_of(hardware.assembly),
                follows: None,
            });
            // The member screwed to the carcass stays put when the assembly
            // the hardware moves moves: it belongs to the assembly outside.
            if let Some(geometry) = stave_geom::hardware_fixed(hardware) {
                let outside = hardware
                    .assembly
                    .and_then(|id| design.assemblies.get(id.index()))
                    .and_then(|assembly| assembly.parent);
                let member = match hardware.base {
                    HardwareBase::Hinge if hardware.is_butt_hinge() => "fixed leaf",
                    HardwareBase::Hinge if hardware.mechanism.is_some() => "plate and arm",
                    HardwareBase::Hinge => "plate",
                    HardwareBase::Slide => "cabinet member",
                    HardwareBase::Pull => "fixed member",
                };
                nodes.push(Node {
                    item: Item::Hardware(hardware.id),
                    name: format!("{} {member}", hardware.name),
                    geometry,
                    material: Material::Metal,
                    assemblies: assemblies_of(outside),
                    follows: None,
                });
                // A cup hinge's links: each turns about its pin in the arm as
                // the door opens.
                if let (Some(links), Some(mechanism), Some(assembly)) = (
                    stave_geom::hardware_links(hardware),
                    hardware.mechanism.as_ref(),
                    hardware.assembly,
                ) {
                    let path = LinkagePath::new(&mechanism.linkage);
                    for (link, (geometry, label)) in links
                        .into_iter()
                        .zip(["inner link", "outer link"])
                        .enumerate()
                    {
                        nodes.push(Node {
                            item: Item::Hardware(hardware.id),
                            name: format!("{} {label}", hardware.name),
                            geometry,
                            material: Material::Metal,
                            assemblies: assemblies_of(outside),
                            follows: Some(LinkFollower {
                                assembly,
                                path,
                                link,
                            }),
                        });
                    }
                }
            }
        }
        let motions: Vec<Option<RenderMotion>> = design
            .assemblies
            .iter()
            .map(|assembly| assembly.motion.as_ref().map(RenderMotion::from_motion))
            .collect();
        let scenes = design
            .scenes
            .iter()
            .map(|scene| SceneEntry {
                id: scene.id,
                name: scene.name.clone(),
                title: scene.title.clone().unwrap_or_else(|| scene.name.clone()),
                moves: scene
                    .poses
                    .iter()
                    .filter_map(|pose| {
                        let motion = motions.get(pose.assembly.index())?.as_ref()?;
                        Some((pose.assembly, motion.amount(pose.value)))
                    })
                    .collect(),
                show: scene.show.clone(),
                hide: scene.hide.clone(),
                explode: scene.explode,
                section: scene.section.is_some(),
            })
            .collect();
        let exploded = Exploded::of(design);
        let (explode_assembly, explode_part) = nodes
            .iter()
            .map(|node| exploded.of_node(design, node))
            .unzip();
        SceneGraph {
            title: design.project.title.clone(),
            nodes,
            scenes,
            motions,
            explode_assembly,
            explode_part,
        }
    }

    /// The range of an assembly's motion; `None` when it has none.
    pub fn motion_range(&self, assembly: AssemblyId) -> Option<MotionRange> {
        let motion = self.motions.get(assembly.index()).copied().flatten()?;
        Some(match motion {
            RenderMotion::Turn { opening, .. } | RenderMotion::Linkage { opening, .. } => {
                MotionRange {
                    turns: true,
                    full: opening,
                }
            }
            RenderMotion::Travel { travel, .. } => MotionRange {
                turns: false,
                full: travel,
            },
        })
    }

    /// The amount (radians or millimetres) a pose value means for an
    /// assembly's motion.
    pub fn amount(&self, assembly: AssemblyId, value: PoseValue) -> Option<f32> {
        let motion = self.motions.get(assembly.index()).copied().flatten()?;
        Some(motion.amount(value))
    }

    /// Where a node goes in a fully exploded view, from rest.
    pub fn explode_offset(&self, node: NodeId) -> Vec3 {
        self.staged_offset(node.0, 1.0)
    }

    /// Where a node is at an explode amount: the first half of the range
    /// moves the assemblies out as wholes, the second half pulls each
    /// apart into its parts.
    fn staged_offset(&self, index: usize, explode: f32) -> Vec3 {
        let t = explode.clamp(0.0, 1.0);
        let first = (t * 2.0).min(1.0);
        let second = (t * 2.0 - 1.0).max(0.0);
        self.explode_assembly
            .get(index)
            .copied()
            .unwrap_or(Vec3::ZERO)
            * first
            + self.explode_part.get(index).copied().unwrap_or(Vec3::ZERO) * second
    }

    /// The node ids, in order.
    pub fn node_ids(&self) -> impl Iterator<Item = NodeId> + use<> {
        (0..self.nodes.len()).map(NodeId)
    }

    /// A node.
    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id.0)
    }

    /// The (first) node that draws an item.
    pub fn node_of(&self, item: Item) -> Option<NodeId> {
        self.nodes
            .iter()
            .position(|node| node.item == item)
            .map(NodeId)
    }

    /// A scene by name.
    pub fn scene_named(&self, name: &str) -> Option<&SceneEntry> {
        self.scenes.iter().find(|scene| scene.name == name)
    }

    /// Everything at rest and shown.
    pub fn rest(&self) -> Posed {
        Posed {
            transforms: vec![Mat4::IDENTITY; self.nodes.len()],
            visible: vec![true; self.nodes.len()],
        }
    }

    /// Every node posed for a scene: its assemblies' motions applied
    /// (outermost last, so a drawer inside a moving cabinet moves with it)
    /// and `show`/`hide` applied.
    pub fn pose(&self, scene: &SceneEntry) -> Posed {
        self.pose_with(scene, &[], if scene.explode { 1.0 } else { 0.0 })
    }

    /// Every node posed for a scene, with some assemblies' motions set to
    /// other amounts (radians or millimetres; a slider, a problem's pose
    /// at contact), and pulled apart by `explode` (0 as built, 1 fully
    /// exploded): up to one half each assembly moves out as a whole, then
    /// its parts move apart along the directions their joints go together.
    pub fn pose_with(
        &self,
        scene: &SceneEntry,
        overrides: &[(AssemblyId, f32)],
        explode: f32,
    ) -> Posed {
        let amount = |assembly: AssemblyId| {
            overrides
                .iter()
                .chain(&scene.moves)
                .find(|(id, _)| *id == assembly)
                .map(|(_, amount)| *amount)
        };
        let matches = |node: &Node, item: &SceneItem| match *item {
            SceneItem::Part(id) => node.item == Item::Part(id),
            SceneItem::Hardware(id) => node.item == Item::Hardware(id),
            SceneItem::Assembly(id) => node.assemblies.contains(&id),
        };
        let mut posed = self.rest();
        for (index, node) in self.nodes.iter().enumerate() {
            let mut transform = Mat4::IDENTITY;
            for assembly in &node.assemblies {
                let motion = self.motions.get(assembly.index()).copied().flatten();
                if let (Some(motion), Some(amount)) = (motion, amount(*assembly)) {
                    transform *= motion.transform(amount);
                }
            }
            if let Some(follower) = node.follows
                && let Some(amount) = amount(follower.assembly)
            {
                transform *= follower.transform(amount);
            }
            transform = Mat4::from_translation(self.staged_offset(index, explode)) * transform;
            let shown = scene
                .show
                .as_ref()
                .is_none_or(|show| show.iter().any(|item| matches(node, item)))
                && !scene.hide.iter().any(|item| matches(node, item));
            if let Some(slot) = posed.transforms.get_mut(index) {
                *slot = transform;
            }
            if let Some(slot) = posed.visible.get_mut(index) {
                *slot = shown;
            }
        }
        posed
    }

    /// The box around every shown node, posed; `None` when nothing is
    /// shown.
    pub fn bounds(&self, posed: &Posed) -> Option<Aabb> {
        self.node_ids()
            .filter_map(|id| {
                let transform = posed.shown(id)?;
                Some(self.node(id)?.geometry.bounds.transformed(transform))
            })
            .reduce(Aabb::union)
    }

    /// The box around some nodes, posed.
    pub fn bounds_of(&self, posed: &Posed, nodes: &[NodeId]) -> Option<Aabb> {
        nodes
            .iter()
            .filter_map(|id| {
                let transform = posed.transforms.get(id.0)?;
                Some(self.node(*id)?.geometry.bounds.transformed(*transform))
            })
            .reduce(Aabb::union)
    }
}

/// The world direction an assembly step moves its part in, as a unit
/// vector in the render frame (a push turned by `angle` leans toward the
/// next axis).
fn step_direction(part: &Part, axis: PartAxis, angle: Option<stave_lang::units::Angle>) -> Vec3 {
    let o = part.pose.orientation;
    let along = render::direction(o.direction(axis));
    let next = match axis {
        PartAxis::Length => PartAxis::Width,
        PartAxis::Width => PartAxis::Thickness,
        PartAxis::Thickness => PartAxis::Length,
    };
    match angle {
        Some(angle) => {
            let a = render::radians(angle);
            along * a.cos() + render::direction(o.direction(next)) * a.sin()
        }
        None => along,
    }
}

/// The middle of a part's cut box, render frame.
fn middle(part: &Part) -> Vec3 {
    let b = part.cut_box();
    (render::point(b.min()) + render::point(b.max())) * 0.5
}

/// How far a part moves out along a direction: the design's base distance
/// plus a third of its size that way, so the joinery shows.
fn explode_distance(part: &Part, direction: Vec3, base: f32) -> f32 {
    let size = render::size(part.cut);
    let o = part.pose.orientation;
    let extent: f32 = [PartAxis::Length, PartAxis::Width, PartAxis::Thickness]
        .iter()
        .zip([size.x, size.y, size.z])
        .map(|(axis, s)| s * render::direction(o.direction(*axis)).dot(direction).abs())
        .sum();
    base + extent / 3.0
}

/// A direction signed to point away from `from`, seen from `part`; when
/// they're centred on each other that way, toward the viewer, then up,
/// then right.
fn away_from(direction: Vec3, part: Vec3, from: Vec3) -> Vec3 {
    let along = (part - from).dot(direction);
    let flip = if along.abs() > 1.0 {
        along < 0.0
    } else {
        direction.dot(Vec3::new(0.001, 0.01, 1.0)) < 0.0
    };
    if flip { -direction } else { direction }
}

/// Per part (by [`PartId`]): where it goes in a fully exploded view
/// (grammar, "Scenes"), deterministically.
///
/// A part that is the moving part of a joint's last `insert` step moves
/// out along the reverse of that step, away from the others of the pair
/// (the last push or drive: a `rotate` turns a part in place).
/// A part that would move out of two of its pairs in opposite directions
/// (a rail tenoned into two stiles, an apron into two legs) stays, and the
/// parts it is joined to move away from it instead. A part's moves in
/// different directions add up (a table leg moves out diagonally from its
/// two aprons), and a part moving out of a part that itself moves goes
/// with it.
fn explode_offsets(design: &SolvedDesign, base: f32) -> Vec<Vec3> {
    /// A part's moves (direction, distance) and the part it moves out of
    /// first.
    type Moves = (Vec<(Vec3, f32)>, Option<PartId>);
    struct Move {
        mover: PartId,
        direction: Vec3,
        partners: Vec<PartId>,
    }
    let mut moves: Vec<Move> = Vec::new();
    for joint in &design.joints {
        for pair in &joint.pairs {
            // A turn moves nothing out: the last push (or drive) does.
            let Some(step) = pair
                .insert
                .iter()
                .rev()
                .find(|step| !matches!(step.kind, StepKind::Rotate))
            else {
                continue;
            };
            let Some(part) = design.part(step.part) else {
                continue;
            };
            let partners: Vec<PartId> = pair
                .roles
                .iter()
                .map(|r| r.part)
                .chain(pair.made_parts.iter().copied())
                .filter(|p| *p != part.id)
                .collect();
            // A joint between an assembly and the rest moves the assembly
            // as a whole (the first stage), not its parts.
            let inside = partners
                .iter()
                .filter_map(|p| design.part(*p))
                .all(|p| p.assembly == part.assembly);
            if !inside {
                continue;
            }
            let centres: Vec<Vec3> = partners
                .iter()
                .filter_map(|p| design.part(*p))
                .map(middle)
                .collect();
            let count = f32::from(u16::try_from(centres.len().max(1)).unwrap_or(u16::MAX));
            let centre = centres.iter().fold(Vec3::ZERO, |a, b| a + *b) / count;
            let direction = away_from(
                step_direction(part, step.axis, step.angle),
                middle(part),
                centre,
            );
            moves.push(Move {
                mover: part.id,
                direction,
                partners,
            });
        }
    }
    let pinned = |part: PartId| {
        let directions: Vec<Vec3> = moves
            .iter()
            .filter(|m| m.mover == part)
            .map(|m| m.direction)
            .collect();
        directions
            .iter()
            .enumerate()
            .any(|(i, a)| directions.iter().skip(i + 1).any(|b| a.dot(*b) < -0.5))
    };
    // Per part: its moves (direction, distance) and the part it moves out
    // of first.
    let mut own: Vec<Moves> = vec![(Vec::new(), None); design.parts.len()];
    let mut add = |part: &Part, direction: Vec3, parent: PartId| {
        if let Some((list, first)) = own.get_mut(part.id.index()) {
            let distance = explode_distance(part, direction, base);
            match list.iter_mut().find(|(d, _)| d.dot(direction) > 0.99) {
                Some(entry) => entry.1 = entry.1.max(distance),
                None => list.push((direction, distance)),
            }
            first.get_or_insert(parent);
        }
    };
    for m in &moves {
        let Some(mover) = design.part(m.mover) else {
            continue;
        };
        if pinned(m.mover) {
            // A partner centred on the held part along the step (a stile
            // either side of a panel's groove) has no side to move to.
            for partner in m.partners.iter().filter_map(|p| design.part(*p)) {
                if (middle(partner) - middle(mover)).dot(m.direction).abs() < 1.0 {
                    continue;
                }
                let direction = away_from(m.direction, middle(partner), middle(mover));
                add(partner, direction, m.mover);
            }
        } else if let Some(parent) = m.partners.first() {
            add(mover, m.direction, *parent);
        }
    }
    // Compose along chains of parts moving out of moving parts.
    (0..design.parts.len())
        .map(|index| {
            let mut offset = Vec3::ZERO;
            let mut current = Some(index);
            let mut seen = Vec::new();
            while let Some(at) = current {
                if seen.contains(&at) {
                    break;
                }
                seen.push(at);
                let Some((list, parent)) = own.get(at) else {
                    break;
                };
                offset += list
                    .iter()
                    .map(|(d, distance)| *d * *distance)
                    .fold(Vec3::ZERO, |a, b| a + b);
                current = parent.map(PartId::index);
            }
            offset
        })
        .collect()
}

/// Two cut boxes that touch or overlap.
fn touching(a: &stave_model::solved::WorldBox, b: &stave_model::solved::WorldBox) -> bool {
    stave_lang::placement::Axis::ALL.into_iter().all(|axis| {
        a.min().get(axis) <= b.max().get(axis) && b.min().get(axis) <= a.max().get(axis)
    })
}

/// The box around some parts, render frame.
fn bounds_of<'d>(parts: impl Iterator<Item = &'d Part>) -> Option<Aabb> {
    parts
        .filter_map(|p| {
            let b = p.cut_box();
            Aabb::around([render::point(b.min()), render::point(b.max())])
        })
        .reduce(Aabb::union)
}

/// The size of a box along a direction.
fn extent_along(bounds: &Aabb, direction: Vec3) -> f32 {
    bounds.size().dot(direction.abs())
}

/// The exploded view of a design, in two stages: each assembly as a whole,
/// then the parts inside each assembly (and the parts of the design that
/// are in none) about where their assembly went.
struct Exploded {
    /// Per assembly (by [`AssemblyId`]): its move, its parents' included.
    assemblies: Vec<Vec3>,
    /// Per part (by [`PartId`]): its move within its assembly.
    parts: Vec<Vec3>,
}

impl Exploded {
    fn of(design: &SolvedDesign) -> Exploded {
        // Distances grow with the design, so the joinery shows at a glance
        // whatever its size.
        let size = bounds_of(design.parts.iter()).map_or(0.0, |b| b.size().length());
        let base = (size * 0.06).max(40.0);
        let parts = explode_offsets(design, base);
        let own: Vec<Vec3> = design
            .assemblies
            .iter()
            .map(|assembly| Exploded::assembly_move(design, assembly, base))
            .collect();
        let assemblies = design
            .assemblies
            .iter()
            .map(|assembly| {
                let mut offset = Vec3::ZERO;
                let mut current = Some(assembly.id);
                let mut seen = Vec::new();
                while let Some(id) = current {
                    if seen.contains(&id) {
                        break;
                    }
                    seen.push(id);
                    offset += own.get(id.index()).copied().unwrap_or(Vec3::ZERO);
                    current = design.assemblies.get(id.index()).and_then(|a| a.parent);
                }
                offset
            })
            .collect();
        Exploded { assemblies, parts }
    }

    /// How an assembly moves out as a whole: a drawer forward along its
    /// slides, clear of its depth; a door or lid off its hinges, toward
    /// the side it swings to; anything else along the joint that attaches
    /// it to the rest of the design.
    fn assembly_move(
        design: &SolvedDesign,
        assembly: &stave_model::solved::Assembly,
        base: f32,
    ) -> Vec3 {
        let members = || assembly.parts.iter().filter_map(|p| design.part(*p));
        let Some(bounds) = bounds_of(members()) else {
            return Vec3::ZERO;
        };
        match assembly.motion {
            Some(Motion::Travel { toward, .. }) => {
                let direction = render::direction(toward);
                direction * (extent_along(&bounds, direction) + base)
            }
            Some(motion @ (Motion::Turn { .. } | Motion::Linkage { .. })) => {
                let Some((free, swing)) = motion.swing() else {
                    return Vec3::ZERO;
                };
                let direction = render::direction(swing);
                let across = render::direction(free);
                direction * (extent_along(&bounds, across) * 0.5 + base)
            }
            None => {
                // The first joint between a part inside and one outside.
                for joint in &design.joints {
                    for pair in &joint.pairs {
                        let parts: Vec<PartId> = pair.roles.iter().map(|r| r.part).collect();
                        let inside = |p: &PartId| assembly.parts.contains(p);
                        let (Some(_), Some(outside)) = (
                            parts.iter().find(|p| inside(p)),
                            parts.iter().find(|p| !inside(p)),
                        ) else {
                            continue;
                        };
                        let Some(step) = pair
                            .insert
                            .iter()
                            .rev()
                            .find(|step| !matches!(step.kind, StepKind::Rotate))
                        else {
                            continue;
                        };
                        let (Some(moving), Some(outside)) =
                            (design.part(step.part), design.part(*outside))
                        else {
                            continue;
                        };
                        let direction = away_from(
                            step_direction(moving, step.axis, step.angle),
                            bounds.centre(),
                            middle(outside),
                        );
                        return direction * (extent_along(&bounds, direction) * 0.5 + base);
                    }
                }
                Vec3::ZERO
            }
        }
    }

    /// The two stages of a node: its assembly's move and its own.
    fn of_node(&self, design: &SolvedDesign, node: &Node) -> (Vec3, Vec3) {
        let of_part = |part: PartId| {
            let assembly = design
                .part(part)
                .and_then(|p| p.assembly)
                .and_then(|a| self.assemblies.get(a.index()).copied())
                .unwrap_or(Vec3::ZERO);
            (
                assembly,
                self.parts.get(part.index()).copied().unwrap_or(Vec3::ZERO),
            )
        };
        match node.item {
            Item::Part(id) => of_part(id),
            Item::Hardware(id) => {
                let Some(hardware) = design.hardware.get(id.index()) else {
                    return (Vec3::ZERO, Vec3::ZERO);
                };
                // The member on the piece goes with the piece; the fixed
                // member with the part it is fixed to: a hinge's `on`, the
                // side a slide's cabinet member is against.
                if node.name == hardware.name {
                    return of_part(hardware.piece);
                }
                let fixed_to = match hardware.data {
                    HardwareData::Hinge { on, .. } | HardwareData::ButtHinge { on, .. } => Some(on),
                    HardwareData::Slide { .. } => hardware.fixed.as_ref().and_then(|member| {
                        design
                            .parts
                            .iter()
                            .filter(|p| {
                                hardware
                                    .assembly
                                    .and_then(|a| design.assemblies.get(a.index()))
                                    .is_none_or(|a| !a.parts.contains(&p.id))
                            })
                            .find(|p| touching(&p.cut_box(), &member.bounds))
                            .map(|p| p.id)
                    }),
                    HardwareData::Pull { .. } => None,
                };
                fixed_to.map_or((Vec3::ZERO, Vec3::ZERO), of_part)
            }
        }
    }
}
