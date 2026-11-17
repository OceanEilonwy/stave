//! Models checked against the text (grammar, "Checked against the text"):
//! a cabinet door on two cup hinges whose kind (the standard `euro_hinge`,
//! copied into the project) has a `.glb` model.
//!
//! The models are built here, by [`glb`], from the nodes the solved
//! design says the kind's text declares, then written next to the project
//! in a scratch folder under the target directory: no Blender and no
//! Python, and a fixture can never go stale against the kind. Each test
//! then spoils the model the way a real export goes wrong.

mod common;

use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

use stave_app::Opened;
use stave_geom::glb::shape_bounds;
use stave_lang::render_diagnostics;
use stave_lang::units::Length;
use stave_model::solved::{DeclaredNode, DeclaredValue, FramePoint};

/// The standard hinges, whose `euro_hinge` the project copies.
const STD_HINGES: &str = include_str!("../../../std/hardware/hinges.stave");

/// The standard cup hinge as a project kind `modelled_hinge`, with
/// `model` as its model entry.
fn modelled_hinge(model: &str) -> String {
    let start = STD_HINGES
        .find("kind euro_hinge : hinge {")
        .expect("std has euro_hinge");
    let rest = &STD_HINGES[start..];
    let end = rest.find("\n}\n").expect("euro_hinge ends") + 1;
    let body = &rest[..end].replacen("kind euro_hinge", "kind modelled_hinge", 1);
    format!("{body}  model: {model}\n}}\n")
}

/// One door on two `modelled_hinge`s, as the hinge oracle builds it.
fn project(model: &str) -> String {
    format!(
        "stave 1

project \"Model check\" {{
  units: mm
}}

stock side_19 {{ species: birch, form: plywood, thickness: 19mm }}

part base : side_19 {{
  length: 600mm
  width:  400mm
  place {{
    ground
  }}
}}

part side_l : side_19 {{
  length: 700mm
  width:  400mm
  place {{
    orient(length: up, width: back)
    against(base.top)
    flush(base.left)
    flush(base.back)
  }}
}}

part side_r = mirror(side_l, about: mid(base, across))

region bay = opening(
  left:   side_l.right,
  right:  side_r.left,
  bottom: base.top,
  top:    side_l.top,
  front:  base.front,
  back:   base.back,
)

region face = cover(bay, overlay(left: 17.5mm, right: 17.5mm), thickness: side_19.thickness)

part door : side_19 {{
  length: face.up
  width:  face.across
  place {{
    orient(length: up, width: right)
    flush(face.left)
    flush(face.bottom)
    flush(face.front)
  }}
}}

hardware hinges[bottom, top] : modelled_hinge(door, on: side_l) {{
  place {{
    spaced(ends(inset: 80mm), from: door.bottom, to: door.top)
  }}
}}

{}",
        modelled_hinge(model)
    )
}

/// A scratch folder for one test, emptied.
fn folder(name: &str) -> PathBuf {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("models")
        .join(name);
    let _ = fs::remove_dir_all(&folder);
    fs::create_dir_all(&folder).expect("the scratch folder is made");
    folder
}

/// Writes the project (and the model, when there is one) and opens it.
fn open(name: &str, model: &str, glb: Option<&[u8]>) -> Opened {
    let folder = folder(name);
    fs::write(folder.join("project.stave"), project(model)).expect("the project is written");
    if let Some(bytes) = glb {
        fs::write(folder.join("hinge.glb"), bytes).expect("the model is written");
    }
    stave_app::open(&folder).expect("the project loads")
}

/// The nodes the kind's text declares, read from a design with no model
/// file (which is quiet), in a folder of the test's own.
fn declared(test: &str) -> Vec<DeclaredNode> {
    let opened = open(&format!("{test}_declared"), "model(\"./hinge.glb\")", None);
    assert!(
        opened.diagnostics.is_empty(),
        "{}",
        render_diagnostics(&opened.sources, &opened.diagnostics)
    );
    let hinge = common::hardware(common::design(&opened), "hinges[bottom]");
    hinge
        .model
        .as_ref()
        .expect("the kind has a model")
        .nodes
        .clone()
}

/// How a test model is exported.
#[derive(Clone, Copy)]
struct Export {
    /// Blender's "+Y Up" left unticked.
    z_up: bool,
    /// Every value times this (1000: drawn in millimetres).
    scale: f64,
}

const RIGHT: Export = Export {
    z_up: false,
    scale: 1.0,
};

