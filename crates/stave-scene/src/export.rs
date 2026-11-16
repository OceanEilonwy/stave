//! Binary glTF 2.0 (`.glb`) export of a posed scene, so a design can be
//! checked in any glTF viewer or in Blender.
//!
//! One glTF node per shown scene node, named after the part or hardware,
//! with its mesh baked in world position (glTF's frame is the render frame,
//! in metres). Wood gets one material per species and stock form, hardware
//! a metal material.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::graph::{Item, Material, Posed, SceneGraph};

/// Why a scene couldn't be exported.
#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    /// The scene is larger than a glTF buffer can address (4 GiB).
    #[error("the scene is too large for a glTF file")]
    TooLarge,
    /// Writing the JSON chunk failed.
    #[error("couldn't write the glTF description: {0}")]
    Json(#[from] serde_json::Error),
}

const ARRAY_BUFFER: u32 = 34962;
const ELEMENT_ARRAY_BUFFER: u32 = 34963;
const FLOAT: u32 = 5126;
const UNSIGNED_INT: u32 = 5125;
const GLB_MAGIC: u32 = 0x4654_6C67;
const CHUNK_JSON: u32 = 0x4E4F_534A;
const CHUNK_BIN: u32 = 0x004E_4942;
const METRES_PER_MM: f32 = 0.001;

fn length_u32(length: usize) -> Result<u32, ExportError> {
    u32::try_from(length).map_err(|_| ExportError::TooLarge)
}

/// Pads to a multiple of four bytes.
fn pad(bytes: &mut Vec<u8>, with: u8) {
    while !bytes.len().is_multiple_of(4) {
        bytes.push(with);
    }
}

/// The material key and its glTF description.
fn material(material: &Material) -> (String, Value) {
    match material {
        Material::Wood(wood) => {
            let [r, g, b] = wood.colour;
            let sheet = if wood.figure < 1.0 { " sheet" } else { "" };
            let name = format!("{}{sheet}", wood.species.title());
            let description = json!({
                "name": name,
                "pbrMetallicRoughness": {
                    "baseColorFactor": [r, g, b, 1.0],
                    "metallicFactor": 0.0,
                    "roughnessFactor": 0.75,
                },
            });
            (name, description)
        }
        Material::Metal => {
            let [r, g, b] = material.base_colour();
            (
                "brushed metal".to_owned(),
                json!({
                    "name": "brushed metal",
                    "pbrMetallicRoughness": {
                        "baseColorFactor": [r, g, b, 1.0],
                        "metallicFactor": 0.85,
                        "roughnessFactor": 0.35,
                    },
                }),
            )
        }
    }
}

impl SceneGraph {
    /// The scene as a binary glTF file.
    pub fn to_glb(&self, posed: &Posed) -> Result<Vec<u8>, ExportError> {
        let mut binary: Vec<u8> = Vec::new();
        let mut buffer_views = Vec::new();
        let mut accessors = Vec::new();
        let mut meshes = Vec::new();
        let mut nodes = Vec::new();
        let mut materials: BTreeMap<String, (usize, Value)> = BTreeMap::new();
        for id in self.node_ids() {
            let (Some(transform), Some(node)) = (posed.shown(id), self.node(id)) else {
                continue;
            };
            let mesh = &node.geometry.mesh;
            if mesh.indices.is_empty() {
                continue;
            }
            let (key, description) = material(&node.material);
            let next = materials.len();
            let material_index = materials.entry(key).or_insert((next, description)).0;

            let mut min = [f32::INFINITY; 3];
            let mut max = [f32::NEG_INFINITY; 3];
            let positions_start = binary.len();
            for vertex in &mesh.vertices {
                let p = transform.transform_point3(vertex.position.into()) * METRES_PER_MM;
                for (axis, value) in p.to_array().into_iter().enumerate() {
                    if let (Some(low), Some(high)) = (min.get_mut(axis), max.get_mut(axis)) {
                        *low = low.min(value);
                        *high = high.max(value);
                    }
                    binary.extend(value.to_le_bytes());
                }
            }
            let normals_start = binary.len();
            for vertex in &mesh.vertices {
                let n = transform
                    .transform_vector3(vertex.normal.into())
                    .normalize_or_zero();
                for value in n.to_array() {
                    binary.extend(value.to_le_bytes());
                }
            }
            let indices_start = binary.len();
            for index in &mesh.indices {
                binary.extend(index.to_le_bytes());
            }
            let end = binary.len();
            let count = length_u32(mesh.vertices.len())?;
            let view = |start: usize, stop: usize, target: u32| -> Result<Value, ExportError> {
                Ok(json!({
                    "buffer": 0,
                    "byteOffset": length_u32(start)?,
                    "byteLength": length_u32(stop - start)?,
                    "target": target,
                }))
            };
            let first_view = buffer_views.len();
            buffer_views.push(view(positions_start, normals_start, ARRAY_BUFFER)?);
            buffer_views.push(view(normals_start, indices_start, ARRAY_BUFFER)?);
            buffer_views.push(view(indices_start, end, ELEMENT_ARRAY_BUFFER)?);
            let first_accessor = accessors.len();
            accessors.push(json!({
                "bufferView": first_view,
                "componentType": FLOAT,
                "count": count,
                "type": "VEC3",
                "min": min,
                "max": max,
            }));
            accessors.push(json!({
                "bufferView": first_view + 1,
                "componentType": FLOAT,
                "count": count,
                "type": "VEC3",
            }));
            accessors.push(json!({
                "bufferView": first_view + 2,
                "componentType": UNSIGNED_INT,
                "count": length_u32(mesh.indices.len())?,
                "type": "SCALAR",
            }));
            meshes.push(json!({
                "name": node.name,
                "primitives": [{
                    "attributes": {
                        "POSITION": first_accessor,
                        "NORMAL": first_accessor + 1,
                    },
                    "indices": first_accessor + 2,
                    "material": material_index,
                }],
            }));
            let kind = match node.item {
                Item::Part(_) => "part",
                Item::Hardware(_) => "hardware",
            };
            nodes.push(json!({
                "name": node.name,
                "mesh": meshes.len() - 1,
                "extras": { "stave": kind },
            }));
        }
        let mut material_list: Vec<(usize, Value)> = materials.into_values().collect();
        material_list.sort_by_key(|(index, _)| *index);
        let document = json!({
            "asset": { "version": "2.0", "generator": "Stave" },
            "scene": 0,
            "scenes": [{ "name": self.title, "nodes": (0..nodes.len()).collect::<Vec<_>>() }],
            "nodes": nodes,
            "meshes": meshes,
            "materials": material_list.into_iter().map(|(_, m)| m).collect::<Vec<_>>(),
            "accessors": accessors,
            "bufferViews": buffer_views,
            "buffers": [{ "byteLength": length_u32(binary.len())? }],
        });
        let mut json_chunk = serde_json::to_vec(&document)?;
        pad(&mut json_chunk, b' ');
        pad(&mut binary, 0);
        let total = 12 + 8 + json_chunk.len() + 8 + binary.len();
        let mut glb = Vec::with_capacity(total);
        glb.extend(GLB_MAGIC.to_le_bytes());
        glb.extend(2_u32.to_le_bytes());
        glb.extend(length_u32(total)?.to_le_bytes());
        glb.extend(length_u32(json_chunk.len())?.to_le_bytes());
        glb.extend(CHUNK_JSON.to_le_bytes());
        glb.extend(json_chunk);
        glb.extend(length_u32(binary.len())?.to_le_bytes());
        glb.extend(CHUNK_BIN.to_le_bytes());
        glb.extend(binary);
        Ok(glb)
    }
}
