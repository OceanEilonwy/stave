//! Reading a hardware kind's `.glb` model, as far as checking it against
//! the kind's text needs (grammar, "Checked against the text"): the name,
//! origin and mesh bounds of every node, after every transform above it.
//! Nothing is drawn from it here, and no check depends on it.
//!
//! A `.glb` is a 12-byte header (`glTF`, version 2, length) and chunks;
//! the first is the JSON document. Only that chunk is read: a mesh's
//! bounds are its `POSITION` accessor's `min` and `max`, which glTF
//! requires, so the binary buffer is never decoded.
//!
//! **Frames.** A [`ModelNode`] is in glTF's own frame (metres, +Y up). The
//! kind's frame is `along` = +X, `across` = +Z, `depth` = -Y
//! ([`Axes::YUp`]); [`Axes::ZUp`] reads a file exported from Blender
//! without "+Y Up", for the help line. The conversion to exact lengths is
//! [`crate::render::length_from_nm`], the one place floats become lengths.

use glam::{DMat4, DQuat, DVec3};
use serde_json::Value;
use stave_lang::units::Length;
use stave_model::solved::{FramePoint, FrameShape};
use stave_model::value::Sha256;

use crate::render;

/// The largest model read: 20 MB (grammar, "Hardware").
pub const MAX_BYTES: usize = 20_000_000;

/// `glTF`, little-endian.
const MAGIC: u32 = 0x4654_6C67;
/// `JSON`, little-endian.
const JSON_CHUNK: u32 = 0x4E4F_534A;

/// Why a file is not a model Stave can read. The message completes "can't
/// read <kind>'s model: ...".
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GlbError {
    /// Over [`MAX_BYTES`].
    #[error("the file is {bytes} bytes; a model may be at most 20 MB")]
    TooLarge {
        /// How big it is.
        bytes: usize,
    },
    /// No `glTF` header.
    #[error("it is not a binary glTF (.glb) file")]
    NotGlb,
    /// A glTF version other than 2.
    #[error("it is glTF version {0}; Stave reads version 2")]
    Version(u32),
    /// Shorter than its header or a chunk says.
    #[error("the file is cut short")]
    Truncated,
    /// The first chunk is not JSON.
    #[error("its first chunk is not the JSON document")]
    NoJson,
    /// The JSON chunk is not valid JSON, or not a glTF document.
    #[error("its JSON is not a glTF document: {0}")]
    Json(String),
}

/// How a model's axes are read into the kind's frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axes {
    /// glTF's convention, which Stave expects: +Y up.
    YUp,
    /// As if exported from Blender without "+Y Up": +Z up.
    ZUp,
}

/// A box in a hardware kind's frame, each range low end first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameBounds {
    /// Along the piece.
    pub along: [Length; 2],
    /// Across the mount face.
    pub across: [Length; 2],
    /// Into the piece.
    pub depth: [Length; 2],
}

impl FrameBounds {
    /// The smallest box holding both.
    pub fn union(&self, other: &FrameBounds) -> FrameBounds {
        let join = |[a, b]: [Length; 2], [c, d]: [Length; 2]| [a.min(c), b.max(d)];
        FrameBounds {
            along: join(self.along, other.along),
            across: join(self.across, other.across),
            depth: join(self.depth, other.depth),
        }
    }
}

/// One node of a model, in glTF's frame (metres, +Y up), after every
/// transform above it.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelNode {
    /// Its name; an unnamed node has none and is never looked up.
    pub name: Option<String>,
    /// Its origin.
    pub translation: [f64; 3],
    /// Its own mesh's bounds, `[min, max]`, when it has a mesh whose
    /// `POSITION` gives them.
    pub bounds: Option<[[f64; 3]; 2]>,
}

impl ModelNode {
    /// Its origin in the kind's frame, relative to `origin` (a node's
    /// translation, in glTF's frame), read with `axes`. `None` when a value
    /// is out of range.
    pub fn point(&self, origin: [f64; 3], axes: Axes) -> Option<FramePoint> {
        let [along, across, depth] = frame(minus(self.translation, origin), axes);
        Some(FramePoint {
            along: metres(along)?,
            across: metres(across)?,
            depth: metres(depth)?,
        })
    }

    /// Its mesh's bounds in the kind's frame, relative to `origin`, read
    /// with `axes`. `None` without a mesh or when a value is out of range.
    pub fn bounds(&self, origin: [f64; 3], axes: Axes) -> Option<FrameBounds> {
        let [low, high] = self.bounds?;
        let low = frame(minus(low, origin), axes);
        let high = frame(minus(high, origin), axes);
        let range = |a: f64, b: f64| -> Option<[Length; 2]> {
            Some([metres(a.min(b))?, metres(a.max(b))?])
        };
        Some(FrameBounds {
            along: range(low[0], high[0])?,
            across: range(low[1], high[1])?,
            depth: range(low[2], high[2])?,
        })
    }
}

