//! The design corpus under `tests/designs/`: small whole projects that
//! show one feature working, end to end. Every one checks with no
//! diagnostic at all, and the tests below say what each one shows.

mod common;

use std::path::{Path, PathBuf};

use common::{mm, part};
use stave_app::Opened;
use stave_geom::solid::Solid;
use stave_lang::render_diagnostics;
use stave_lang::units::Length;
use stave_model::solved::SolvedDesign;
use stave_schema::CutOperation;

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Opens a design of the corpus; panics unless it checks with no
/// diagnostic.
fn clean(name: &str) -> stave_app::Opened {
    let opened = stave_app::open(&workspace().join("tests/designs").join(name)).unwrap();
    assert!(
        opened.diagnostics.is_empty(),
        "{name}:\n{}",
        render_diagnostics(&opened.sources, &opened.diagnostics)
    );
    opened
}

fn solved(opened: &stave_app::Opened) -> &SolvedDesign {
    opened.design.as_ref().expect("a solved design")
}

#[test]
fn every_design_checks_with_no_diagnostic() {
    let mut checked = 0;
    let mut folders: Vec<PathBuf> = std::fs::read_dir(workspace().join("tests/designs"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_dir())
        .collect();
    folders.sort();
    for folder in folders {
        let name = folder.file_name().unwrap().to_string_lossy().into_owned();
        clean(&name);
        checked += 1;
    }
    assert!(checked >= 1, "no designs under tests/designs");
}

#[test]
fn unglued_top_is_three_boards_in_tongue_and_groove_held_by_battens() {
    let opened = clean("unglued_top");
    let design = solved(&opened);
    let top = part(design, "top");
    let boards = top.boards.as_ref().expect("the top is made of boards");
    // 420mm across the grain from boards 150mm wide at most: three of 140mm,
    // the first two cut wider by the tongue (19 / 3 mm).
    let widths: Vec<_> = boards.boards.iter().map(|b| b.width).collect();
    assert_eq!(widths, [mm("146.333333"), mm("146.333333"), mm("140")]);
    assert_eq!(boards.seams.len(), 2);
    let methods: Vec<_> = boards
        .methods
        .iter()
        .map(|m| (m.name.as_str(), m.glue))
        .collect();
    assert_eq!(methods, [("tongue_and_groove", false)]);
    // Nothing in the panel is glued: not the seams, not the battens.
    let screws = design
        .joints()
        .iter()
        .find(|j| j.name == "batten_screws")
        .unwrap();
    assert!(!screws.glue);
    // The tongue and groove makes no part and uses no hardware.
    assert!(design.made_parts().next().is_none());
    assert_eq!(design.fasteners.len(), 1);
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

/// The cut operations on each board of a part, board by board.
fn board_operations(design: &SolvedDesign, name: &str) -> Vec<Vec<CutOperation>> {
    let boards = part(design, name).boards.as_ref().expect("made of boards");
    boards
        .boards
        .iter()
        .map(|board| board.cuts.iter().map(|c| c.kind.operation()).collect())
        .collect()
}

/// Asserts that a part's cut mesh is watertight and that its solid keeps
/// `prisms` angled cuts with a slanted side.
fn watertight(design: &SolvedDesign, name: &str, prisms: usize) {
    let part = part(design, name);
    let solid = Solid::of(part, design);
    assert_eq!(solid.prisms().len(), prisms, "{name}");
    assert!(solid.misshapen().is_empty(), "{name}");
    let mesh = stave_geom::cut_part(part, &solid).mesh;
    assert_eq!(open_edges(&mesh), 0, "{name} has open edges");
}

fn ranges(values: [(&str, &str); 3]) -> [(Length, Length); 3] {
    common::ranges(values)
}

#[test]
fn chigiri_top_is_three_boards_held_by_butterfly_keys() {
    let opened = clean("chigiri_top");
    let design = solved(&opened);
    let boards = part(design, "top").boards.as_ref().unwrap();
    let widths: Vec<_> = boards.boards.iter().map(|b| b.width).collect();
    assert_eq!(widths, [mm("140"), mm("140"), mm("140")]);
    let methods: Vec<_> = boards
        .methods
        .iter()
        .map(|m| (m.name.as_str(), m.glue))
        .collect();
    assert_eq!(methods, [("chigiri", false)]);
    // Each board has half a socket at every seam it is on, cut for real.
    use CutOperation::Outline;
    assert_eq!(
        board_operations(design, "top"),
        [vec![Outline], vec![Outline, Outline], vec![Outline]]
    );
    // One key per seam: a block with a vee cut from each long side, flush
    // with the top face, centred on its seam (140mm and 280mm from the
    // reference edge) and on the top's length.
    let made: Vec<&str> = design.made_parts().map(|p| p.name.as_str()).collect();
    assert_eq!(made, ["top.key[1]", "top.key[2]"]);
    for (name, deep) in [
        ("top.key[1]", ("102", "178")),
        ("top.key[2]", ("242", "318")),
    ] {
        let key = part(design, name);
        assert_eq!(
            (key.cut.length, key.cut.width, key.cut.thickness),
            (mm("76"), mm("38"), mm("9.5"))
        );
        assert_eq!(
            common::extent(key),
            ranges([("281", "319"), ("9.5", "19"), deep])
        );
        assert_eq!(key.cuts.len(), 2);
        watertight(design, name, 2);
    }
    // The four half sockets are the top's angled cuts.
    watertight(design, "top", 4);
}

#[test]
fn batten_top_is_held_flat_by_sliding_dovetail_battens() {
    let opened = clean("batten_top");
    let design = solved(&opened);
    let boards = part(design, "top").boards.as_ref().unwrap();
    let methods: Vec<_> = boards
        .methods
        .iter()
        .map(|m| (m.name.as_str(), m.glue))
        .collect();
    assert_eq!(
        methods,
        [("tongue_and_groove", false), ("suitsuki_zan", false)]
    );
    // Every board has a housing for each batten, after its tongue and
    // groove.
    use CutOperation::{DovetailSocket, Slot, Tenon};
    assert_eq!(
        board_operations(design, "top"),
        [
            vec![Tenon, DovetailSocket, DovetailSocket],
            vec![Slot, Tenon, DovetailSocket, DovetailSocket],
            vec![Slot, DovetailSocket, DovetailSocket],
        ]
    );
    // Two battens as long as the top is wide, 100mm in from each end,
    // sunk into their housings by a quarter of the top's thickness.
    let made: Vec<&str> = design.made_parts().map(|p| p.name.as_str()).collect();
    assert_eq!(made, ["top.batten_start", "top.batten_end"]);
    for (name, across) in [
        ("top.batten_start", ("100", "157")),
        ("top.batten_end", ("443", "500")),
    ] {
        let batten = part(design, name);
        assert_eq!(batten.cut.length, mm("420"));
        assert_eq!(
            common::extent(batten),
            ranges([across, ("-20.25", "4.75"), ("0", "420")])
        );
        let operations: Vec<_> = batten.cuts.iter().map(|c| c.kind.operation()).collect();
        assert_eq!(operations, [CutOperation::DovetailTail]);
        // The tongue: a wedge cut away either side, the batten's length.
        watertight(design, name, 2);
    }
    // A housing in each of three boards for each of two battens.
    watertight(design, "top", 6);
}

/// A design of the corpus, its top joined by a copy of a standard joinery
/// kind with one change.
fn with_changed_kind(design: &str, kind: &str, renamed: &str, change: (&str, &str)) -> Opened {
    let read = |path: &str| std::fs::read_to_string(workspace().join(path)).unwrap();
    let top = read(&format!("tests/designs/{design}/top.stave"))
        .replace(&format!("{kind}("), &format!("{renamed}("));
    let source = read(&format!("std/joinery/{kind}.stave"));
    assert!(
        source.contains(change.0),
        "{kind} no longer has {}",
        change.0
    );
    let copy = source
        .replacen("stave 1\n", "", 1)
        .replace(&format!("kind {kind} "), &format!("kind {renamed} "))
        .replacen(change.0, change.1, 1);
    common::open_files(&[("top.stave", &format!("{top}\n{copy}"))])
}

#[test]
fn a_butterfly_key_wider_than_its_socket_overlaps_the_boards() {
    let opened = with_changed_kind(
        "chigiri_top",
        "chigiri",
        "wide_key",
        (
            "across: (key_width - waist) / 2)",
            "across: (key_width - waist) / 2 - 1mm)",
        ),
    );
    let all = common::diagnostics(&opened);
    assert_eq!(all, ["overlap: top and top.key[1] overlap"], "{all:?}");
}

#[test]
fn a_housing_narrower_than_its_batten_overlaps_it() {
    let opened = with_changed_kind(
        "batten_top",
        "suitsuki_zan",
        "tight_zan",
        (
            "dovetail_socket(neck: neck,",
            "dovetail_socket(neck: neck - 2mm,",
        ),
    );
    let all = common::diagnostics(&opened);
    assert_eq!(
        all,
        ["overlap: top and top.batten_start overlap"],
        "{all:?}"
    );
}

#[test]
fn shelf_on_pins_rests_on_them_with_a_row_of_spare_holes() {
    use stave_model::solved::CutPlace;
    // Clean: the shelf is held by its pins, with no joint and no
    // `loose: true`.
    let opened = clean("shelf_on_pins");
    let design = solved(&opened);
    assert!(part(design, "shelf").cuts.is_empty());
    for side in ["side_l", "side_r"] {
        let holes: Vec<_> = part(design, side)
            .cuts
            .iter()
            .filter(|cut| cut.name.as_ref().is_some_and(|n| n.as_str() == "pin_hole"))
            .collect();
        // Two pins, each with its own hole, three spare holes above and
        // two below.
        assert_eq!(holes.len(), 2 * 6, "{side}");
        assert!(
            holes
                .iter()
                .all(|cut| cut.kind.operation() == CutOperation::Drill)
        );
        let mut starts: Vec<Length> = holes
            .iter()
            .filter_map(|cut| match &cut.place {
                CutPlace::Member { shape, .. } => Some(shape.min.length),
                CutPlace::End { .. }
                | CutPlace::Side { .. }
                | CutPlace::Crossing { .. }
                | CutPlace::Edge { .. } => None,
            })
            .collect();
        starts.sort();
        starts.dedup();
        // The pin's hole has its top at the shelf's underside, 400mm up;
        // the spare holes are 32mm apart, centre to centre.
        let expected: Vec<Length> = [331, 363, 395, 427, 459, 491]
            .into_iter()
            .map(|at| mm(&at.to_string()))
            .collect();
        assert_eq!(starts, expected, "{side}");
    }
}

/// The cuts on a part made by one named cut of a kind.
fn named_cuts<'a>(
    design: &'a SolvedDesign,
    part_name: &str,
    cut_name: &str,
) -> Vec<&'a stave_model::solved::Cut> {
    part(design, part_name)
        .cuts
        .iter()
        .filter(|cut| cut.name.as_ref().is_some_and(|n| n.as_str() == cut_name))
        .collect()
}

