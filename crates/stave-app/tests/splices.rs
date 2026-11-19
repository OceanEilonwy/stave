//! Splices (grammar, "Splices") and stock lengths (checks gaps review,
//! stage 5): a part longer than its stock names where to splice it, and
//! becomes pieces joined end to end by an ordinary joint kind, checked,
//! drawn, focused and pulled apart like any other parts and joint.

mod common;

use std::path::{Path, PathBuf};

use common::{HEADER, diagnostics, mm, open_files_with_every_warning, part};
use stave_app::focus::{self, Options};
use stave_app::view::{self, SceneGraph};
use stave_lang::render_diagnostics;
use stave_model::solved::{JointOrigin, PartOrigin, SolvedDesign};

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn opened(folder: &str) -> stave_app::Opened {
    let opened = stave_app::open(&workspace().join(folder)).unwrap();
    assert!(
        opened.diagnostics.is_empty(),
        "{}",
        render_diagnostics(&opened.sources, &opened.diagnostics)
    );
    opened
}

fn rendered(folder: &str) -> String {
    let opened = stave_app::open(&workspace().join(folder)).unwrap();
    render_diagnostics(&opened.sources, &opened.diagnostics)
        .replace(&format!("{}/", workspace().display()), "")
}

fn design(opened: &stave_app::Opened) -> &SolvedDesign {
    opened.design.as_ref().expect("it solves")
}

#[test]
fn a_spliced_seat_is_two_pieces_and_a_lap_splice() {
    let opened = opened("tests/designs/spliced_bench");
    let design = design(&opened);
    let first = part(design, "seat.piece[1]");
    let second = part(design, "seat.piece[2]");
    // 1100mm and 2100mm drawn, each with half the 300mm lap past the
    // splice: both within a 2700mm board.
    assert_eq!(first.drawn.length, mm("1100"));
    assert_eq!(first.cut.length, mm("1250"));
    assert_eq!(second.drawn.length, mm("2100"));
    assert_eq!(second.cut.length, mm("2250"));
    assert!(matches!(second.origin, PartOrigin::Piece { part, .. } if part == first.id));
    let pieces = first.pieces.as_ref().expect("the first piece lists them");
    assert_eq!(pieces.name, "seat");
    assert_eq!(pieces.parts.len(), 2);
    let splice = design
        .joints()
        .iter()
        .find(|j| j.name == "seat.splice[1]")
        .unwrap();
    assert_eq!(splice.kind, "lap_splice");
    assert!(splice.glue);
    assert!(matches!(splice.origin, JointOrigin::Splice { part, .. } if part == first.id));
    assert_eq!(pieces.splices, [splice.id]);
    let roles: Vec<_> = splice.pairs[0].roles.iter().map(|r| r.part).collect();
    assert_eq!(roles, [first.id, second.id]);
    // Each piece is cut with one lap, on opposite faces.
    let laps = |p: &stave_model::solved::Part| {
        p.cuts
            .iter()
            .filter(|c| c.name.as_ref().is_some_and(|n| n.as_str().contains("lap")))
            .count()
    };
    assert_eq!((laps(first), laps(second)), (1, 1));
    // The legs' dowels land on the piece above each leg.
    let dowels = design
        .joints()
        .iter()
        .find(|j| j.name == "leg_dowels")
        .unwrap();
    let into: Vec<(&str, &str)> = dowels
        .pairs
        .iter()
        .map(|pair| {
            let name = |i: usize| design.part(pair.roles[i].part).unwrap().name.as_str();
            (name(0), name(1))
        })
        .collect();
    assert_eq!(
        into,
        [
            ("leg_front_left", "seat.piece[1]"),
            ("leg_back_left", "seat.piece[1]"),
            ("leg_front_middle", "seat.piece[2]"),
            ("leg_back_middle", "seat.piece[2]"),
            ("leg_front_right", "seat.piece[2]"),
            ("leg_back_right", "seat.piece[2]"),
        ]
    );
    assert_eq!(first.joints, [dowels.id, splice.id]);
    assert_eq!(second.joints, [dowels.id, splice.id]);
    // The holes went with them: four in the first piece, eight in the
    // second, besides the laps.
    assert_eq!(first.cuts.len(), 5);
    assert_eq!(second.cuts.len(), 9);
}

#[test]
fn the_parts_tree_lists_every_piece() {
    let opened = opened("tests/designs/spliced_bench");
    let tree = stave_app::inspect::parts_tree(design(&opened));
    let names = format!("{tree:?}");
    assert!(names.contains("seat.piece[1]"), "{names}");
    assert!(names.contains("seat.piece[2]"), "{names}");
}

