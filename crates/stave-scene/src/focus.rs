//! What focus mode draws besides the posed scene: how each node looks
//! (the item in focus as it is, the part receiving it see-through, the
//! rest faded to ghosts), and an overlay of lines and numbered labels (the
//! dotted path of a point as an assembly moves, the dashed outline of
//! where a part ends up, the numbers of a hinge's members).

use glam::{Mat4, Vec3};
use stave_geom::Aabb;
use stave_geom::render;
use stave_geom::sequence::{Moved, Traced};
use stave_lang::placement::Direction;
use stave_model::solved::{AssemblyId, PartId, WorldBox};

use crate::graph::{Item, MotionRange, NodeId, Posed, SceneEntry, SceneGraph, Section};

/// How a node is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Look {
    /// As usual.
    #[default]
    Normal,
    /// Faded to a pale ghost: there, but not what this is about.
    Ghost,
    /// Dimmed, still reading as wood: the rest of the design while
    /// something in it is chosen.
    Dim,
    /// See-through, so what is inside it shows (a rung in its socket, a
    /// tenon in its mortise), with its cut faces showing through.
    XRay,
}

/// What kind of line an overlay line is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LineKind {
    /// The path a point follows as something moves, still to come
    /// (dotted).
    Path,
    /// The part of the path already travelled (solid).
    PathTravelled,
    /// Where a part ends up when its joint is together (dashed).
    Home,
    /// Between a moving part and the fixed part it comes closest to.
    Gap,
}

/// One straight piece of an overlay line, in the render frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OverlayLine {
    /// Where it starts.
    pub from: Vec3,
    /// Where it ends.
    pub to: Vec3,
    /// What it shows.
    pub kind: LineKind,
}

/// A number drawn at a point (a member of a piece of hardware).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Label {
    /// Where, in the render frame.
    pub at: Vec3,
    /// The number, from 1.
    pub number: u8,
}

/// A short text drawn at a point: a measurement ("3.5 mm"). Only digits,
/// `.`, `°`, spaces and the letters of `mm` are drawn.
#[derive(Debug, Clone, PartialEq)]
pub struct Note {
    /// Where, in the render frame.
    pub at: Vec3,
    /// The text.
    pub text: String,
}

/// Lines and labels drawn over the view, on top of everything.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Overlay {
    /// The line pieces (already broken into dots or dashes).
    pub lines: Vec<OverlayLine>,
    /// The labels.
    pub labels: Vec<Label>,
    /// The notes.
    pub notes: Vec<Note>,
    /// Small dots with a tiny caption: the ends of a path ("0°", "110°").
    pub markers: Vec<Note>,
}

impl Overlay {
    /// Adds a polyline broken into pieces `dash` long with `gap` between
    /// them (millimetres), the pattern carried on across joined pieces.
    pub fn add_broken(&mut self, segments: &[[Vec3; 2]], kind: LineKind, dash: f32, gap: f32) {
        let period = (dash + gap).max(1e-3);
        // Distance along the joined line where each piece starts.
        let mut start = 0.0_f32;
        let mut last_end: Option<Vec3> = None;
        for [from, to] in segments {
            if last_end.is_none_or(|end| end.distance(*from) > 1e-3) {
                start = 0.0;
            }
            last_end = Some(*to);
            let length = from.distance(*to);
            if !length.is_finite() || length <= 1e-6 {
                continue;
            }
            let end = start + length;
            let direction = (*to - *from) / length;
            // Dash k covers [k * period, k * period + dash].
            let first = (start / period).floor();
            let mut k = first;
            while k * period < end && k - first < 100_000.0 {
                let low = (k * period).max(start);
                let high = (k * period + dash).min(end);
                if high > low {
                    self.lines.push(OverlayLine {
                        from: *from + direction * (low - start),
                        to: *from + direction * (high - start),
                        kind,
                    });
                }
                k += 1.0;
            }
            start = end;
        }
    }