#[test]
fn a_top_on_tabletop_fasteners_is_held_by_them_with_no_joint() {
    use stave_model::solved::HardwareBase;
    let opened = clean("top_on_fasteners");
    let design = solved(&opened);
    // Six fasteners, all of the neutral base: nothing in the core knows
    // what a tabletop fastener is.
    assert_eq!(design.hardware().len(), 6);
    assert!(
        design
            .hardware()
            .iter()
            .all(|hardware| hardware.base == HardwareBase::Fitting)
    );
    // Each one is screwed up into the top and sits in a recess in its
    // apron's top edge, screwed down into it.
    assert_eq!(named_cuts(design, "top", "top_pilot_hole").len(), 6);
    for apron in ["apron_front", "apron_back"] {
        assert_eq!(named_cuts(design, apron, "recess").len(), 3, "{apron}");
        assert_eq!(
            named_cuts(design, apron, "apron_pilot_hole").len(),
            3,
            "{apron}"
        );
    }
    // The short aprons have none.
    assert!(named_cuts(design, "apron_left", "recess").is_empty());
}

#[test]
fn a_knock_down_cabinet_is_held_together_by_cam_locks() {
    let opened = clean("knock_down_cabinet");
    let design = solved(&opened);
    for panel in ["bottom", "top"] {
        // Two cams at each end: a bore in the face and a hole into the
        // end for each bolt.
        assert_eq!(named_cuts(design, panel, "cam_bore").len(), 4, "{panel}");
        let bolt_holes = named_cuts(design, panel, "bolt_hole");
        assert_eq!(bolt_holes.len(), 4, "{panel}");
        assert!(
            bolt_holes
                .iter()
                .all(|cut| cut.kind.operation() == CutOperation::Drill)
        );
    }
    // Each side takes the threads of the bolts of both panels: the cam is
    // set in from whichever end of the panel meets it.
    for side in ["side_l", "side_r"] {
        assert_eq!(named_cuts(design, side, "thread_hole").len(), 4, "{side}");
    }
}
