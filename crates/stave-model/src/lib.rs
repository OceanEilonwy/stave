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

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod defs;
pub mod ids;
pub mod instance;
mod resolve;
pub mod solved;
pub mod types;
pub mod value;

pub use defs::Defs;
pub use instance::Model;
pub use resolve::resolve;
