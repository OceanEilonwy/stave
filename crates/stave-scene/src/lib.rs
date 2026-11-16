//! The render-ready scene of a solved design: one node per part and piece
//! of hardware with its mesh, material and assembly path; the scene poses
//! (motions applied); an orbit camera; ray picking; and a binary glTF
//! export.
//!
//! Everything is in the render frame of [`stave_geom`] (millimetres, `+y`
//! up, `+z` toward the viewer). Exact values are converted once, while the
//! graph is built, through [`stave_geom::render`].
//!
//! Not done yet: a scene's `explode` and `section` are carried
//! ([`SceneEntry::explode`], [`SceneEntry::section`]) but not applied.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod camera;
pub mod export;
mod graph;
mod pick;

pub use camera::{Orbit, Ray};
pub use graph::{BuildOptions, Item, Material, Node, NodeId, Posed, SceneEntry, SceneGraph, Wood};
pub use pick::Hit;
pub use stave_geom::{Aabb, Extent, Geometry};