/// A model's nodes, in the order a walk of its scene meets them.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Model {
    /// Every node.
    pub nodes: Vec<ModelNode>,
}

impl Model {
    /// The first node with this name.
    pub fn node(&self, name: &str) -> Option<&ModelNode> {
        self.nodes
            .iter()
            .find(|node| node.name.as_deref() == Some(name))
    }
}

/// Reads a `.glb` file's nodes.
pub fn read(bytes: &[u8]) -> Result<Model, GlbError> {
    if bytes.len() > MAX_BYTES {
        return Err(GlbError::TooLarge { bytes: bytes.len() });
    }
    let word = |at: usize| -> Option<u32> {
        let slice = bytes.get(at..at.checked_add(4)?)?;
        Some(u32::from_le_bytes(slice.try_into().ok()?))
    };
    if word(0) != Some(MAGIC) {
        return Err(GlbError::NotGlb);
    }
    let version = word(4).ok_or(GlbError::Truncated)?;
    if version != 2 {
        return Err(GlbError::Version(version));
    }
    let length =
        usize::try_from(word(8).ok_or(GlbError::Truncated)?).map_err(|_| GlbError::Truncated)?;
    if length > bytes.len() {
        return Err(GlbError::Truncated);
    }
    let chunk_length =
        usize::try_from(word(12).ok_or(GlbError::Truncated)?).map_err(|_| GlbError::Truncated)?;
    if word(16).ok_or(GlbError::Truncated)? != JSON_CHUNK {
        return Err(GlbError::NoJson);
    }
    let json = 20usize
        .checked_add(chunk_length)
        .and_then(|end| bytes.get(20..end))
        .ok_or(GlbError::Truncated)?;
    let document: Value =
        serde_json::from_slice(json).map_err(|error| GlbError::Json(error.to_string()))?;
    nodes(&document)
}

/// Every node of a glTF document, walked from its scene's roots.
fn nodes(document: &Value) -> Result<Model, GlbError> {
    let empty = Vec::new();
    let all = match document.get("nodes") {
        Some(Value::Array(nodes)) => nodes,
        Some(_) => return Err(GlbError::Json("'nodes' is not a list".to_owned())),
        None => &empty,
    };
    let index = |value: &Value| value.as_u64().and_then(|i| usize::try_from(i).ok());
    let scene = document.get("scene").and_then(index).unwrap_or(0);
    let roots: Vec<usize> = match document
        .get("scenes")
        .and_then(|scenes| scenes.get(scene))
        .and_then(|scene| scene.get("nodes"))
        .and_then(Value::as_array)
    {
        Some(roots) => roots.iter().filter_map(index).collect(),
        // No scene: every node that is no node's child.
        None => {
            let children: Vec<usize> = all
                .iter()
                .filter_map(|node| node.get("children").and_then(Value::as_array))
                .flatten()
                .filter_map(index)
                .collect();
            (0..all.len()).filter(|i| !children.contains(i)).collect()
        }
    };
    let mut out = Vec::new();
    let mut seen = vec![false; all.len()];
    // Depth first, in order: each entry is a node and its parent's world
    // transform.
    let mut stack: Vec<(usize, DMat4)> = roots
        .into_iter()
        .rev()
        .map(|root| (root, DMat4::IDENTITY))
        .collect();
    while let Some((at, parent)) = stack.pop() {
        let (Some(node), Some(visited)) = (all.get(at), seen.get_mut(at)) else {
            return Err(GlbError::Json(format!("there is no node {at}")));
        };
        if *visited {
            return Err(GlbError::Json(format!("node {at} is reached twice")));
        }
        *visited = true;
        let world = parent * local(node)?;
        let translation = world.transform_point3(DVec3::ZERO).to_array();
        let bounds = match node.get("mesh").and_then(index) {
            Some(mesh) => mesh_bounds(document, mesh)?.map(|[low, high]| {
                // The eight corners, transformed.
                let mut min = DVec3::splat(f64::INFINITY);
                let mut max = DVec3::splat(f64::NEG_INFINITY);
                for corner in 0..8u8 {
                    let pick = |bit: u8, axis: usize| {
                        let source = if corner & bit == 0 { low } else { high };
                        source.get(axis).copied().unwrap_or(0.0)
                    };
                    let point =
                        world.transform_point3(DVec3::new(pick(1, 0), pick(2, 1), pick(4, 2)));
                    min = min.min(point);
                    max = max.max(point);
                }
                [min.to_array(), max.to_array()]
            }),
            None => None,
        };
        out.push(ModelNode {
            name: node.get("name").and_then(Value::as_str).map(str::to_owned),
            translation,
            bounds,
        });
        if let Some(children) = node.get("children").and_then(Value::as_array) {
            for child in children.iter().rev().filter_map(index) {
                stack.push((child, world));
            }
        }
    }
    Ok(Model { nodes: out })
}