/// A point in the kind's frame (nanometres) in the file's frame (metres).
fn gltf([along, across, depth]: [f64; 3], export: Export) -> [f64; 3] {
    let metres = |nm: f64| nm * 1e-9 * export.scale;
    if export.z_up {
        [metres(along), -metres(across), -metres(depth)]
    } else {
        [metres(along), -metres(depth), metres(across)]
    }
}

fn nm(length: Length) -> f64 {
    stave_geom::render::nm(length)
}

/// A `.glb` with a node for every declared node: an empty at each point,
/// a mesh spanning each shape's bounds. `moved` shifts one node `across`
/// by some nanometres; `extra` adds an unnamed node (a change that leaves
/// every named node as it was).
fn glb(nodes: &[DeclaredNode], export: Export, moved: Option<(&str, f64)>, extra: bool) -> Vec<u8> {
    let mut json_nodes = Vec::new();
    let mut meshes = Vec::new();
    let mut accessors = Vec::new();
    let mut views = Vec::new();
    let mut buffer: Vec<u8> = Vec::new();
    for node in nodes {
        let shift = match moved {
            Some((name, by)) if name == node.name => by,
            _ => 0.0,
        };
        match &node.value {
            DeclaredValue::Point(FramePoint {
                along,
                across,
                depth,
            }) => {
                let [x, y, z] = gltf([nm(*along), nm(*across) + shift, nm(*depth)], export);
                json_nodes.push(format!(
                    "{{\"name\":\"{}\",\"translation\":[{x},{y},{z}]}}",
                    node.name
                ));
            }
            DeclaredValue::Shapes(shapes) => {
                let bounds = shape_bounds(shapes).expect("shapes have bounds");
                let low = gltf(
                    [
                        nm(bounds.along[0]),
                        nm(bounds.across[0]) + shift,
                        nm(bounds.depth[0]),
                    ],
                    export,
                );
                let high = gltf(
                    [
                        nm(bounds.along[1]),
                        nm(bounds.across[1]) + shift,
                        nm(bounds.depth[1]),
                    ],
                    export,
                );
                let min: Vec<f64> = (0..3).map(|i| low[i].min(high[i])).collect();
                let max: Vec<f64> = (0..3).map(|i| low[i].max(high[i])).collect();
                let offset = buffer.len();
                for value in min.iter().chain(&max) {
                    buffer.extend_from_slice(&(*value as f32).to_le_bytes());
                }
                let index = accessors.len();
                views.push(format!(
                    "{{\"buffer\":0,\"byteOffset\":{offset},\"byteLength\":24}}"
                ));
                accessors.push(format!(
                    "{{\"bufferView\":{index},\"componentType\":5126,\"count\":2,\"type\":\"VEC3\",\"min\":[{},{},{}],\"max\":[{},{},{}]}}",
                    min[0], min[1], min[2], max[0], max[1], max[2]
                ));
                meshes.push(format!(
                    "{{\"primitives\":[{{\"attributes\":{{\"POSITION\":{index}}}}}]}}"
                ));
                json_nodes.push(format!("{{\"name\":\"{}\",\"mesh\":{index}}}", node.name));
            }
        }
    }
    if extra {
        json_nodes.push("{\"translation\":[0,0.1,0]}".to_owned());
    }
    let roots: Vec<String> = (0..json_nodes.len()).map(|i| i.to_string()).collect();
    let mut json = String::new();
    write!(
        json,
        "{{\"asset\":{{\"version\":\"2.0\"}},\"scene\":0,\"scenes\":[{{\"nodes\":[{}]}}],\"nodes\":[{}],\"meshes\":[{}],\"accessors\":[{}],\"bufferViews\":[{}],\"buffers\":[{{\"byteLength\":{}}}]}}",
        roots.join(","),
        json_nodes.join(","),
        meshes.join(","),
        accessors.join(","),
        views.join(","),
        buffer.len()
    )
    .expect("writing to a string");
    let mut json = json.into_bytes();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let word = |value: usize| u32::try_from(value).expect("small").to_le_bytes();
    let total = 12 + 8 + json.len() + 8 + buffer.len();
    let mut out = Vec::new();
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&word(total));
    out.extend_from_slice(&word(json.len()));
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&json);
    out.extend_from_slice(&word(buffer.len()));
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&buffer);
    out
}

/// The one diagnostic, which must be a `model-mismatch` warning.
fn the_warning(opened: &Opened) -> &stave_lang::Diagnostic {
    let rendered = render_diagnostics(&opened.sources, &opened.diagnostics);
    assert_eq!(opened.diagnostics.len(), 1, "{rendered}");
    let warning = &opened.diagnostics[0];
    assert_eq!(warning.code.as_str(), "model-mismatch", "{rendered}");
    assert!(!warning.is_error(), "{rendered}");
    warning
}

