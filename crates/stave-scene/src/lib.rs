//! The render-ready scene of a solved design: one node per part, per
//! member of a piece of hardware and per member of a joint pair (a dowel's
//! pins) with its mesh, material and assembly path; the design's regions
//! as volumes; the scene poses (motions applied); an orbit camera; ray
//! picking (a part and the face hit, the region a ray looks into, the two
//! parts facing each other across a gap); and a binary glTF export.
//!
//! Everything is in the render frame of [`stave_geom`] (millimetres, `+y`
//! up, `+z` toward the viewer). Exact values are converted once, while the
//! graph is built, through [`stave_geom::render`].
//!
//! Exploded views ([`SceneGraph::pose_with`]): every part moves out along
//! the reverse of its joint's last insert step, by any amount from 0 to 1;
//! a scene with `explode: true` is posed fully exploded. A [`Highlight`]
//! is what a viewer tints and draws in red for a problem.
//!
//! For the editor: [`Ghosts`] of a candidate edit over the base scene,
//! with its verdict; the nodes [`SceneGraph::covering`] what the user is
//! working on from the camera, to draw see-through; and where a grip
//! sits ([`SceneGraph::grip_place`]).
//!
//! Not done yet: a scene's `section` is carried ([`SceneEntry::section`])
//! but not applied.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod camera;
mod cover;
pub mod export;
pub mod focus;
mod ghost;
mod graph;
mod grip;
mod pick;
mod space;

pub use camera::{Orbit, Projection, Ray};
pub use cover::Subject;
pub use focus::{
    Anchor, GapMark, Label, LineKind, Look, MotionPlay, Note, Overlay, OverlayLine, Plan, Play,
    PlayFrame, StepPlay,
};
pub use ghost::{Ghost, GhostChange, GhostVerdict, Ghosts};
pub use graph::{
    BuildOptions, Highlight, Item, Material, MemberFollower, MotionRange, Node, NodeId, Posed,
    RegionNode, SceneEntry, SceneGraph, Section, Wood,
};
pub use grip::{FaceOf, GripFace, GripHandle, GripOf, GripPlace, Nudged};
pub use pick::{Hit, PartFace};
pub use space::{Facing, RegionHit, Target};
pub use stave_geom::{Aabb, Extent, Geometry};