    /// Adds a path through points: solid up to point `travelled`, dotted
    /// (closely, so a short curve still reads) after it.
    pub fn add_path(&mut self, points: &[Vec3], travelled: usize) {
        let segments = |points: &[Vec3]| -> Vec<[Vec3; 2]> {
            points
                .windows(2)
                .filter_map(|pair| match pair {
                    [a, b] => Some([*a, *b]),
                    _ => None,
                })
                .collect()
        };
        let split = travelled.min(points.len().saturating_sub(1));
        let (done, ahead) = (
            points.get(..=split).unwrap_or(&[]),
            points.get(split..).unwrap_or(&[]),
        );
        for [from, to] in segments(done) {
            self.lines.push(OverlayLine {
                from,
                to,
                kind: LineKind::PathTravelled,
            });
        }
        self.add_broken(&segments(ahead), LineKind::Path, 0.7, 0.9);
    }
}

impl SceneGraph {
    /// Every node that draws an item (a hinge is its cup, its plate and
    /// arm, and its two links).
    pub fn nodes_of(&self, item: Item) -> Vec<NodeId> {
        self.node_ids()
            .filter(|id| self.node(*id).is_some_and(|node| node.item == item))
            .collect()
    }

    /// The looks of every node: `focus` and `context` as they are, `xray`
    /// see-through, the rest ghosts when `fade` (else as they are).
    pub fn looks(&self, focus: &[NodeId], xray: &[NodeId], fade: bool) -> Vec<Look> {
        self.node_ids()
            .map(|id| {
                if xray.contains(&id) {
                    Look::XRay
                } else if focus.contains(&id) || !fade {
                    Look::Normal
                } else {
                    Look::Ghost
                }
            })
            .collect()
    }

    /// Every node that moves with a part: its own, and the members it
    /// carries (a joint's pins, a hinge's cup).
    pub fn carried_nodes(&self, part: PartId) -> Vec<NodeId> {
        self.node_ids()
            .filter(|id| {
                self.node(*id)
                    .is_some_and(|node| node.carried_by() == Some(part))
            })
            .collect()
    }

    /// Moves a part's nodes, and the members it carries, by a
    /// render-frame transform, on top of where they are posed.
    pub fn move_part(&self, posed: &mut Posed, part: PartId, transform: Mat4) {
        for id in self.carried_nodes(part) {
            posed.move_node(id, transform);
        }
    }

    /// Where a point carried by an assembly goes over its whole motion,
    /// from closed to fully open, at `samples + 1` evenly spaced amounts
    /// (render frame; the point given at rest). Empty when it doesn't
    /// move.
    pub fn path_of(&self, assembly: AssemblyId, point: Vec3, samples: u16) -> Vec<Vec3> {
        let Some(range) = self.motion_range(assembly) else {
            return Vec::new();
        };
        let count = samples.max(1);
        (0..=count)
            .filter_map(|k| {
                let amount = range.full() * f32::from(k) / f32::from(count);
                Some(
                    self.motion_transform(assembly, amount)?
                        .transform_point3(point),
                )
            })
            .collect()
    }

    /// The box to frame a joint in: where the moving parts meet the
    /// receiving ones at rest, grown so the parts around it show (by 1.2
    /// times its largest size, at least 90mm), but no bigger than the
    /// parts themselves.
    pub fn joint_zone(&self, moving: &[PartId], receiving: &[PartId]) -> Option<Aabb> {
        let rest = self.rest();
        let bounds = |parts: &[PartId]| {
            let nodes: Vec<NodeId> = parts
                .iter()
                .flat_map(|part| self.nodes_of(Item::Part(*part)))
                .collect();
            self.bounds_of(&rest, &nodes)
        };
        let (a, b) = (bounds(moving)?, bounds(receiving)?);
        let all = a.union(b);
        let low = a.min.max(b.min);
        let high = a.max.min(b.max);
        if (high - low).min_element() < -1.0 {
            return Some(all);
        }
        let grow = ((high - low).max_element() * 1.2).max(90.0);
        Some(Aabb {
            min: (low - Vec3::splat(grow)).max(all.min),
            max: (high + Vec3::splat(grow)).min(all.max),
        })
    }

