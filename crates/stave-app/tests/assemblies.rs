//! `examples/clothing_cabinet_assemblies` is the clothing cabinet rebuilt
//! from `std:casework` components: only the base, the sides and the top are
//! hand-written parts, and the back, divider, shelf, hanging rail, feet,
//! drawers and doors are assemblies. It must make the same cabinet as
//! `examples/clothing_cabinet`: the same parts, compared by stock and cut
//! size with names ignored, and the same joints by kind and count.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use stave_app::Opened;
use stave_lang::render_diagnostics;
use stave_lang::units::Length;
use stave_model::solved::SolvedDesign;

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Opens an example; panics unless it checks with no diagnostic at all.
fn clean(name: &str) -> Opened {
    let opened = stave_app::open(&workspace().join("examples").join(name)).unwrap();
    assert!(
        opened.diagnostics.is_empty(),
        "{name}:\n{}",
        render_diagnostics(&opened.sources, &opened.diagnostics)
    );
    opened
}

fn solved(opened: &Opened) -> &SolvedDesign {
    opened.design.as_ref().expect("a solved design")
}

/// One part with its name left out: its stock, its cut length, width and
/// thickness, and how many cuts it receives.
type PartKey = (String, Length, Length, Length, usize);

/// Every part as a [`PartKey`], sorted, so two designs that differ only in
/// names compare equal.
fn parts(design: &SolvedDesign) -> Vec<PartKey> {
    let mut parts: Vec<PartKey> = design
        .parts()
        .iter()
        .map(|part| {
            (
                part.stock.name.clone(),
                part.cut.length,
                part.cut.width,
                part.cut.thickness,
                part.cuts.len(),
            )
        })
        .collect();
    parts.sort();
    parts
}

/// The joints by kind: how many connections (pairs of parts) each kind
/// makes. One declaration can join several parts (the hand-written
/// `shelf_dados` houses both the divider and the shelf), so pairs are
/// counted, not declarations.
fn joints(design: &SolvedDesign) -> BTreeMap<String, usize> {
    let mut joints = BTreeMap::new();
    for joint in design.joints() {
        *joints.entry(joint.kind.clone()).or_insert(0) += joint.pairs.len();
    }
    joints
}

#[test]
fn the_assemblies_cabinet_checks_clean() {
    let opened = clean("clothing_cabinet_assemblies");
    let design = solved(&opened);
    // Only the base, the sides and the top are outside an assembly.
    let mut loose: Vec<&str> = design
        .parts()
        .iter()
        .filter(|part| part.assembly.is_none())
        .map(|part| part.name.as_str())
        .collect();
    loose.sort_unstable();
    assert_eq!(loose, ["base", "side_l", "side_r", "top"]);
}

#[test]
fn the_assemblies_cabinet_has_the_same_parts_as_the_hand_written_one() {
    let written = clean("clothing_cabinet");
    let assembled = clean("clothing_cabinet_assemblies");
    let written = parts(solved(&written));
    let assembled = parts(solved(&assembled));
    assert_eq!(written.len(), 34);
    assert_eq!(written, assembled);
}

#[test]
fn the_assemblies_cabinet_has_the_same_joints_as_the_hand_written_one() {
    let written = clean("clothing_cabinet");
    let assembled = clean("clothing_cabinet_assemblies");
    let written = joints(solved(&written));
    let assembled = joints(solved(&assembled));
    for kind in ["dado", "rabbet", "socket", "round_tenon", "dowel"] {
        assert!(written.contains_key(kind), "the cabinet has no {kind}");
    }
    assert_eq!(written, assembled);
}

#[test]
fn a_selected_assembly_stands_for_everything_inside_it() {
    use stave_app::inspect::items_in;
    use stave_app::session::Selection;
    use stave_scene::Item;
    let opened = clean("clothing_cabinet_assemblies");
    let design = solved(&opened);
    let outer = design
        .assemblies()
        .iter()
        .find(|a| a.parent.is_none() && design.assemblies().iter().any(|b| b.parent == Some(a.id)))
        .or_else(|| design.assemblies().first())
        .expect("an assembly");
    let items = items_in(design, &Selection::Assembly(outer.id));
    assert!(!items.is_empty());
    // Everything listed is inside it at some depth; everything inside it is listed.
    let within = |mut assembly: Option<stave_model::solved::AssemblyId>| {
        while let Some(id) = assembly {
            if id == outer.id {
                return true;
            }
            assembly = design.assembly(id).and_then(|a| a.parent);
        }
        false
    };
    let expected: Vec<Item> = design
        .parts()
        .iter()
        .filter(|p| within(p.assembly))
        .map(|p| Item::Part(p.id))
        .chain(
            design
                .hardware()
                .iter()
                .filter(|h| within(h.assembly))
                .map(|h| Item::Hardware(h.id)),
        )
        .collect();
    assert_eq!(items, expected);
    let part = design.parts().first().expect("a part");
    assert_eq!(
        items_in(design, &Selection::Part(part.id)),
        vec![Item::Part(part.id)]
    );
}
