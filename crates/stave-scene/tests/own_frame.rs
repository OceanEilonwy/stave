//! Every node's mesh is in its own frame and placed by its rest
//! transform: placed, it is the mesh drawn in place, for every part of
//! every example, and parts alike in shape share one mesh.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use glam::Vec3;
use stave_geom::solid::Solid;
use stave_model::solved::SolvedDesign;
use stave_scene::{BuildOptions, Item, SceneGraph};

fn solve(folder: &Path) -> Option<SolvedDesign> {
    let modules = stave_modules::ModuleSet::load(folder).ok()?;
    let (defs, _) = stave_model::resolve(&modules);
    let (model, _) = stave_model::instance::instantiate(defs, modules.root);
    stave_solve::solve(&model).0
}

fn examples() -> Vec<(String, SolvedDesign)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut folders: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_dir())
        .collect();
    folders.sort();
    // An example that doesn't solve on its own here has no scene to test.
    let solved: Vec<_> = folders
        .into_iter()
        .filter_map(|folder| {
            let name = folder.file_name()?.to_string_lossy().into_owned();
            Some((name, solve(&folder)?))
        })
        .collect();
    assert!(solved.len() >= 8, "most examples solve");
    solved
}

/// Whether every edge of a mesh (its corners welded to a thousandth of a
/// millimetre) is shared by a triangle on either side.
fn closed(mesh: &stave_geom::Mesh) -> bool {
    let corner = |index: u32| {
        let position = mesh.vertices[index as usize].position;
        position.map(|value| (value * 1000.0).round() as i64)
    };
    let mut edges: HashMap<([i64; 3], [i64; 3]), i32> = HashMap::new();
    for triangle in mesh.indices.chunks(3) {
        for (a, b) in [
            (triangle[0], triangle[1]),
            (triangle[1], triangle[2]),
            (triangle[2], triangle[0]),
        ] {
            let (a, b) = (corner(a), corner(b));
            if a != b {
                *edges.entry((a, b)).or_default() += 1;
                *edges.entry((b, a)).or_default() -= 1;
            }
        }
    }
    edges.values().all(|count| *count == 0)
}

#[test]
fn every_part_placed_at_rest_is_its_cut_mesh_in_place() {
    for (name, design) in examples() {
        let graph = SceneGraph::build(&design, BuildOptions::default());
        for node in &graph.nodes {
            let Item::Part(id) = node.item else {
                // Hardware and joint members are drawn where they are.
                assert_eq!(node.rest, glam::Mat4::IDENTITY);
                continue;
            };
            let part = design.part(id).unwrap();
            assert!(
                node.rest.determinant() > 0.0,
                "{name}: {} reflects",
                node.name
            );
            let world = stave_geom::cut_part(part, &Solid::of(part, &design));
            // The same vertices; the triangles may differ by a sliver
            // the mesher drops in one frame and not the other (it tests
            // for slivers in floats, which round differently far from the
            // origin), so they are compared by the volume they enclose.
            assert_eq!(
                node.local.mesh.vertices.len(),
                world.mesh.vertices.len(),
                "{name}: {}",
                node.name
            );
            if closed(&world.mesh) {
                assert!(closed(&node.local.mesh), "{name}: {} is open", node.name);
                let (local_volume, world_volume) =
                    (node.local.mesh.signed_volume(), world.mesh.signed_volume());
                // Where a hole meets a faceted round surface the patch
                // between is not flat, and two triangulations of it
                // enclose a little more or less: a thousandth is far
                // inside the facets' own error.
                assert!(
                    (local_volume - world_volume).abs() <= world_volume.abs() * 1e-3 + 1e-2,
                    "{name}: {} encloses {local_volume} mm³ in its own frame, \
                     {world_volume} mm³ in place",
                    node.name
                );
            } else {
                // An open mesh encloses no volume to compare (a few
                // joints' parts are drawn open either way).
                let (local, world) = (
                    node.local.mesh.triangle_count(),
                    world.mesh.triangle_count(),
                );
                assert!(local.abs_diff(world) * 50 <= world, "{name}: {}", node.name);
            }
            for (local, world) in node.local.mesh.vertices.iter().zip(&world.mesh.vertices) {
                let placed = node.rest.transform_point3(Vec3::from(local.position));
                assert!(
                    (placed - Vec3::from(world.position)).abs().max_element() < 1e-3,
                    "{name}: {} at {placed} is drawn at {:?}",
                    node.name,
                    world.position
                );
                let normal = node.rest.transform_vector3(Vec3::from(local.normal));
                assert!((normal - Vec3::from(world.normal)).length() < 1e-4);
                // The grain coordinates are the part's own, in either
                // frame; some are measured back from a point, so they
                // round a little differently.
                assert!(
                    (Vec3::from(local.grain) - Vec3::from(world.grain))
                        .abs()
                        .max_element()
                        < 1e-3
                );
            }
            let bounds = node.bounds();
            assert!(
                (bounds.min - world.bounds.min).abs().max_element() < 1e-3
                    && (bounds.max - world.bounds.max).abs().max_element() < 1e-3,
                "{name}: {}",
                node.name
            );
        }
    }
}

#[test]
fn parts_alike_in_shape_share_one_mesh() {
    for (name, design) in examples() {
        let graph = SceneGraph::build(&design, BuildOptions::default());
        let parts: Vec<_> = graph
            .nodes
            .iter()
            .filter(|node| matches!(node.item, Item::Part(_)))
            .collect();
        let distinct: HashSet<*const stave_geom::Geometry> =
            parts.iter().map(|node| Arc::as_ptr(&node.local)).collect();
        // A mesh made once per distinct shape: no two shared meshes
        // are alike.
        let mut seen: Vec<&stave_geom::Geometry> = Vec::new();
        for node in &parts {
            if seen
                .iter()
                .any(|mesh| std::ptr::eq(*mesh, Arc::as_ptr(&node.local)))
            {
                continue;
            }
            assert!(
                !seen.iter().any(|mesh| **mesh == *node.local),
                "{name}: {} has a mesh alike another's but not shared",
                node.name
            );
            seen.push(&node.local);
        }
        if name == "clothing_cabinet" {
            assert!(
                distinct.len() < parts.len(),
                "{name}: {} meshes for {} parts",
                distinct.len(),
                parts.len()
            );
        }
    }
}