    /// The box to frame a joint's members in (a dowel's pins, a screwed
    /// joint's screws): around them at rest, grown by 60mm or a quarter
    /// of their largest size, whichever is more, so the ends of the parts
    /// they join show around them and they stay in view as they go in.
    pub fn members_zone(&self, nodes: &[NodeId]) -> Option<Aabb> {
        let bounds = self.bounds_of(&self.rest(), nodes)?;
        let grow = Vec3::splat((bounds.size().max_element() * 0.25).max(60.0));
        Some(Aabb {
            min: bounds.min - grow,
            max: bounds.max + grow,
        })
    }

    /// The edges of a part's nodes where it is at rest, for a dashed
    /// outline of where it ends up.
    pub fn rest_edges(&self, part: PartId) -> Vec<[Vec3; 2]> {
        self.nodes_of(Item::Part(part))
            .into_iter()
            .filter_map(|id| self.node(id))
            .flat_map(|node| {
                let rest = node.rest;
                node.local.edges.segments.iter().map(move |[a, b]| {
                    [
                        rest.transform_point3(Vec3::from(*a)),
                        rest.transform_point3(Vec3::from(*b)),
                    ]
                })
            })
            .collect()
    }
}

/// How focus mode moves its item, ready to pose.
#[derive(Debug, Clone, PartialEq)]
pub enum Play {
    /// A joint pair's `insert:` steps, traced.
    Steps(StepPlay),
    /// An assembly's motion, driven by a piece of hardware.
    Motion(MotionPlay),
    /// Nothing moves.
    Still,
    /// Nothing moves, but its members are numbered (a pull on its door).
    Fixed(Vec<(u8, Anchor)>),
}

/// A joint pair's steps: its parts, each step's move (from
/// [`stave_geom::sequence::trace`]) and each step as written.
#[derive(Debug, Clone, PartialEq)]
pub struct StepPlay {
    /// The pair's parts, in the order the trace indexes them.
    pub members: Vec<PartId>,
    /// Each step, in order.
    pub traced: Vec<Traced>,
    /// Each step as written with its parts named:
    /// `push(rung[front], along: length)`.
    pub texts: Vec<String>,
}

/// Where a numbered label goes.
#[derive(Debug, Clone, PartialEq)]
pub enum Anchor {
    /// These nodes where they are posed: their middle, or (with a
    /// direction) a fifth of the way in from their end that way.
    Nodes(Vec<NodeId>, Option<Direction>),
    /// A world box that doesn't move: its middle, or (with a direction)
    /// a fifth of the way in from its end that way, so labels of boxes
    /// one above the other (a hinge's plate under its arm) don't cover
    /// each other.
    Box(WorldBox, Option<Direction>),
}

/// An assembly's motion with what focus mode draws over it.
#[derive(Debug, Clone, PartialEq)]
pub struct MotionPlay {
    /// The assembly.
    pub assembly: AssemblyId,
    /// How far it moves.
    pub range: MotionRange,
    /// Where the dotted point goes, closed to open (render frame).
    pub path: Vec<Vec3>,
    /// Numbered labels.
    pub anchors: Vec<(u8, Anchor)>,
    /// Where it comes closest to a fixed part, marked with the gap.
    pub gap: Option<GapMark>,
}

/// The closest approach of a motion, drawn as a line from the point on
/// the fixed part to the point on the moving part (wherever it is), with
/// the smallest gap written by the fixed point.
#[derive(Debug, Clone, PartialEq)]
pub struct GapMark {
    /// The point on the fixed part (render frame).
    pub fixed: Vec3,
    /// The point on the moving part where it comes closest (render
    /// frame): exactly the gap from `fixed`.
    pub moving: Vec3,
    /// Where along the motion it comes closest (radians or millimetres).
    pub at: f32,
    /// "closest at 108°: 3.5 mm".
    pub text: String,
}

