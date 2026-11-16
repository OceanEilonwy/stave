//! Nodes, materials and scene poses.

use glam::{Mat4, Quat, Vec3};
use stave_geom::{Aabb, Extent, Geometry, render};
use stave_model::solved::{
    AssemblyId, HardwareBase, HardwareId, Motion, Part, PartId, PoseValue, SceneId, SceneItem,
    SolvedDesign,
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
            (RenderMotion::Turn { opening, .. }, PoseValue::Open) => *opening,
            (RenderMotion::Travel { travel, .. }, PoseValue::Open) => *travel,
            (RenderMotion::Turn { .. }, PoseValue::Angle(angle)) => render::radians(angle),
            (RenderMotion::Travel { .. }, PoseValue::Travel(length)) => render::mm(length),
            // A value of the other kind is a type error the checker
            // reports; it doesn't move anything.
            (RenderMotion::Turn { .. }, PoseValue::Travel(_))
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
            let geometry = stave_geom::part(part, options.extent);
            let material = Material::Wood(wood(part));
            nodes.push(Node {
                item: Item::Part(part.id),
                name: part.name.clone(),
                geometry,
                material,
                assemblies: assemblies_of(part.assembly),
            });
        }
        for hardware in &design.hardware {
            nodes.push(Node {
                item: Item::Hardware(hardware.id),
                name: hardware.name.clone(),
                geometry: stave_geom::hardware(hardware),
                material: Material::Metal,
                assemblies: assemblies_of(hardware.assembly),
            });
            // The member screwed to the carcass stays put when the assembly
            // the hardware moves moves: it belongs to the assembly outside.
            if let Some(geometry) = stave_geom::hardware_fixed(hardware) {
                let outside = hardware
                    .assembly
                    .and_then(|id| design.assemblies.get(id.index()))
                    .and_then(|assembly| assembly.parent);
                let member = match hardware.base {
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
                });
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
        SceneGraph {
            title: design.project.title.clone(),
            nodes,
            scenes,
            motions,
        }
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
        let amount = |assembly: AssemblyId| {
            scene
                .moves
                .iter()
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