/// A node's own transform: its `matrix`, or its translation, rotation and
/// scale.
fn local(node: &Value) -> Result<DMat4, GlbError> {
    let numbers = |name: &str, count: usize| -> Result<Option<Vec<f64>>, GlbError> {
        match node.get(name) {
            None => Ok(None),
            Some(Value::Array(values)) if values.len() == count => values
                .iter()
                .map(|value| {
                    value
                        .as_f64()
                        .ok_or_else(|| GlbError::Json(format!("a node's '{name}' is not numbers")))
                })
                .collect::<Result<Vec<f64>, GlbError>>()
                .map(Some),
            Some(_) => Err(GlbError::Json(format!(
                "a node's '{name}' needs {count} numbers"
            ))),
        }
    };
    if let Some(matrix) = numbers("matrix", 16)? {
        let mut columns = [0.0; 16];
        for (slot, value) in columns.iter_mut().zip(matrix) {
            *slot = value;
        }
        return Ok(DMat4::from_cols_array(&columns));
    }
    let vector = |values: Option<Vec<f64>>, default: DVec3| match values.as_deref() {
        Some([x, y, z]) => DVec3::new(*x, *y, *z),
        _ => default,
    };
    let translation = vector(numbers("translation", 3)?, DVec3::ZERO);
    let scale = vector(numbers("scale", 3)?, DVec3::ONE);
    let rotation = match numbers("rotation", 4)?.as_deref() {
        Some([x, y, z, w]) => DQuat::from_xyzw(*x, *y, *z, *w).normalize(),
        _ => DQuat::IDENTITY,
    };
    Ok(DMat4::from_scale_rotation_translation(
        scale,
        rotation,
        translation,
    ))
}

/// The bounds of every primitive of a mesh together, from their
/// `POSITION` accessors' `min` and `max`; `None` when it gives none.
fn mesh_bounds(document: &Value, mesh: usize) -> Result<Option<[[f64; 3]; 2]>, GlbError> {
    let Some(primitives) = document
        .get("meshes")
        .and_then(|meshes| meshes.get(mesh))
        .and_then(|mesh| mesh.get("primitives"))
        .and_then(Value::as_array)
    else {
        return Err(GlbError::Json(format!("there is no mesh {mesh}")));
    };
    let mut bounds: Option<[[f64; 3]; 2]> = None;
    for primitive in primitives {
        let Some(accessor) = primitive
            .get("attributes")
            .and_then(|attributes| attributes.get("POSITION"))
            .and_then(Value::as_u64)
            .and_then(|i| usize::try_from(i).ok())
            .and_then(|i| document.get("accessors")?.get(i))
        else {
            continue;
        };
        let corner = |name: &str| -> Option<[f64; 3]> {
            match accessor.get(name)?.as_array()?.as_slice() {
                [x, y, z] => Some([x.as_f64()?, y.as_f64()?, z.as_f64()?]),
                _ => None,
            }
        };
        let (Some(low), Some(high)) = (corner("min"), corner("max")) else {
            continue;
        };
        bounds = Some(match bounds {
            None => [low, high],
            Some([a, b]) => [
                [a[0].min(low[0]), a[1].min(low[1]), a[2].min(low[2])],
                [b[0].max(high[0]), b[1].max(high[1]), b[2].max(high[2])],
            ],
        });
    }
    Ok(bounds)
}

fn minus([a, b, c]: [f64; 3], [x, y, z]: [f64; 3]) -> [f64; 3] {
    [a - x, b - y, c - z]
}

/// A glTF-frame vector as `[along, across, depth]`.
fn frame([x, y, z]: [f64; 3], axes: Axes) -> [f64; 3] {
    match axes {
        Axes::YUp => [x, z, -y],
        Axes::ZUp => [x, -y, -z],
    }
}

/// Metres as a length, to the nearest nanometre.
fn metres(value: f64) -> Option<Length> {
    render::length_from_nm(value * 1e9)
}

