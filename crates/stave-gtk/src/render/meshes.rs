//! The scene's meshes on the GPU: one vertex, one index and one outline
//! buffer shared by every node, each node drawing its own range of them.
//!
//! A pass binds each buffer once and draws node after node from it, so a
//! design of hundreds of parts costs a few binds a pass, not thousands.
//!
//! A node's mesh is in its own frame, so nodes alike in shape (four legs,
//! two sides) share one mesh, held once and drawn from the same range;
//! their node transforms place each copy.
//!
//! Meshes are kept by their content. A new check result usually changes
//! few parts (often none: a renamed part, an edited comment), so an upload
//! writes only the meshes the buffers don't hold yet, and nothing at all
//! when every node's mesh is one already there. The buffers grow by
//! doubling; when the meshes no longer fit, the ones still in use are
//! written afresh from the start, which also drops the ones no longer used.

use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasher as _, Hasher as _, RandomState};
use std::ops::Range;
use std::sync::Arc;

use anyhow::{Context as _, Result, bail};
use stave_geom::{Geometry, Vertex};
use stave_scene::Node;

/// The fewest elements a shared buffer is made for.
const MINIMUM_CAPACITY: u64 = 4096;

/// Where a node's mesh lies in the shared buffers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct MeshRange {
    /// Its first index in the index buffer; the indices already point at
    /// the mesh's vertices where they lie in the vertex buffer.
    first_index: u32,
    /// How many indices it has (three per triangle).
    index_count: u32,
    /// Its first point in the outline buffer.
    first_edge_point: u32,
    /// How many outline points it has (two per segment).
    edge_point_count: u32,
}

impl MeshRange {
    /// Its indices in the shared index buffer.
    pub fn indices(self) -> Range<u32> {
        self.first_index..self.first_index.saturating_add(self.index_count)
    }

    /// Its outline's points in the shared outline buffer; empty when it
    /// has no outline.
    pub fn edges(self) -> Range<u32> {
        self.first_edge_point..self.first_edge_point.saturating_add(self.edge_point_count)
    }
}

/// What a mesh is, by content: its sizes and a 128-bit hash of its bytes,
/// so two meshes alike in every byte share one key and different ones in
/// practice never do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct MeshKey {
    vertices: usize,
    indices: usize,
    edge_points: usize,
    hash: [u64; 2],
}

/// How many elements of each buffer a set of meshes takes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Sizes {
    vertices: u64,
    indices: u64,
    edge_points: u64,
}

impl Sizes {
    fn of(key: &MeshKey) -> Sizes {
        let count = |n: usize| u64::try_from(n).unwrap_or(u64::MAX);
        Sizes {
            vertices: count(key.vertices),
            indices: count(key.indices),
            edge_points: count(key.edge_points),
        }
    }

    fn add(self, other: Sizes) -> Sizes {
        Sizes {
            vertices: self.vertices.saturating_add(other.vertices),
            indices: self.indices.saturating_add(other.indices),
            edge_points: self.edge_points.saturating_add(other.edge_points),
        }
    }
}

/// One shared buffer, filled from the start.
struct Arena {
    label: &'static str,
    usage: wgpu::BufferUsages,
    /// Bytes per element.
    stride: u64,
    buffer: wgpu::Buffer,
    /// Elements it has room for.
    capacity: u64,
    /// Elements written so far.
    used: u64,
    /// Bytes written but not yet sent to the GPU, from element
    /// `staged_from` on.
    staged: Vec<u8>,
    staged_from: u64,
}

impl Arena {
    fn new(
        device: &wgpu::Device,
        label: &'static str,
        usage: wgpu::BufferUsages,
        stride: u64,
    ) -> Arena {
        Arena {
            label,
            usage,
            stride,
            buffer: Self::buffer(device, label, usage, stride, MINIMUM_CAPACITY),
            capacity: MINIMUM_CAPACITY,
            used: 0,
            staged: Vec::new(),
            staged_from: 0,
        }
    }

    fn buffer(
        device: &wgpu::Device,
        label: &'static str,
        usage: wgpu::BufferUsages,
        stride: u64,
        capacity: u64,
    ) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: capacity.saturating_mul(stride),
            usage: usage | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Whether `more` elements fit after those written.
    fn fits(&self, more: u64) -> bool {
        self.used.saturating_add(more) <= self.capacity
    }

