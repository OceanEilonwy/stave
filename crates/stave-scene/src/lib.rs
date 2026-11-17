//! The render-ready scene of a solved design: one node per part and piece
//! of hardware with its mesh, material and assembly path; the scene poses
//! (motions applied); an orbit camera; ray picking; and a binary glTF
//! export.
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
//! Not done yet: a scene's `section` is carried ([`SceneEntry::section`])
//! but not applied.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod camera;
pub mod export;
pub mod focus;
mod graph;
mod pick;

pub use camera::{Orbit, Projection, Ray};
pub use focus::{
    Anchor, GapMark, Label, LineKind, Look, MotionPlay, Note, Overlay, OverlayLine, Plan, Play,
    PlayFrame, StepPlay,
};
pub use graph::{
    BuildOptions, Highlight, Item, Material, MemberFollower, MotionRange, Node, NodeId, Posed,
    SceneEntry, SceneGraph, Section, Wood,
};
pub use pick::Hit;
pub use stave_geom::{Aabb, Extent, Geometry};