/// The bounds of a member's declared shapes together, in the kind's frame:
/// a bar's round its four corners. `None` for no shapes, or a value out of
/// range.
pub fn shape_bounds(shapes: &[FrameShape]) -> Option<FrameBounds> {
    let mut out: Option<FrameBounds> = None;
    for shape in shapes {
        let bounds = match *shape {
            FrameShape::Bounds {
                along,
                across,
                depth,
            } => FrameBounds {
                along,
                across,
                depth,
            },
            FrameShape::Bar {
                from,
                to,
                thickness,
                along,
            } => bar_bounds(from, to, thickness, along)?,
        };
        out = Some(match out {
            None => bounds,
            Some(sofar) => sofar.union(&bounds),
        });
    }
    out
}

/// A bar's bounds: a rectangle `thickness` wide, reaching half its
/// thickness past each centre, on the across/depth plane.
fn bar_bounds(
    from: [Length; 2],
    to: [Length; 2],
    thickness: Length,
    along: [Length; 2],
) -> Option<FrameBounds> {
    let point = |[a, d]: [Length; 2]| glam::DVec2::new(render::nm(a), render::nm(d));
    let (start, end) = (point(from), point(to));
    let half = render::nm(thickness) / 2.0;
    let direction = (end - start).try_normalize().unwrap_or(glam::DVec2::X);
    let normal = direction.perp();
    let corners = [
        start - direction * half + normal * half,
        start - direction * half - normal * half,
        end + direction * half + normal * half,
        end + direction * half - normal * half,
    ];
    let low = corners.iter().copied().reduce(glam::DVec2::min)?;
    let high = corners.iter().copied().reduce(glam::DVec2::max)?;
    Some(FrameBounds {
        along,
        across: [
            render::length_from_nm(low.x)?,
            render::length_from_nm(high.x)?,
        ],
        depth: [
            render::length_from_nm(low.y)?,
            render::length_from_nm(high.y)?,
        ],
    })
}