impl GapMark {
    /// The mark for a closest approach of an assembly's motion found by
    /// [`stave_geom::clearance`], `gap` its size in words ("3.5 mm");
    /// `None` when it has no points.
    pub fn new(
        graph: &SceneGraph,
        assembly: AssemblyId,
        closest: &stave_geom::clearance::Closest,
        gap: &str,
    ) -> Option<GapMark> {
        let (on_moving, on_fixed) = closest.points?;
        let range = graph.motion_range(assembly)?;
        // Radians for a turn; nanometres along a slide, millimetres here.
        let at = render::narrow(if range.turns() {
            closest.at
        } else {
            closest.at * 1e-6
        });
        Some(GapMark {
            fixed: render::point_of_nm(on_fixed),
            moving: render::point_of_nm(on_moving),
            at,
            text: format!("closest at {}: {gap}", amount_text(range, at)),
        })
    }

    /// Whether a motion at `amount` is where this gap is measured: within
    /// 2° of a turn, 2mm of a slide.
    pub fn is_at(&self, range: MotionRange, amount: f32) -> bool {
        let tolerance = if range.turns() {
            2.0_f32.to_radians()
        } else {
            2.0
        };
        (amount - self.at).abs() <= tolerance
    }
}

/// One posed frame of a [`Play`].
#[derive(Debug, Clone, PartialEq)]
pub struct PlayFrame {
    /// Where everything is.
    pub posed: Posed,
    /// The lines and labels.
    pub overlay: Overlay,
    /// "Step 1 of 1 · push(rung[front], along: length)".
    pub step: Option<String>,
    /// "rung[front] is 6 mm from home", "open 45°".
    pub status: String,
}

/// A count as an `f64`.
fn f64_of(count: usize) -> f64 {
    f64::from(u32::try_from(count).unwrap_or(u32::MAX))
}

/// The whole number at or below a non-negative amount, as an index.
fn index_of(value: f64) -> usize {
    render::integer_from_f64(value.max(0.0).floor())
        .and_then(|v| usize::try_from(v).ok())
        .unwrap_or(0)
}

/// A value rounded to a whole number, as text.
fn whole(value: f64) -> String {
    render::integer_from_f64(value.round())
        .unwrap_or(0)
        .to_string()
}

/// An amount of a motion as text: `45°` for a hinge (radians given),
/// `250 mm` for a slide.
pub fn amount_text(range: MotionRange, amount: f32) -> String {
    if range.turns() {
        format!("{}°", whole(f64::from(amount).to_degrees()))
    } else {
        format!("{} mm", whole(f64::from(amount)))
    }
}

/// The poses for small pictures beside a motion: closed, 45° (or half
/// way for a slide) and fully open, as scrub amounts (0 to 1) with their
/// captions.
pub fn key_poses(range: MotionRange) -> Vec<(f32, String)> {
    let amounts = if range.turns() {
        [0.0, 45.0_f32.to_radians().min(range.full()), range.full()]
    } else {
        [0.0, range.full() * 0.5, range.full()]
    };
    amounts
        .into_iter()
        .map(|amount| {
            let progress = if range.full() > 0.0 {
                amount / range.full()
            } else {
                0.0
            };
            (progress, amount_text(range, amount))
        })
        .collect()
}

impl Play {
    /// The design posed at `progress`, 0 to 1 (apart to together for a
    /// joint, closed to open for a motion), in a scene.
    pub fn frame(&self, graph: &SceneGraph, scene: &SceneEntry, progress: f32) -> PlayFrame {
        let progress = progress.clamp(0.0, 1.0);
        match self {
            Play::Steps(play) => play.frame(graph, scene, progress),
            Play::Motion(play) => play.frame(graph, scene, progress),
            Play::Still => PlayFrame {
                posed: graph.pose_with(scene, &[], 0.0),
                overlay: Overlay::default(),
                step: None,
                status: "it doesn't move".to_owned(),
            },
            Play::Fixed(anchors) => {
                let posed = graph.pose_with(scene, &[], 0.0);
                let mut overlay = Overlay::default();
                label_anchors(graph, &posed, anchors, &mut overlay);
                PlayFrame {
                    posed,
                    overlay,
                    step: None,
                    status: "fixed in place".to_owned(),
                }
            }
        }
    }
}

