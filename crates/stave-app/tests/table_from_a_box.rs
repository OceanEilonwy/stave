//! `examples/side_table_from_box` is the side table of
//! `examples/side_table` started from a box and built from `std:frames`
//! components: the box is the root, every piece is an assembly, and the
//! back and right aprons, the back cleat and the back stretcher are mirror
//! images. It must make the same table as the hand-written one: the same
//! parts, compared by stock and cut size with names ignored, and the same
//! joints by kind and count, with the same fasteners to buy and the same
//! joint members (what each is and how many copies).
//!
//! This reads `examples/side_table`, which another lane may own; it only
//! reads it.

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
        .parts
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

/// The joints by kind: how many connections (pairs of parts that meet)
/// each kind makes. The hand-written table tenons all four aprons in one
/// declaration; the assemblies make one per apron. So pairs are counted,
/// not declarations.
fn joints(design: &SolvedDesign) -> BTreeMap<String, usize> {
    let mut joints = BTreeMap::new();
    for joint in &design.joints {
        *joints.entry(joint.kind.clone()).or_insert(0) += joint.pairs.len();
    }
    joints
}

/// The fasteners to buy, totalled by kind and size over every joint.
fn fasteners(design: &SolvedDesign) -> BTreeMap<(String, Length, Length), u32> {
    let mut totals = BTreeMap::new();
    for fastener in &design.fasteners {
        *totals
            .entry((fastener.kind.clone(), fastener.diameter, fastener.length))
            .or_insert(0) += fastener.count;
    }
    totals
}

/// The joints' members, names left out: by joint kind, member name and
/// what the member is, how many copies there are in all.
fn members(design: &SolvedDesign) -> BTreeMap<(String, String, Option<String>), u32> {
    let mut members = BTreeMap::new();
    for joint in &design.joints {
        for pair in &joint.pairs {
            for member in &pair.members {
                let item = member.item.as_ref().map(|item| item.kind.clone());
                *members
                    .entry((joint.kind.clone(), member.name.clone(), item))
                    .or_insert(0) += member.copies();
            }
        }
    }
    members
}

#[test]
fn the_table_from_a_box_checks_clean() {
    let opened = clean("side_table_from_box");
    let design = solved(&opened);
    // Every piece is in an assembly; nothing is a loose part.
    let loose: Vec<&str> = design
        .parts
        .iter()
        .filter(|part| part.assembly.is_none())
        .map(|part| part.name.as_str())
        .collect();
    assert_eq!(loose, Vec::<&str>::new());
    // The mirror images are assemblies of their own.
    for name in ["apron_back", "apron_right", "cleat_back", "stretcher_back"] {
        assert!(
            design
                .assemblies
                .iter()
                .any(|assembly| assembly.name == name),
            "no assembly {name}"
        );
    }
}

#[test]
fn the_table_from_a_box_has_the_same_parts_as_the_hand_written_one() {
    let written = clean("side_table");
    let assembled = clean("side_table_from_box");
    let written = parts(solved(&written));
    let assembled = parts(solved(&assembled));
    // A top, four legs, four aprons, two cleats, two stretchers, five slats.
    assert_eq!(written.len(), 18);
    assert_eq!(written, assembled);
}

#[test]
fn the_table_from_a_box_has_the_same_joints_as_the_hand_written_one() {
    let written = clean("side_table");
    let assembled = clean("side_table_from_box");
    let written = joints(solved(&written));
    let assembled = joints(solved(&assembled));
    for kind in ["mortise_tenon", "screw", "glued_face"] {
        assert!(written.contains_key(kind), "the table has no {kind}");
    }
    assert_eq!(written, assembled);
}

/// The back legs' mortises come from the mirrored aprons and stretcher: a
/// joint inside a mirrored assembly cuts the outside parts it is re-bound
/// to, as its source cuts the front legs.
#[test]
fn the_mirrored_rails_cut_their_mortises_in_the_back_legs() {
    let opened = clean("side_table_from_box");
    let design = solved(&opened);
    let cuts = |name: &str| {
        design
            .parts
            .iter()
            .find(|part| part.name == name)
            .map(|part| part.cuts.len())
            .unwrap_or_else(|| panic!("no part {name}"))
    };
    // Two aprons and a stretcher meet each leg.
    for leg in ["front_left", "front_right", "back_left", "back_right"] {
        assert_eq!(cuts(&format!("legs.leg[{leg}]")), 3, "legs.leg[{leg}]");
    }
}

/// The mirrored cleat's screws are bought too, and its joint's member says
/// what it is and how many: a mirrored assembly's joint keeps its source's
/// fasteners.
#[test]
fn the_table_from_a_box_buys_the_same_fasteners_as_the_hand_written_one() {
    let written = clean("side_table");
    let assembled = clean("side_table_from_box");
    let written_fasteners = fasteners(solved(&written));
    assert_eq!(
        written_fasteners.values().sum::<u32>(),
        26,
        "six top screws and twenty slat screws"
    );
    assert_eq!(written_fasteners, fasteners(solved(&assembled)));
    assert_eq!(members(solved(&written)), members(solved(&assembled)));
}