    /// Empties it, with room for at least `live` elements and as many
    /// again to grow into, made anew only when its size changes: it grows
    /// by doubling and shrinks once it is eight times what is needed.
    fn reset(&mut self, device: &wgpu::Device, live: u64) -> Result<()> {
        let limit = device.limits().max_buffer_size / self.stride;
        if live > limit {
            bail!(
                "the design's {} need {} MB of graphics memory in one piece, and this GPU allows {} MB",
                self.label,
                live.saturating_mul(self.stride) >> 20,
                device.limits().max_buffer_size >> 20,
            );
        }
        let wanted = live
            .saturating_mul(2)
            .max(MINIMUM_CAPACITY)
            .checked_next_power_of_two()
            .unwrap_or(u64::MAX)
            .min(limit);
        let grow = live.saturating_mul(2) > self.capacity;
        let shrink = wanted.saturating_mul(4) < self.capacity;
        if grow || shrink {
            self.capacity = wanted;
            self.buffer = Self::buffer(device, self.label, self.usage, self.stride, wanted);
        }
        self.used = 0;
        self.staged.clear();
        Ok(())
    }

    /// Adds `bytes` (whole elements) after those written, returning where
    /// they start. They reach the GPU at [`Arena::flush`], in one write.
    fn write(&mut self, bytes: &[u8]) -> Result<u32> {
        let start = self.used;
        let count = u64::try_from(bytes.len())? / self.stride;
        if self.staged.is_empty() {
            self.staged_from = start;
        }
        self.staged.extend_from_slice(bytes);
        self.used = start.saturating_add(count);
        u32::try_from(start).with_context(|| format!("too many {} for one buffer", self.label))
    }

    /// Sends what [`Arena::write`] added to the GPU.
    fn flush(&mut self, queue: &wgpu::Queue) {
        if !self.staged.is_empty() {
            queue.write_buffer(
                &self.buffer,
                self.staged_from.saturating_mul(self.stride),
                &self.staged,
            );
            self.staged.clear();
        }
    }
}

/// The shared buffers and which meshes are where in them.
pub(super) struct MeshStore {
    vertices: Arena,
    indices: Arena,
    edges: Arena,
    /// Every mesh the buffers hold, used or not, by content.
    held: HashMap<MeshKey, MeshRange>,
    /// The last upload's meshes, node by node.
    keys: Vec<MeshKey>,
    /// Where each node of the last upload draws from.
    ranges: Vec<MeshRange>,
    /// Two independent hashers, for a 128-bit key.
    hashers: [RandomState; 2],
    /// Why the last upload failed, if it did.
    failed: Option<String>,
}

impl MeshStore {
    /// Empty buffers on `device`.
    pub fn new(device: &wgpu::Device) -> MeshStore {
        let stride = |size: usize| crate::convert::bytes(size);
        MeshStore {
            vertices: Arena::new(
                device,
                "vertices",
                wgpu::BufferUsages::VERTEX,
                stride(std::mem::size_of::<Vertex>()),
            ),
            indices: Arena::new(
                device,
                "indices",
                wgpu::BufferUsages::INDEX,
                stride(std::mem::size_of::<u32>()),
            ),
            edges: Arena::new(
                device,
                "outlines",
                wgpu::BufferUsages::VERTEX,
                stride(std::mem::size_of::<[f32; 3]>()),
            ),
            held: HashMap::new(),
            keys: Vec::new(),
            ranges: Vec::new(),
            hashers: [RandomState::new(), RandomState::new()],
            failed: None,
        }
    }

    /// The shared vertex buffer.
    pub fn vertex_buffer(&self) -> &wgpu::Buffer {
        &self.vertices.buffer
    }

    /// The shared index buffer (32-bit indices).
    pub fn index_buffer(&self) -> &wgpu::Buffer {
        &self.indices.buffer
    }

    /// The shared outline buffer (points, two per segment).
    pub fn edge_buffer(&self) -> &wgpu::Buffer {
        &self.edges.buffer
    }

    /// Where each node of the last upload draws from, by node index; empty
    /// when the upload failed.
    pub fn ranges(&self) -> &[MeshRange] {
        &self.ranges
    }