impl StepPlay {
    fn frame(&self, graph: &SceneGraph, scene: &SceneEntry, progress: f32) -> PlayFrame {
        let mut posed = graph.pose_with(scene, &[], 0.0);
        let mut overlay = Overlay::default();
        let count = self.traced.len();
        if count == 0 {
            return PlayFrame {
                posed,
                overlay,
                step: None,
                status: String::new(),
            };
        }
        let along = f64::from(progress) * f64_of(count);
        let current = index_of(along).min(count - 1);
        let within = (along - f64_of(current)).clamp(0.0, 1.0);
        // Steps not started yet leave their parts apart: later steps
        // first, so a part moved by several starts where the earliest of
        // them starts.
        let mut placements: Vec<Option<glam::DMat4>> = vec![None; self.members.len()];
        for step in self.traced.iter().skip(current + 1).rev() {
            if let Some(slot) = placements.get_mut(step.member) {
                *slot = Some(step.placement(0.0));
            }
        }
        let step = self.traced.get(current);
        if let Some(step) = step
            && let Some(slot) = placements.get_mut(step.member)
        {
            *slot = Some(step.placement(within));
        }
        for (member, placement) in self.members.iter().zip(&placements) {
            if let Some(matrix) = placement {
                graph.move_part(&mut posed, *member, render::transform(*matrix));
            }
        }
        let moving = step.and_then(|s| self.members.get(s.member)).copied();
        if let Some(part) = moving
            && progress < 1.0
        {
            overlay.add_broken(&graph.rest_edges(part), LineKind::Home, 2.0, 1.5);
        }
        let step_line = self
            .texts
            .get(current)
            .map(|text| format!("Step {} of {count} · {text}", current + 1));
        let status = match (step, moving) {
            (Some(step), Some(part)) => {
                let name = graph
                    .nodes_of(Item::Part(part))
                    .first()
                    .and_then(|id| graph.node(*id))
                    .map_or("it", |node| node.name.as_str());
                let left = 1.0 - within;
                if progress >= 1.0 {
                    format!("together: {name} is home")
                } else {
                    match step.moved {
                        Moved::Slide(vector) => {
                            let later: f64 = self
                                .traced
                                .iter()
                                .skip(current + 1)
                                .filter(|s| s.member == step.member)
                                .map(|s| match s.moved {
                                    Moved::Slide(v) => v.length(),
                                    Moved::Turn { .. } => 0.0,
                                })
                                .sum();
                            let nm = vector.length() * left + later;
                            format!("{name} is {} mm from home", whole(nm / 1e6))
                        }
                        Moved::Turn { angle, .. } => format!(
                            "{name} has {}° still to turn",
                            whole((angle.abs() * left).to_degrees())
                        ),
                    }
                }
            }
            _ => String::new(),
        };
        PlayFrame {
            posed,
            overlay,
            step: step_line,
            status,
        }
    }
}

impl MotionPlay {
    /// Where along the motion (0 to 1) the closest approach is, when it
    /// is marked.
    pub fn closest_progress(&self) -> Option<f32> {
        let gap = self.gap.as_ref()?;
        (self.range.full() > 0.0).then(|| gap.at / self.range.full())
    }