#[test]
fn a_kanawa_tsugi_splice_makes_its_pin_and_plays_its_steps_in_focus() {
    let opened = opened("tests/designs/spliced_beam");
    let design = design(&opened);
    let pin = part(design, "beam.splice[1].pin");
    assert!(matches!(pin.origin, PartOrigin::Made { .. }));
    let graph: SceneGraph = view::scene(design);
    let target = focus::find(design, "beam.splice[1]", None).unwrap();
    let found = focus::open(&opened, design, &graph, target).unwrap();
    assert_eq!(found.heading, "Kanawa tsugi");
    // Lay the piece down, slide it home, drive the pin: the kind's own
    // three steps.
    assert_eq!(found.step_count(), 3);
    let rest = graph.scenes.first().unwrap();
    let frame = found.frame(&graph, rest, 0.9, &Options::default());
    let step = frame.step.unwrap_or_default();
    assert!(
        step.starts_with("Step 3 of 3 · drive(beam.splice[1].pin"),
        "{step}"
    );
}

#[test]
fn pulled_apart_the_pieces_of_a_splice_come_apart() {
    // The lap splice's step lifts its first piece off the second.
    let opened = opened("tests/designs/spliced_bench");
    let graph = view::scene(design(&opened));
    let offset = |name: &str| {
        let node = graph
            .node_ids()
            .find(|id| graph.node(*id).is_some_and(|n| n.name == name))
            .unwrap_or_else(|| panic!("no node '{name}'"));
        graph.explode_offset(node)
    };
    let (first, second) = (offset("seat.piece[1]"), offset("seat.piece[2]"));
    assert!((first - second).length() > 10.0, "{first} {second}");
}

#[test]
fn a_mirror_image_is_spliced_at_the_same_distance_from_its_mirrored_end() {
    let body = "stock long { species: pine, form: solid, thickness: 19mm, max_length: 1000mm }
part rail : long {
  length:  1500mm
  width:   40mm
  lengths: spliced(by: lap_splice, at: 600mm, from: left)
  place {
    against(base.top)
    flush(base.left, inset: 400mm)
    flush(base.front)
  }
}
part rail_b = mirror(rail, about: mid(base, across))
joint rail_glue : glued_face([rail, rail_b], into: base)";
    let opened = common::open(body);
    assert!(
        diagnostics(&opened).is_empty(),
        "{:?}",
        diagnostics(&opened)
    );
    let design = common::design(&opened);
    let a = part(design, "rail.piece[1]");
    let b = part(design, "rail_b.piece[1]");
    // The first piece of each is at its `from` end: rail's on the left,
    // the image's on the right, both 600mm long as drawn.
    assert_eq!(a.drawn.length, mm("600"));
    assert_eq!(b.drawn.length, mm("600"));
    assert_eq!(a.drawn_box().min().across, mm("400"));
    assert_eq!(b.drawn_box().max().across, mm("200"));
    assert!(design.part_named("rail_b.piece[2]").is_some());
}

#[test]
fn a_stock_without_max_length_is_warned_once() {
    let body = "part shelf : pine_19 {
  length: 500mm
  width:  200mm
  place {
    against(base.top)
    flush(base.left)
    flush(base.front)
  }
}
joint shelf_glue : glued_face(shelf, into: base)";
    let opened = open_files_with_every_warning(&[("project.stave", &format!("{HEADER}\n{body}"))]);
    assert_eq!(
        diagnostics(&opened),
        [
            "missing-field: pine_19 doesn't say how long its boards are, so Stave can't check part lengths"
        ]
    );
}

#[test]
fn snapshot_a_stock_without_max_length() {
    insta::assert_snapshot!(rendered("tests/broken/stock_without_max_length"));
}

#[test]
fn snapshot_a_side_longer_than_its_stock() {
    insta::assert_snapshot!(rendered("tests/broken/side_longer_than_its_stock"));
}

#[test]
fn snapshot_a_piece_longer_than_its_stock() {
    insta::assert_snapshot!(rendered("tests/broken/piece_longer_than_its_stock"));
}

#[test]
fn snapshot_a_dado_across_a_splice() {
    insta::assert_snapshot!(rendered("tests/broken/dado_across_a_splice"));
}

#[test]
fn snapshot_a_dado_beside_a_lap() {
    insta::assert_snapshot!(rendered("tests/broken/dado_beside_a_lap"));
}

#[test]
fn snapshot_a_splice_measured_from_a_side() {
    insta::assert_snapshot!(rendered("tests/broken/splice_from_a_side"));
}

#[test]
fn a_part_whose_grain_runs_across_it_is_not_spliced() {
    let body = "part panel : pine_19 {
  length:  3000mm
  width:   200mm
  grain:   width
  lengths: spliced(by: lap_splice, at: 1500mm, from: left)
  place {
    against(base.top)
    flush(base.left)
    flush(base.front)
  }
}
joint panel_glue : glued_face(panel, into: base)";
    let opened = open_files_with_every_warning(&[("project.stave", &format!("{HEADER}\n{body}"))]);
    assert_eq!(
        diagnostics(&opened),
        [
            "missing-field: pine_19 doesn't say how long its boards are, so Stave can't check part lengths",
            "type-mismatch: panel's grain runs across its width, so it can't be spliced along its length",
        ]
    );
}