    /// Why the last upload failed, if it did.
    pub fn failed(&self) -> Option<&str> {
        self.failed.as_deref()
    }

    /// How many meshes the buffers hold, used or not.
    #[cfg(test)]
    pub fn held(&self) -> usize {
        self.held.len()
    }

    fn key(&self, geometry: &Geometry) -> MeshKey {
        let [first, second] = &self.hashers;
        let mesh = &geometry.mesh;
        let segments = &geometry.edges.segments;
        let hash = |state: &RandomState| {
            let mut hasher = state.build_hasher();
            hasher.write(bytemuck::cast_slice(&mesh.vertices));
            hasher.write(bytemuck::cast_slice(&mesh.indices));
            hasher.write(bytemuck::cast_slice(segments));
            hasher.finish()
        };
        MeshKey {
            vertices: mesh.vertices.len(),
            indices: mesh.indices.len(),
            edge_points: segments.len().saturating_mul(2),
            hash: [hash(first), hash(second)],
        }
    }

    /// Puts `nodes`' meshes in the buffers, writing only those not there
    /// already. Returns whether anything a node draws changed.
    pub fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, nodes: &[Node]) -> bool {
        // A mesh shared by several nodes is hashed once.
        let mut shared: HashMap<*const Geometry, MeshKey> = HashMap::new();
        let keys: Vec<MeshKey> = nodes
            .iter()
            .map(|node| {
                *shared
                    .entry(Arc::as_ptr(&node.local))
                    .or_insert_with(|| self.key(&node.local))
            })
            .collect();
        if self.failed.is_none() && keys == self.keys {
            return false;
        }
        match self.place(device, queue, nodes, &keys) {
            Ok(ranges) => {
                self.ranges = ranges;
                self.keys = keys;
                self.failed = None;
            }
            Err(error) => {
                // Nothing refers to what was written: start afresh.
                self.ranges.clear();
                self.keys.clear();
                self.held.clear();
                for arena in [&mut self.vertices, &mut self.indices, &mut self.edges] {
                    arena.used = 0;
                    arena.staged.clear();
                }
                self.failed = Some(format!("{error:#}"));
            }
        }
        true
    }

    fn place(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        nodes: &[Node],
        keys: &[MeshKey],
    ) -> Result<Vec<MeshRange>> {
        let mut fresh = Sizes::default();
        let mut live = Sizes::default();
        let mut counted = HashSet::new();
        for key in keys {
            if counted.insert(*key) {
                live = live.add(Sizes::of(key));
                if !self.held.contains_key(key) {
                    fresh = fresh.add(Sizes::of(key));
                }
            }
        }
        let fits = self.vertices.fits(fresh.vertices)
            && self.indices.fits(fresh.indices)
            && self.edges.fits(fresh.edge_points);
        if !fits {
            self.held.clear();
            self.vertices.reset(device, live.vertices)?;
            self.indices.reset(device, live.indices)?;
            self.edges.reset(device, live.edge_points)?;
        }
        let mut ranges = Vec::with_capacity(nodes.len());
        let mut rebased: Vec<u32> = Vec::new();
        for (node, key) in nodes.iter().zip(keys) {
            if let Some(range) = self.held.get(key) {
                ranges.push(*range);
                continue;
            }
            let mesh = &node.local.mesh;
            let base = self.vertices.write(bytemuck::cast_slice(&mesh.vertices))?;
            rebased.clear();
            for index in &mesh.indices {
                rebased.push(
                    index
                        .checked_add(base)
                        .context("too many vertices for 32-bit indices")?,
                );
            }
            let first_index = self.indices.write(bytemuck::cast_slice(&rebased))?;
            let first_edge_point = self
                .edges
                .write(bytemuck::cast_slice(&node.local.edges.segments))?;
            let range = MeshRange {
                first_index,
                index_count: u32::try_from(key.indices)?,
                first_edge_point,
                edge_point_count: u32::try_from(key.edge_points)?,
            };
            self.held.insert(*key, range);
            ranges.push(range);
        }
        for arena in [&mut self.vertices, &mut self.indices, &mut self.edges] {
            arena.flush(queue);
        }
        Ok(ranges)
    }
}