    fn frame(&self, graph: &SceneGraph, scene: &SceneEntry, progress: f32) -> PlayFrame {
        let amount = self.range.full() * progress;
        let posed = graph.pose_with(scene, &[(self.assembly, amount)], 0.0);
        let mut overlay = Overlay::default();
        // The path: travelled solid, still to come dotted, its ends marked.
        let steps = self.path.len().saturating_sub(1);
        let travelled = index_of(f64::from(progress) * f64_of(steps));
        overlay.add_path(&self.path, travelled);
        for (point, amount) in [
            (self.path.first(), 0.0),
            (self.path.last(), self.range.full()),
        ] {
            if let Some(point) = point {
                overlay.markers.push(Note {
                    at: *point,
                    text: amount_text(self.range, amount),
                });
            }
        }
        // The gap is written where it is found; its dimension line only
        // where it is true, at the pose it is measured in.
        if let Some(gap) = &self.gap {
            if gap.is_at(self.range, amount) {
                overlay.lines.push(OverlayLine {
                    from: gap.fixed,
                    to: gap.moving,
                    kind: LineKind::Gap,
                });
            }
            overlay.notes.push(Note {
                at: gap.fixed,
                text: gap.text.clone(),
            });
        }
        label_anchors(graph, &posed, &self.anchors, &mut overlay);
        let closed = if self.range.turns() {
            amount <= 1e-4
        } else {
            amount <= 1e-3
        };
        let status = if closed {
            if self.range.turns() { "closed" } else { "shut" }.to_owned()
        } else if self.range.turns() {
            format!("open {}", amount_text(self.range, amount))
        } else {
            format!("out {}", amount_text(self.range, amount))
        };
        PlayFrame {
            posed,
            overlay,
            step: None,
            status,
        }
    }
}

/// Numbers each anchor's member where it is posed.
fn label_anchors(
    graph: &SceneGraph,
    posed: &Posed,
    anchors: &[(u8, Anchor)],
    overlay: &mut Overlay,
) {
    for (number, anchor) in anchors {
        let at = match anchor {
            Anchor::Box(region, toward) => {
                let (low, high) = (render::point(region.min()), render::point(region.max()));
                let middle = (low + high) * 0.5;
                Some(toward.map_or(middle, |direction| {
                    let along = render::direction(direction);
                    middle + along * ((high - low).abs().dot(along.abs()) * 0.3)
                }))
            }
            Anchor::Nodes(nodes, toward) => graph.bounds_of(posed, nodes).map(|b| {
                toward.map_or(b.centre(), |direction| {
                    let along = render::direction(direction);
                    b.centre() + along * (b.size().dot(along.abs()) * 0.3)
                })
            }),
        };
        if let Some(at) = at {
            overlay.labels.push(Label {
                at,
                number: *number,
            });
        }
    }
}

/// The point whose path focus mode dots: for a hinge (`swing` its free
/// side and swing direction) the door's front edge on its hinge side, at
/// the height of `level`; for a slide (`toward` its direction) the middle
/// of the drawer's front. `door` is the box around the moving parts at
/// rest, `level` the box around the hardware.
pub fn edge_point(
    door: Aabb,
    level: Aabb,
    swing: Option<(Direction, Direction)>,
    toward: Option<Direction>,
) -> Vec3 {
    match (swing, toward) {
        (Some((free, swing)), _) => {
            let free = render::direction(free);
            let swing = render::direction(swing);
            let mut point = level.centre();
            for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
                // Against the free side (the hinge edge), and toward the
                // side it swings to (the front face).
                let wanted = if free.dot(axis).abs() > 0.5 {
                    Some(-free.dot(axis))
                } else if swing.dot(axis).abs() > 0.5 {
                    Some(swing.dot(axis))
                } else {
                    None
                };
                if let Some(sign) = wanted {
                    let value = if sign > 0.0 {
                        door.max.dot(axis)
                    } else {
                        door.min.dot(axis)
                    };
                    point += axis * (value - point.dot(axis));
                }
            }
            point
        }
        (None, toward) => {
            // On the front face, level with and beside the slide, so the
            // path runs past it.
            let toward = toward.map_or(Vec3::Z, render::direction);
            let beside = level.centre().clamp(door.min, door.max);
            let front = door.centre() + toward * (door.size().dot(toward.abs()) * 0.5);
            beside * (Vec3::ONE - toward.abs()) + front * toward.abs()
        }
    }
}