/// The SHA-256 digest of a file's bytes (FIPS 180-4).
pub fn sha256(bytes: &[u8]) -> Sha256 {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut state: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    // The message, a 1 bit, zeros to 56 bytes past a multiple of 64, and
    // its length in bits.
    let bits = u64::try_from(bytes.len())
        .unwrap_or(u64::MAX)
        .wrapping_mul(8);
    let mut message = bytes.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bits.to_be_bytes());
    for block in message.as_chunks::<64>().0 {
        let mut schedule = [0u32; 64];
        for (word, four) in schedule.iter_mut().zip(block.as_chunks::<4>().0) {
            *word = u32::from_be_bytes(*four);
        }
        for i in 16..64 {
            let at = |back: usize| schedule.get(i - back).copied().unwrap_or(0);
            let (w15, w2) = (at(15), at(2));
            let s0 = w15.rotate_right(7) ^ w15.rotate_right(18) ^ (w15 >> 3);
            let s1 = w2.rotate_right(17) ^ w2.rotate_right(19) ^ (w2 >> 10);
            let value = at(16).wrapping_add(s0).wrapping_add(at(7)).wrapping_add(s1);
            if let Some(slot) = schedule.get_mut(i) {
                *slot = value;
            }
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = state;
        for (k, w) in K.iter().zip(schedule.iter()) {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choose = (e & f) ^ (!e & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(choose)
                .wrapping_add(*k)
                .wrapping_add(*w);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(majority);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (slot, value) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *slot = slot.wrapping_add(value);
        }
    }
    let mut digest = [0u8; 32];
    for (chunk, word) in digest.as_chunks_mut::<4>().0.iter_mut().zip(state) {
        *chunk = word.to_be_bytes();
    }
    Sha256::from_bytes(digest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_the_standards_examples() {
        assert_eq!(
            sha256(b"").to_string(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256(b"abc").to_string(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        // 56 bytes: the padding takes a second block.
        assert_eq!(
            sha256(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq").to_string(),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    /// A `.glb` holding only this JSON document.
    fn file(json: &str) -> Vec<u8> {
        let mut json = json.as_bytes().to_vec();
        while !json.len().is_multiple_of(4) {
            json.push(b' ');
        }
        let length = u32::try_from(json.len()).unwrap();
        let mut out = b"glTF".to_vec();
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&(length + 20).to_le_bytes());
        out.extend_from_slice(&length.to_le_bytes());
        out.extend_from_slice(b"JSON");
        out.extend_from_slice(&json);
        out
    }

    fn mm(value: i64) -> Length {
        Length::from_mm(value)
    }

    #[test]
    fn nodes_carry_every_transform_above_them() {
        // A parent 10mm along and scaled by 2; its child 1mm up (-depth)
        // with a mesh from -1mm to 1mm on each axis, turned 90deg about
        // +Y (which takes +X to -Z).
        let half = std::f64::consts::FRAC_1_SQRT_2;
        let model = read(&file(&format!(
            r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0]}}],
            "nodes":[{{"name":"parent","translation":[0.01,0,0],"scale":[2,2,2],"children":[1]}},
                     {{"name":"child","translation":[0,0.001,0],"rotation":[0,{half},0,{half}],"mesh":0}}],
            "meshes":[{{"primitives":[{{"attributes":{{"POSITION":0}}}}]}}],
            "accessors":[{{"count":2,"type":"VEC3","componentType":5126,"min":[-0.001,-0.001,-0.002],"max":[0.001,0.001,0.002]}}]}}"#
        )))
        .unwrap();
        let child = model.node("child").unwrap();
        assert_eq!(
            child.point([0.0; 3], Axes::YUp),
            Some(FramePoint {
                along: mm(10),
                across: mm(0),
                depth: mm(-2),
            })
        );
        // The mesh's z (4mm long, scaled to 8mm) now runs along x.
        let bounds = child.bounds([0.0; 3], Axes::YUp).unwrap();
        assert_eq!(bounds.along, [mm(6), mm(14)]);
        assert_eq!(bounds.across, [mm(-2), mm(2)]);
        assert_eq!(bounds.depth, [mm(-4), mm(0)]);
        // Read Z up, glTF's -Y is across and -Z is depth.
        assert_eq!(
            child.point([0.0; 3], Axes::ZUp),
            Some(FramePoint {
                along: mm(10),
                across: mm(-2),
                depth: mm(0),
            })
        );
        assert!(model.node("parent").unwrap().bounds.is_none());
        assert!(model.node("nothing").is_none());
    }

    #[test]
    fn a_matrix_is_read_by_columns() {
        let model = read(&file(
            r#"{"nodes":[{"name":"pin","matrix":[1,0,0,0, 0,1,0,0, 0,0,1,0, 0.002,0,0.003,1]}]}"#,
        ))
        .unwrap();
        let pin = model.node("pin").unwrap().point([0.0; 3], Axes::YUp);
        assert_eq!(
            pin,
            Some(FramePoint {
                along: mm(2),
                across: mm(3),
                depth: mm(0),
            })
        );
    }

    #[test]
    fn files_that_are_not_models_are_refused() {
        assert_eq!(read(b"solid"), Err(GlbError::NotGlb));
        let mut version_one = file("{}");
        version_one[4] = 1;
        assert_eq!(read(&version_one), Err(GlbError::Version(1)));
        let whole = file("{}");
        assert_eq!(read(&whole[..14]), Err(GlbError::Truncated));
        assert!(matches!(read(&file("[1, 2")), Err(GlbError::Json(_))));
        assert!(matches!(
            read(&file(
                r#"{"nodes":[{"children":[0]}],"scenes":[{"nodes":[0]}]}"#
            )),
            Err(GlbError::Json(_))
        ));
        assert_eq!(
            read(&vec![0u8; MAX_BYTES + 1]),
            Err(GlbError::TooLarge {
                bytes: MAX_BYTES + 1
            })
        );
    }

    #[test]
    fn a_bars_bounds_reach_round_its_corners() {
        // A bar 10mm long lying at 45deg, 2mm thick.
        let bar = FrameShape::Bar {
            from: [mm(0), mm(0)],
            to: [mm(10), mm(10)],
            thickness: mm(2),
            along: [mm(-1), mm(1)],
        };
        let bounds = shape_bounds(&[bar]).unwrap();
        // Each corner is sqrt(2) out from the end centre on each axis.
        let corner = Length::from_nm(1_414_214);
        assert_eq!(
            bounds.across,
            [
                Length::ZERO.checked_sub(corner).unwrap(),
                mm(10).checked_add(corner).unwrap()
            ]
        );
        assert_eq!(bounds.depth, bounds.across);
        assert_eq!(bounds.along, [mm(-1), mm(1)]);
        // A box and a bar together: the smallest box holding both.
        let both = shape_bounds(&[
            bar,
            FrameShape::Bounds {
                along: [mm(-5), mm(0)],
                across: [mm(0), mm(1)],
                depth: [mm(0), mm(1)],
            },
        ])
        .unwrap();
        assert_eq!(both.along, [mm(-5), mm(1)]);
        assert!(shape_bounds(&[]).is_none());
    }
}
