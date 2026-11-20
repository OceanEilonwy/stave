//! Stave's versions: every revision of a project, the suggestions agents
//! make, and the project's main line, kept with jj (Jujutsu) in
//! `.stave/history/` inside the project folder.
//!
//! The project folder stays plain files with no `.jj`: the repository is
//! loaded from `.stave/history/repo` and Stave is its working copy, building
//! a tree from the project's files to record a revision and handing merged
//! files back to be written. Everyone works in a fork and **main** moves only
//! by merges: the window's fork is the folder (a save merges it into main),
//! and each agent's suggestions are lines of revisions with no folder of
//! their own. One lock file guards the step that moves main.
//!
//! Who made each revision is kept in its description's trailers
//! (`Stave-Actor: agent Claude Desktop 3f2a`), and a suggestion's patches in
//! `Stave-Patch:` trailers (base64), one per patch. Each operation carries
//! the same actor as an attribute, which is how undo knows whose it is.
//!
//! No jj type appears in this crate's public API. See the design in
//! `docs/superpowers/specs/2026-11-20-ai-integration-design.md`,
//! "Revisions".

#![forbid(unsafe_code)]
#![expect(
    dead_code,
    reason = "saving, catching up and suggestions, which use these helpers, come next"
)]

mod actor;
mod error;
mod folder;
mod history;
mod paths;
mod store;

use std::fmt;

pub use actor::{Actor, ActorKind};
pub use error::HistoryError;
pub use folder::VersionsStatus;
pub use history::History;
pub use paths::{PathError, ProjectFiles, RelativePath};

/// A revision's id: the hex id of its commit.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RevisionId(String);

impl RevisionId {
    /// Wraps a revision id as [`History`] reported it.
    pub fn new(hex: impl Into<String>) -> Self {
        Self(hex.into())
    }

    /// The id as hex text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RevisionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// A suggestion's id: the end of its bookmark's name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SuggestionId(String);

impl SuggestionId {
    /// Wraps a suggestion id as [`History`] reported it.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The id as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SuggestionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// One revision on main.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    /// Its id.
    pub id: RevisionId,
    /// Who made it. A revision Stave didn't record (from git history) is
    /// the person, named as its author.
    pub actor: Actor,
    /// What it did, in the maker's words.
    pub message: String,
    /// When, as `YYYY-MM-DD HH:MM` in the time zone it was made in.
    pub time: String,
}