/// The direction to look at a hinge from in 3D, as an orbit's yaw and
/// pitch (radians): from inside the cabinet, where its plate is screwed,
/// and a little in front and above, so the plate, arm, links and cup all
/// show. `free` is the direction from the hinge edge toward the middle of
/// the door.
pub fn hinge_view(free: Direction) -> (f32, f32) {
    let toward = render::direction(free) + Vec3::Z * 0.45;
    (toward.x.atan2(toward.z), 25.0_f32.to_radians())
}

/// A flat view from above, cut just over the hardware's highest point and
/// framed on it and on the path its door's edge follows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plan {
    /// The section: what is above is cut away.
    pub section: Section,
    /// What to frame.
    pub bounds: Aabb,
    /// The orbit's yaw (radians): 0 has the front at the bottom of the
    /// picture; a slide's view turns so the drawer travels across it.
    pub yaw: f32,
}

impl Plan {
    /// The plan of a hinge drawn by `nodes` whose door's edge follows
    /// `path`, cut just above `cut_above` (a cup hinge's arm, so the cut
    /// passes through its plate and cup and shows the arm, the links and
    /// the boss) or else above the whole hinge.
    pub fn of(
        graph: &SceneGraph,
        nodes: &[NodeId],
        path: &[Vec3],
        cut_above: Option<WorldBox>,
    ) -> Option<Plan> {
        let hardware = graph.bounds_of(&graph.rest(), nodes)?;
        let top = cut_above.map_or(hardware.max.y, |region| render::point(region.max()).y);
        let bounds = Aabb::around(path.iter().copied()).map_or(hardware, |p| p.union(hardware));
        // Room around it for the door as it swings.
        let margin = Vec3::new(30.0, 0.0, 30.0);
        let bounds = Aabb {
            min: bounds.min - margin,
            max: bounds.max + margin,
        };
        Some(Plan {
            section: Section {
                normal: Vec3::Y,
                offset: top + 0.5,
            },
            bounds,
            yaw: 0.0,
        })
    }

    /// The plan of a joint whose part turns about the up axis (the
    /// chidori's lock): looking down along that axis, cut through the
    /// middle of where the turning part passes through the receiving ones,
    /// so its section and the slots it turns into show together, framed
    /// close on them.
    pub fn joint(graph: &SceneGraph, turning: PartId, receiving: &[PartId]) -> Option<Plan> {
        let rest = graph.rest();
        let a = graph.bounds_of(&rest, &graph.nodes_of(Item::Part(turning)))?;
        let nodes: Vec<NodeId> = receiving
            .iter()
            .flat_map(|part| graph.nodes_of(Item::Part(*part)))
            .collect();
        let b = graph.bounds_of(&rest, &nodes)?;
        let low = a.min.max(b.min);
        let high = a.max.min(b.max);
        if (high - low).min_element() < 0.0 {
            return None;
        }
        let grow = Vec3::splat((high - low).max_element().max(20.0));
        Some(Plan {
            section: Section {
                normal: Vec3::Y,
                offset: (low.y + high.y) * 0.5,
            },
            bounds: Aabb {
                min: low - grow,
                max: high + grow,
            },
            yaw: 0.0,
        })
    }

    /// The plan of a slide drawn by `nodes`, travelling `toward` up to
    /// `travel` millimetres: cut through the middle of its height, framed
    /// across from the cabinet side to the drawer side and along from a
    /// little behind its front to 180mm out (or its whole travel, if
    /// shorter), close enough that its members read, turned so the travel
    /// runs across the picture.
    pub fn slide(
        graph: &SceneGraph,
        nodes: &[NodeId],
        toward: Direction,
        travel: f32,
    ) -> Option<Plan> {
        let slide = graph.bounds_of(&graph.rest(), nodes)?;
        let along = render::direction(toward);
        let front = if along.dot(Vec3::ONE) >= 0.0 {
            slide.max.dot(along.abs())
        } else {
            slide.min.dot(along.abs())
        };
        let behind = front - along.dot(Vec3::ONE).signum() * 120.0;
        let reach = front + along.dot(Vec3::ONE).signum() * travel.min(180.0);
        let mut min = slide.min - Vec3::splat(30.0);
        let mut max = slide.max + Vec3::splat(30.0);
        let axis = along.abs();
        let (low, high) = (behind.min(reach), behind.max(reach));
        min = min * (Vec3::ONE - axis) + axis * low;
        max = max * (Vec3::ONE - axis) + axis * high;
        Some(Plan {
            section: Section {
                normal: Vec3::Y,
                offset: slide.centre().y,
            },
            bounds: Aabb { min, max },
            yaw: if axis.z > 0.5 {
                std::f32::consts::FRAC_PI_2
            } else {
                0.0
            },
        })
    }
}