fn quiet(opened: &Opened) {
    assert!(
        opened.diagnostics.is_empty(),
        "{}",
        render_diagnostics(&opened.sources, &opened.diagnostics)
    );
}

#[test]
fn the_text_declares_a_node_for_every_shape_and_pin() {
    let names: Vec<String> = declared("the_text_declares_a_node_for_every_shape_and_pin")
        .into_iter()
        .map(|node| node.name)
        .collect();
    assert_eq!(
        names,
        [
            "plate",
            "arm body",
            "arm knuckle",
            "cup",
            "inner_link",
            "outer_link near",
            "outer_link far",
            "arm inner_link",
            "cup inner_link",
            "arm outer_link",
            "cup outer_link",
        ]
    );
}

#[test]
fn a_matching_model_is_quiet() {
    let nodes = declared("a_matching_model_is_quiet");
    quiet(&open(
        "matching",
        "model(\"./hinge.glb\")",
        Some(&glb(&nodes, RIGHT, None, false)),
    ));
    // Within 0.01mm is a match.
    quiet(&open(
        "within_tolerance",
        "model(\"./hinge.glb\")",
        Some(&glb(
            &nodes,
            RIGHT,
            Some(("arm inner_link", 5_000.0)),
            false,
        )),
    ));
}

#[test]
fn a_point_off_by_a_tenth_warns() {
    let nodes = declared("a_point_off_by_a_tenth_warns");
    let opened = open(
        "point_off",
        "model(\"./hinge.glb\")",
        Some(&glb(
            &nodes,
            RIGHT,
            Some(("arm inner_link", 100_000.0)),
            false,
        )),
    );
    let warning = the_warning(&opened);
    assert_eq!(
        warning.message,
        "modelled_hinge's model puts 'arm inner_link' at along 0mm, across 13.6mm, depth 10.9mm, but the kind says along 0mm, across 13.5mm, depth 10.9mm"
    );
    let help = common::help(&opened);
    assert!(help.contains("the kind's numbers are the truth"), "{help}");
    assert!(help.contains("+Y up") && help.contains("Ctrl+A"), "{help}");
    // At the kind's model entry.
    let source = opened.sources.get(warning.primary().unwrap().file).unwrap();
    let span = warning.primary().unwrap().span;
    assert_eq!(
        &source.text()[span.start() as usize..span.end() as usize],
        "model(\"./hinge.glb\")"
    );
}

#[test]
fn a_shape_off_by_a_tenth_warns() {
    let nodes = declared("a_shape_off_by_a_tenth_warns");
    let opened = open(
        "shape_off",
        "model(\"./hinge.glb\")",
        Some(&glb(&nodes, RIGHT, Some(("cup", 100_000.0)), false)),
    );
    assert_eq!(
        the_warning(&opened).message,
        "modelled_hinge's model has 'cup' over along -17.5mm..17.5mm, across -17.4mm..17.6mm, depth 0mm..12.8mm, but the kind says along -17.5mm..17.5mm, across -17.5mm..17.5mm, depth 0mm..12.8mm"
    );
}

#[test]
fn a_model_in_millimetres_gets_the_scale_help() {
    let nodes = declared("a_model_in_millimetres_gets_the_scale_help");
    let opened = open(
        "millimetres",
        "model(\"./hinge.glb\")",
        Some(&glb(
            &nodes,
            Export {
                z_up: false,
                scale: 1000.0,
            },
            None,
            false,
        )),
    );
    the_warning(&opened);
    let help = common::help(&opened);
    assert!(help.contains("1000 times too large"), "{help}");
}

#[test]
fn a_z_up_model_gets_the_axes_help() {
    let nodes = declared("a_z_up_model_gets_the_axes_help");
    let opened = open(
        "z_up",
        "model(\"./hinge.glb\")",
        Some(&glb(
            &nodes,
            Export {
                z_up: true,
                scale: 1.0,
            },
            None,
            false,
        )),
    );
    the_warning(&opened);
    let help = common::help(&opened);
    assert!(help.contains("Z up") && help.contains("+Y Up"), "{help}");
}

