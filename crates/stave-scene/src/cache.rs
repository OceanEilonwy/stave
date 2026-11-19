//! Part meshes kept from one build of a scene graph to the next.
//!
//! Making a part's cuts in its mesh (the booleans of [`stave_geom::cut_part_local`])
//! is the costly part of building a scene graph, and an edit usually
//! changes few parts' cuts: moving a shelf changes the cuts in the parts it
//! is joined to, and every other part keeps its cuts. So
//! the mesh of each part is kept, in the part's own frame, under a key of
//! everything the mesh is made from: the part's [`Solid`] (its blocks,
//! holes, round tenons and profile, already measured in the part's own
//! frame from its corner), its cut size, its grain and its
//! [`Handedness`]. Where the part is, its name and its id are not in the
//! key, so a part that only moved keeps its mesh, and parts alike in all
//! of those (four legs, two sides) share one.
//!
//! The key holds exact values ([`stave_lang::units::Length`]), so a mesh
//! is reused only for exactly the same cuts, and the mesh drawn is the
//! one a fresh build would make.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use stave_geom::solid::Solid;
use stave_geom::{Geometry, Handedness};
use stave_model::solved::{Part, Size3};
use stave_schema::words::Grain;

/// Everything a part's own-frame cut mesh is made from.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct PartKey {
    solid: Solid,
    cut: Size3,
    grain: Grain,
    handedness: Handedness,
}

/// How a build used the cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CacheUse {
    /// Part meshes made afresh.
    pub made: usize,
    /// Part meshes taken from an earlier build, or shared with another
    /// part of the same build.
    pub reused: usize,
    /// Distinct part meshes the cache holds after the build.
    pub held: usize,
}

#[derive(Debug, Default)]
struct Entries {
    /// The meshes of the last build, by what they are made from.
    meshes: HashMap<PartKey, Arc<Geometry>>,
    /// How the last build used them.
    last: CacheUse,
}

/// Part meshes kept between builds of a scene graph
/// ([`crate::SceneGraph::build_with`]). Cloning it shares the same store,
/// so a viewer can keep one and hand it to each check's worker thread.
///
/// After each build it holds exactly the meshes that build used: a mesh
/// no part uses any more is let go.
#[derive(Debug, Clone, Default)]
pub struct MeshCache {
    entries: Arc<Mutex<Entries>>,
}

impl MeshCache {
    /// An empty cache.
    pub fn new() -> MeshCache {
        MeshCache::default()
    }

    /// How the last build used the cache.
    pub fn last_use(&self) -> CacheUse {
        self.lock().last
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Entries> {
        // A build that panicked leaves whole entries behind (each is
        // inserted complete), so they are still good.
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Starts a build.
    pub(crate) fn build(&self) -> CacheBuild<'_> {
        CacheBuild {
            cache: self,
            used: HashMap::new(),
            usage: CacheUse::default(),
        }
    }
}

/// One build's use of a [`MeshCache`]: the meshes it used so far.
pub(crate) struct CacheBuild<'c> {
    cache: &'c MeshCache,
    used: HashMap<PartKey, Arc<Geometry>>,
    usage: CacheUse,
}

impl CacheBuild<'_> {
    /// A part's cut mesh in its own frame, made only when neither this
    /// build nor the last has one for the same cuts.
    pub(crate) fn cut_part(&mut self, part: &Part, solid: Solid) -> Arc<Geometry> {
        let key = PartKey {
            solid,
            cut: part.cut,
            grain: part.grain,
            handedness: stave_geom::handedness(part),
        };
        if let Some(mesh) = self.used.get(&key) {
            self.usage.reused = self.usage.reused.saturating_add(1);
            return Arc::clone(mesh);
        }
        let kept = self.cache.lock().meshes.get(&key).cloned();
        let mesh = match kept {
            Some(mesh) => {
                self.usage.reused = self.usage.reused.saturating_add(1);
                mesh
            }
            None => {
                self.usage.made = self.usage.made.saturating_add(1);
                Arc::new(stave_geom::cut_part_local(part, &key.solid))
            }
        };
        self.used.insert(key, Arc::clone(&mesh));
        mesh
    }

    /// Ends the build: the cache keeps what it used and lets the rest go.
    pub(crate) fn finish(self) {
        let mut entries = self.cache.lock();
        entries.last = CacheUse {
            held: self.used.len(),
            ..self.usage
        };
        entries.meshes = self.used;
    }
}