/// The box around some nodes at rest, grown by its own largest size, so
/// what they are fixed to shows around them (a pull on its door).
pub fn around(graph: &SceneGraph, nodes: &[NodeId]) -> Option<Aabb> {
    let bounds = graph.bounds_of(&graph.rest(), nodes)?;
    let grow = Vec3::splat(bounds.size().max_element());
    Some(Aabb {
        min: bounds.min - grow,
        max: bounds.max + grow,
    })
}

/// What to frame a piece of hardware in, in 3D: its nodes wherever the
/// motion takes them, the dotted path and the gap mark, so the camera
/// keeps everything in view through the whole motion.
pub fn motion_zone(
    graph: &SceneGraph,
    nodes: &[NodeId],
    assembly: AssemblyId,
    path: &[Vec3],
    gap: Option<&GapMark>,
) -> Option<Aabb> {
    let scene = graph.scenes.first()?;
    let swept = graph.swept_bounds(scene, nodes, assembly, 16);
    let mut points: Vec<Vec3> = path.to_vec();
    if let Some(gap) = gap {
        points.push(gap.fixed);
        if let Some(range) = graph.motion_range(assembly) {
            for k in 0..=16_u16 {
                if let Some(transform) =
                    graph.motion_transform(assembly, range.full() * f32::from(k) / 16.0)
                {
                    points.push(transform.transform_point3(gap.moving));
                }
            }
        }
    }
    let zone = match (swept, Aabb::around(points)) {
        (Some(a), Some(b)) => a.union(b),
        (a, b) => a.or(b)?,
    };
    Some(Aabb {
        min: zone.min - Vec3::splat(10.0),
        max: zone.max + Vec3::splat(10.0),
    })
}

impl SceneGraph {
    /// The box around some nodes over an assembly's whole motion, at
    /// `samples + 1` amounts from closed to fully open, in a scene.
    pub fn swept_bounds(
        &self,
        scene: &SceneEntry,
        nodes: &[NodeId],
        assembly: AssemblyId,
        samples: u16,
    ) -> Option<Aabb> {
        let range = self.motion_range(assembly)?;
        let count = samples.max(1);
        (0..=count)
            .filter_map(|k| {
                let amount = range.full() * f32::from(k) / f32::from(count);
                let posed = self.pose_with(scene, &[(assembly, amount)], 0.0);
                self.bounds_of(&posed, nodes)
            })
            .reduce(Aabb::union)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_broken_line_keeps_its_pattern_across_joined_pieces() {
        let mut overlay = Overlay::default();
        // Two joined pieces 3mm each; dashes 2mm with 1mm gaps.
        overlay.add_broken(
            &[
                [Vec3::ZERO, Vec3::new(3.0, 0.0, 0.0)],
                [Vec3::new(3.0, 0.0, 0.0), Vec3::new(6.0, 0.0, 0.0)],
            ],
            LineKind::Home,
            2.0,
            1.0,
        );
        let dashes: Vec<(f32, f32)> = overlay.lines.iter().map(|l| (l.from.x, l.to.x)).collect();
        assert_eq!(dashes, [(0.0, 2.0), (3.0, 3.0 + 2.0)]);
        assert!(overlay.lines.iter().all(|l| l.kind == LineKind::Home));
    }
}