#[test]
fn a_changed_file_fails_its_pin() {
    let nodes = declared("a_changed_file_fails_its_pin");
    let pinned = glb(&nodes, RIGHT, None, false);
    let digest = stave_geom::glb::sha256(&pinned);
    let model = format!("model(\"./hinge.glb\", sha256: \"{digest}\")");
    quiet(&open("pinned", &model, Some(&pinned)));
    // Another file with the same nodes: only the pin notices.
    let changed = glb(&nodes, RIGHT, None, true);
    let found = stave_geom::glb::sha256(&changed);
    assert_ne!(found, digest);
    let opened = open("pin_changed", &model, Some(&changed));
    assert_eq!(
        the_warning(&opened).message,
        format!("modelled_hinge's model is not the pinned file: its SHA-256 is {found}")
    );
    assert!(common::help(&opened).contains(&format!("sha256: \"{found}\"")));
}

#[test]
fn a_pin_is_64_hexadecimal_digits() {
    let opened = open("bad_pin", "model(\"./hinge.glb\", sha256: \"abc\")", None);
    common::only_error(
        &opened,
        "type-mismatch",
        "'sha256' needs 64 hexadecimal digits: it has 3 characters, not 64",
    );
}

#[test]
fn a_model_without_the_nodes_lists_them() {
    let nodes = declared("a_model_without_the_nodes_lists_them");
    let some: Vec<DeclaredNode> = nodes
        .iter()
        .filter(|node| !node.name.starts_with("cup"))
        .cloned()
        .collect();
    let opened = open(
        "missing",
        "model(\"./hinge.glb\")",
        Some(&glb(&some, RIGHT, None, false)),
    );
    assert_eq!(
        the_warning(&opened).message,
        "modelled_hinge's model has no node 'cup', 'cup inner_link' and 'cup outer_link'"
    );
}

#[test]
fn align_puts_a_node_on_the_kinds_origin() {
    let mut nodes = declared("align_puts_a_node_on_the_kinds_origin");
    // Every node 10mm across, and a `mount` node there: a match.
    for node in &mut nodes {
        let shift = |length: &mut Length| *length = Length::from_nm(length.nm() + 10_000_000);
        match &mut node.value {
            DeclaredValue::Point(point) => shift(&mut point.across),
            DeclaredValue::Shapes(shapes) => {
                for shape in shapes {
                    match shape {
                        stave_model::solved::FrameShape::Bounds { across, .. } => {
                            shift(&mut across[0]);
                            shift(&mut across[1]);
                        }
                        stave_model::solved::FrameShape::Bar { from, to, .. } => {
                            shift(&mut from[0]);
                            shift(&mut to[0]);
                        }
                    }
                }
            }
        }
    }
    nodes.push(DeclaredNode {
        name: "mount".to_owned(),
        value: DeclaredValue::Point(FramePoint {
            along: Length::ZERO,
            across: Length::from_nm(10_000_000),
            depth: Length::ZERO,
        }),
    });
    let file = glb(&nodes, RIGHT, None, false);
    quiet(&open(
        "aligned",
        "model(\"./hinge.glb\", align: node(\"mount\"))",
        Some(&file),
    ));
    // Without align, the same file is 10mm off.
    let opened = open("not_aligned", "model(\"./hinge.glb\")", Some(&file));
    assert!(
        the_warning(&opened).message.contains("but the kind says"),
        "{}",
        the_warning(&opened).message
    );
}

#[test]
fn a_file_that_is_not_a_model_warns() {
    let opened = open(
        "not_glb",
        "model(\"./hinge.glb\")",
        Some(b"solid cube\nendsolid cube\n"),
    );
    assert_eq!(
        the_warning(&opened).message,
        "can't read modelled_hinge's model: it is not a binary glTF (.glb) file"
    );
}

/// `tests/broken/model_mismatch/knob.glb` is this helper's: its kind's
/// nodes with the knob drawn 0.1mm across. With `STAVE_WRITE_FIXTURES=1`
/// the test writes it instead of comparing.
#[test]
fn the_broken_designs_model_is_built_by_this_helper() {
    let folder =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/broken/model_mismatch");
    let opened = stave_app::open(&folder).expect("the broken design loads");
    let design = opened.design.as_ref().expect("a warning keeps the design");
    let knob = common::hardware(design, "handle");
    let nodes = &knob.model.as_ref().expect("the kind has a model").nodes;
    let built = glb(nodes, RIGHT, Some(("knob", 100_000.0)), false);
    let path = folder.join("knob.glb");
    if std::env::var_os("STAVE_WRITE_FIXTURES").is_some() {
        fs::write(&path, &built).expect("the fixture is written");
    }
    assert!(
        fs::read(&path).expect("the fixture is there") == built,
        "{} is not what this helper builds: run with STAVE_WRITE_FIXTURES=1",
        path.display()
    );
}
