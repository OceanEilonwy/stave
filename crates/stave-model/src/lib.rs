//! The resolved model of a `.stave` design and the solved-design contract.
//!
//! - [`resolve()`] (layer 2): names resolved by the four namespaces of the
//!   grammar, expressions typed and unit-checked, declaration tables
//!   checked. The result is [`Defs`]: generic definitions, before anything
//!   is instantiated.
//! - [`instance`]: instantiates the project into a concrete [`Model`]:
//!   every part, region, joint, piece of hardware and assembly gets an id,
//!   components are expanded once per assembly and arrays once per element.
//! - [`value`]: the exact values expressions evaluate to, and their
//!   arithmetic (exact fractions; never floats).
//! - [`solved`]: [`solved::SolvedDesign`], the contract every later stage
//!   reads. It holds only resolved, typed data.
//!
//! The stages are separate types, so a later stage can't receive unchecked
//! data: [`Defs`] holds syntax-free resolved expressions, [`Model`] holds
//! ids, and a [`solved::SolvedDesign`] holds numbers.
//!
//! # Resolving a changed copy again
//!
//! An editor checks many changed copies of one project. Resolving runs in
//! two phases: first every library (the standard modules and every
//! imported module), which nothing in the project can change, then the
//! project's own module. [`resolve_keeping`] keeps the first phase's
//! result as [`ResolvedLibraries`]; [`resolve_reusing`] starts from it and
//! resolves only the project, giving exactly what [`resolve()`] gives
//! (`tests/reuse.rs`, and `stave-app`'s `tests/reopen.rs` over the whole
//! corpus). With `stave_modules::ModuleSet::reload_with`, which reuses the
//! parse of every unchanged file, this is `stave_app::Prepared::reopen`.
//!
//! Measured on `examples/clothing_cabinet` with one comment added to
//! `cabinet.stave` (release build, mean of 200 runs, 2026-11-18;
//! `cargo test --release -p stave-model --test reuse -- --ignored
//! --nocapture` prints them again):
//!
//! | Step | Full | Reusing |
//! |---|---|---|
//! | load and parse (`load_with` / `reload_with`) | 2.95 ms | 0.53 ms |
//! | resolve (`resolve` / `resolve_reusing`) | 0.73 ms | 0.30 ms |
//!
//! Instantiating (0.04 ms) and solving (1.4 ms) run in full either way;
//! the fit and motion checks (about 24 ms here) are what remains, which
//! `stave_check`'s two tiers address.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod defs;
pub mod ids;
pub mod instance;
pub mod mechanism;
pub mod nonempty;
mod resolve;
pub mod solved;
pub mod suggest;
pub mod trace;
pub mod types;
pub mod value;

pub use defs::Defs;
pub use instance::Model;
pub use resolve::{ResolvedLibraries, resolve, resolve_keeping, resolve_reusing};
