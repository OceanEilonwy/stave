//! `stave-web`: the browser viewer. One WASM build shows a project in 3D
//! on WebGPU, both as the MCP App page an AI app shows and as the page the
//! MCP server serves on localhost (`docs/superpowers/specs/
//! 2026-11-20-ai-integration-design.md`, "The viewer").
//!
//! The page hands the viewer the project's text, not a scene: the viewer
//! runs parse, resolve, solve, check and scene itself
//! ([`stave_scene::pipeline`], over [`stave_modules::MemoryFiles`]; the
//! standard library is embedded), and draws with `stave-render` into a
//! canvas. It is read only: nothing here writes a file or asks for a
//! change.
//!
//! What a viewer shows is worked out without the browser, so it is tested
//! natively:
//!
//! - [`Files`]: the project's files as the page sends them, as JSON
//!   (`{"box.stave": "stave 1 …"}`).
//! - [`Project`]: the files opened, and an agent's suggestion as ghosts
//!   over them, tinted by its verdict (it checks clean or it doesn't).
//! - [`View`]: the camera (rotate and zoom), a slider per motion (doors
//!   and drawers open), the exploded view, and picking (a part's name and
//!   what it is joined to).
//!
//! On `wasm32` the functions the page calls are exported with
//! `wasm-bindgen`: `start(canvas)`, `load(folder, files_json,
//! suggestion_json)`, `set_pose(motion_index, amount)`,
//! `set_explode(amount)`, `orbit(dx, dy)`, `zoom(steps)`,
//! `resize(width, height)` and `pick(x, y)`. `tools/build-viewer.sh`
//! builds the one self-contained page, `target/viewer/stave-viewer.html`,
//! whose `window.staveViewer.show({files, suggestion, folder})` the host
//! calls.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod files;
pub mod page;
mod project;
mod view;
#[cfg(target_arch = "wasm32")]
mod web;

pub use files::{Files, FilesError};
pub use project::{Motion, Project, SuggestionVerdict};
pub use view::{Drawing, NoSuchMotion, Picked, View};
