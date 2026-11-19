//! Two projects started from a box, the way the editor starts one: the root
//! is `region envelope = box(...)` and every piece is an assembly of a
//! `std:casework` component.
//!
//! - `examples/clothing_cabinet_from_box` must make the same cabinet as the
//!   hand-written `examples/clothing_cabinet` (read here, not changed): the
//!   same parts, compared by stock and cut size with names ignored, and the
//!   same joints by kind and count. Its envelope is the whole piece, so its
//!   overall sizes are the hand-written cabinet's plus what the case puts
//!   inside the box: the top's 20mm overhang at each end and at the front
//!   (940mm wide, 580mm deep) and the feet's 80mm under the base, given as
//!   the case's `raise` (1780mm high).
//! - `examples/chest_of_drawers` splits its drawer space with `shares:`, and
//!   the four bays come out in exactly that ratio.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use stave_app::Opened;
use stave_lang::placement::Axis;
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

/// Panics unless the root is the region `envelope` and every part belongs
/// to an assembly.
fn assert_built_from_a_box(design: &SolvedDesign) {
    let roots: Vec<&str> = design
        .regions()
        .iter()
        .filter(|region| region.root)
        .map(|region| region.name.as_str())
        .collect();
    assert_eq!(roots, ["envelope"]);
    let loose: Vec<&str> = design
        .parts()
        .iter()
        .filter(|part| part.assembly.is_none())
        .map(|part| part.name.as_str())
        .collect();
    assert!(loose.is_empty(), "parts outside an assembly: {loose:?}");
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
/// makes, so one declaration joining several parts counts each pair.
fn joints(design: &SolvedDesign) -> BTreeMap<String, usize> {
    let mut joints = BTreeMap::new();
    for joint in design.joints() {
        *joints.entry(joint.kind.clone()).or_insert(0) += joint.pairs.len();
    }
    joints
}

#[test]
fn the_cabinet_from_a_box_checks_clean() {
    let opened = clean("clothing_cabinet_from_box");
    assert_built_from_a_box(solved(&opened));
}

#[test]
fn the_chest_of_drawers_checks_clean() {
    let opened = clean("chest_of_drawers");
    assert_built_from_a_box(solved(&opened));
}

#[test]
fn the_cabinet_from_a_box_has_the_same_parts_as_the_hand_written_one() {
    let written = clean("clothing_cabinet");
    let from_box = clean("clothing_cabinet_from_box");
    let written = parts(solved(&written));
    let from_box = parts(solved(&from_box));
    assert_eq!(written.len(), 34);
    assert_eq!(written, from_box);
}

#[test]
fn the_cabinet_from_a_box_has_the_same_joints_as_the_hand_written_one() {
    let written = clean("clothing_cabinet");
    let from_box = clean("clothing_cabinet_from_box");
    let written = joints(solved(&written));
    let from_box = joints(solved(&from_box));
    for kind in ["dado", "rabbet", "socket", "round_tenon", "dowel"] {
        assert!(written.contains_key(kind), "the cabinet has no {kind}");
    }
    assert_eq!(written, from_box);
}

#[test]
fn the_chests_drawer_bays_are_in_the_ratio_of_its_shares() {
    // The chest's split, bottom bay first.
    const SHARES: [i128; 4] = [5, 4, 3, 2];
    let text = std::fs::read_to_string(workspace().join("examples/chest_of_drawers/chest.stave"))
        .expect("the chest's text");
    assert!(
        text.contains("split(back.in_front, up, shares: [5, 4, 3, 2])"),
        "the chest's shares changed: update SHARES"
    );

    let opened = clean("chest_of_drawers");
    let design = solved(&opened);
    let region = |name: &str| {
        design
            .regions()
            .iter()
            .find(|region| region.name == name)
            .unwrap_or_else(|| panic!("no region {name}"))
    };
    let bays: Vec<_> = (0..SHARES.len())
        .map(|index| region(&format!("drawer_bay[{index}]")))
        .collect();
    let heights: Vec<i128> = bays
        .iter()
        .map(|bay| i128::from(bay.bounds.size(Axis::Up).nm()))
        .collect();

    // Exactly in proportion: every pair of heights is in the ratio of its
    // shares, with nothing rounded.
    for (i, (height_i, share_i)) in heights.iter().zip(SHARES).enumerate() {
        for (height_j, share_j) in heights.iter().zip(SHARES).skip(i + 1) {
            assert_eq!(height_i * share_j, height_j * share_i, "{heights:?}");
        }
    }
    // Taller toward the bottom, stacked from the bottom up with nothing
    // between them, filling the space in front of the back.
    assert!(
        heights.windows(2).all(|pair| pair[0] > pair[1]),
        "{heights:?}"
    );
    for pair in bays.windows(2) {
        assert_eq!(
            pair[0].bounds.max().get(Axis::Up),
            pair[1].bounds.min().get(Axis::Up)
        );
    }
    let space = region("back.in_front");
    assert_eq!(
        heights.iter().sum::<i128>(),
        i128::from(space.bounds.size(Axis::Up).nm())
    );
    assert_eq!(heights[0], i128::from(Length::from_mm(315).nm()));
}
