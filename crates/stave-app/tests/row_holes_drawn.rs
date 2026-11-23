//! A row of holes is drawn: the tapestry loom's uprights show every height
//! hole the pivot bolt can be moved to, not only the one it is in.

use std::path::Path;

use stave_geom::solid::Solid;
use stave_model::solved::CutKind;

#[test]
fn every_height_hole_in_the_loom_uprights_is_drawn() {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/tapestry_loom");
    let opened = stave_app::open(&folder).expect("opens");
    let design = opened.design.as_ref().expect("it solves");
    for name in ["upright_l", "upright_r"] {
        let upright = design.part_named(name).expect("an upright");
        let drills = upright
            .cuts
            .iter()
            .filter(|cut| matches!(cut.kind, CutKind::Drill { .. }))
            .count();
        assert_eq!(drills, 4, "{name}: the hole the bolt is in and three below");
        let solid = Solid::of(upright, design);
        assert_eq!(solid.holes().len(), 4, "{name}");
        let drilled = stave_geom::cut_part(upright, &solid);
        let mut plain = upright.clone();
        plain
            .cuts
            .retain(|cut| !matches!(cut.kind, CutKind::Drill { .. }));
        let whole = stave_geom::cut_part(&plain, &Solid::of(&plain, design));
        let removed = whole.mesh.signed_volume() - drilled.mesh.signed_volume();
        // Four 9mm holes through 45mm: each π·4.5²·45 ≈ 2863 mm³.
        let expected = 4.0 * std::f32::consts::PI * 4.5 * 4.5 * 45.0;
        assert!(
            (removed - expected).abs() < expected * 0.05,
            "{name}: the holes remove {removed} mm³, not about {expected} mm³"
        );
        // The stretcher's mortise splits the holes' squares on the grid;
        // the faces round them still meet without a crack.
        assert_eq!(open_edges(&drilled.mesh), 0, "{name} has open edges");
    }
}

/// How many edges of a mesh, by the positions of their ends, are used by
/// other than two triangles: none in a watertight mesh.
fn open_edges(mesh: &stave_geom::Mesh) -> usize {
    use std::collections::BTreeMap;
    let key = |p: glam::Vec3| p.to_array().map(|c| (c + 0.0).to_bits());
    let mut counts: BTreeMap<([u32; 3], [u32; 3]), usize> = BTreeMap::new();
    for [a, b, c] in mesh.triangles() {
        if (b - a).cross(c - a).length_squared() < 1e-12 {
            continue;
        }
        for (p, q) in [(a, b), (b, c), (c, a)] {
            let (p, q) = (key(p), key(q));
            let edge = if p <= q { (p, q) } else { (q, p) };
            *counts.entry(edge).or_default() += 1;
        }
    }
    counts.values().filter(|count| **count != 2).count()
}
