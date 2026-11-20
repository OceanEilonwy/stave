//! Executable spec: joint members ("Joint members"): a joint kind's
//! `member`s laid out `on:` a contact patch, the cuts they make, the
//! hardware list, the overlap check, and the rendered diagnostics of the
//! broken designs it catches (docs/reviews/2026-11-17-checks-gaps.md,
//! stage 2).

mod common;

use std::path::{Path, PathBuf};

use common::{design, diagnostics, mm, open, part};
use stave_geom::collide::{WorldRod, WorldSolid};
use stave_geom::solid::Solid;
use stave_lang::placement::Axis;
use stave_lang::render_diagnostics;
use stave_model::solved::{BodyOwner, CutKind, CutPlace, Depth, MemberShape, Part, SolvedDesign};

fn workspace() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

/// The rendered diagnostics of a folder, with paths relative to the
/// workspace.
fn rendered(folder: &str) -> String {
    let path = workspace().join(folder);
    let opened = stave_app::open(&path).unwrap();
    render_diagnostics(&opened.sources, &opened.diagnostics)
        .replace(&format!("{}/", workspace().display()), "")
}

/// A side standing on the left end of the base, dowelled to it.
const SIDE_ON_BASE: &str = "part side : pine_19 {
  length: 400mm
  width:  300mm
  place {
    orient(length: up, width: back)
    against(base.top)
    flush(base.left)
    flush(base.front)
  }
}
";

/// The holes drilled in a part, in world coordinates.
fn holes(design: &SolvedDesign, part: &Part) -> Vec<WorldRod> {
    WorldSolid::of_part(part, &Solid::of(part, design))
        .holes()
        .to_vec()
}

#[test]
fn a_dowel_lays_its_pins_out_evenly_on_the_patch() {
    let opened = open(&format!(
        "{SIDE_ON_BASE}joint side_dowels : dowel(side, into: base, count: 3)"
    ));
    let design = design(&opened);
    let pair = &design.joints()[0].pairs[0];
    let [pin] = pair.members.as_slice() else {
        panic!("one member: {:?}", pair.members);
    };
    assert_eq!(pin.name, "pin");
    assert!(matches!(
        pin.id.owner,
        BodyOwner::Joint {
            pair: stave_model::solved::PairIndex::FIRST,
            ..
        }
    ));
    // Three 8mm pins on the side's 19 x 300 end: equal gaps of
    // (300 - 24) / 4 = 69mm before, between and after them, centred across
    // the 19mm, half in each part (15mm either side of the base's top).
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "a test picks out the variant it checks, and any other is a failure or not what it counts"
    )]
    let centres: Vec<_> = pin
        .shapes()
        .iter()
        .map(|shape| match shape {
            MemberShape::Cylinder {
                axis,
                centre,
                diameter,
                start,
                end,
            } => {
                assert_eq!(*axis, Axis::Up);
                assert_eq!(*diameter, mm("8"));
                assert_eq!((*start, *end), (mm("4"), mm("34")));
                *centre
            }
            other => panic!("a cylinder: {other:?}"),
        })
        .collect();
    assert_eq!(
        centres,
        vec![
            [mm("9.5"), mm("73")],
            [mm("9.5"), mm("150")],
            [mm("9.5"), mm("227")],
        ]
    );
    // A pin is drawn from the solved design alone.
    let mesh = stave_geom::body(pin).mesh;
    assert!(mesh.triangle_count() > 0);
}

#[test]
fn a_members_cuts_are_one_hole_per_pin_in_each_part() {
    let opened = open(&format!(
        "{SIDE_ON_BASE}joint side_dowels : dowel(side, into: base, count: 3)"
    ));
    let design = design(&opened);
    for name in ["side", "base"] {
        let drilled: Vec<_> = part(design, name)
            .cuts
            .iter()
            .filter(|cut| matches!(cut.place, CutPlace::Member { .. }))
            .collect();
        assert_eq!(drilled.len(), 3, "{name}");
        for cut in drilled {
            assert_eq!(
                cut.kind,
                CutKind::Drill {
                    diameter: mm("8"),
                    depth: Depth::Length(mm("15")),
                    from_depth: stave_lang::units::Length::ZERO,
                    tilt: None,
                }
            );
        }
    }
    // The holes in both parts share their centres.
    let mut side: Vec<_> = holes(design, part(design, "side"))
        .iter()
        .map(|h| h.centre)
        .collect();
    let mut base: Vec<_> = holes(design, part(design, "base"))
        .iter()
        .map(|h| h.centre)
        .collect();
    side.sort();
    base.sort();
    assert_eq!(side, base);
}

#[test]
fn the_hardware_list_counts_every_pin_placed() {
    let opened = open(&format!(
        "{SIDE_ON_BASE}joint side_dowels : dowel(side, into: base, count: 3)"
    ));
    let design = design(&opened);
    let [fastener] = design.fasteners.as_slice() else {
        panic!("{:?}", design.fasteners);
    };
    assert_eq!(fastener.kind, "dowel_pin");
    assert_eq!(fastener.count, 3);
    assert_eq!(fastener.diameter, mm("8"));
    assert_eq!(fastener.length, mm("30"));
}

/// A peg joint whose kind is written here, with a layout to vary.
fn peg_kind(layout: &str) -> String {
    format!(
        "kind peg_joint : joint {{
  insert: [push(piece, along: length)]
  member peg {{
    on: contact(piece, into)
{layout}
    shape: cylinder(diameter: 10mm, depth: -12mm..12mm)
  }}
  cut peg_hole: crossing(piece, peg) {{ drill(diameter: 10mm, depth: 12mm) }}
  cut socket: crossing(into, peg) {{ drill(diameter: 10mm, depth: 12mm) }}
}}
{SIDE_ON_BASE}joint pegs : peg_joint(side, into: base)"
    )
}

/// The centres of the pegs along the patch's length (deep). A peg flush
/// with the end of the patch has a hole that breaks out of the board
/// (`error[wall]`), which is not what these tests are about.
#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "a test picks out the variant it checks, and any other is a failure or not what it counts"
)]
fn peg_centres(layout: &str) -> Vec<stave_lang::units::Length> {
    let opened = open(&peg_kind(layout));
    let design = common::solved(&opened);
    design.joints()[0].pairs[0].members[0]
        .shapes()
        .iter()
        .map(|shape| match shape {
            MemberShape::Cylinder { centre, .. } => centre[1],
            other => panic!("a cylinder: {other:?}"),
        })
        .collect()
}

#[test]
fn spacing_is_the_spacing_parts_use() {
    // One peg, centred on the 300mm patch, by default.
    assert_eq!(peg_centres(""), vec![mm("150")]);
    // `ends`: the first and last flush with the ends of the patch.
    assert_eq!(
        peg_centres("    count: 3\n    spaced: ends"),
        vec![mm("5"), mm("150"), mm("295")]
    );
    assert_eq!(
        peg_centres("    count: 2\n    spaced: ends(inset: 20mm)"),
        vec![mm("25"), mm("275")]
    );
    // `pitch(d)`: centre to centre from the start; `gap(d)`: between them.
    assert_eq!(
        peg_centres("    count: 3\n    spaced: pitch(100mm)"),
        vec![mm("5"), mm("105"), mm("205")]
    );
    assert_eq!(
        peg_centres("    count: 2\n    spaced: gap(30mm)"),
        vec![mm("5"), mm("45")]
    );
}

#[test]
fn members_spaced_past_the_end_of_the_patch_are_an_overlap() {
    let opened = open(&peg_kind("    count: 4\n    spaced: pitch(100mm)"));
    assert_eq!(
        diagnostics(&opened),
        vec!["overlap: pegs: its pegs run 10mm past the end of where its parts touch"]
    );
}

#[test]
fn a_joint_member_needs_on_and_has_no_with_or_mount() {
    let kind = |entries: &str| {
        format!(
            "kind peg_joint : joint {{
  member peg {{
{entries}
    shape: cylinder(diameter: 10mm, depth: -12mm..12mm)
  }}
}}"
        )
    };
    assert_eq!(
        diagnostics(&open(&kind(""))),
        vec!["missing-field: member 'peg' is missing 'on'"]
    );
    assert_eq!(
        diagnostics(&open(&kind(
            "    on: contact(piece, into)\n    with: piece"
        ))),
        vec!["unknown-field: member 'peg' has no field 'with'"]
    );
    assert_eq!(
        diagnostics(&open(&kind("    on: contact(piece, into)\n    spaced: 3"))),
        vec!["type-mismatch: 'spaced' needs a spacing, found a count"]
    );
    // A joint's member without a body is not a word for a viewer's list.
    assert_eq!(
        diagnostics(&open("kind peg_joint : joint {\n  member peg\n}")),
        vec!["missing-field: member 'peg' is missing 'shape'"]
    );
}

#[test]
fn only_a_joints_members_are_laid_out_or_name_a_fastener() {
    let pull = |member: &str| {
        format!(
            "kind small_knob : pull {{
  roles {{
    piece: Part
  }}
  mount {{
    face:   front
    across: open
    along:  open
  }}
{member}
}}"
        )
    };
    assert_eq!(
        diagnostics(&open(&pull(
            "  member knob {\n    with: piece\n    on: contact(piece, piece)\n    shape: cylinder(diameter: 30mm, depth: -20mm..0mm)\n  }"
        ))),
        vec!["unknown-field: member 'knob' has no field 'on'"]
    );
    assert_eq!(
        diagnostics(&open(&pull(
            "  member screw : wood_screw {\n    with: piece\n    shape: cylinder(diameter: 4mm, depth: 0mm..10mm)\n  }"
        ))),
        vec!["unknown-field: member 'screw' of a hardware kind can't name a fastener"]
    );
}

/// The reported clothing cabinet (docs/reviews/2026-11-17-checks-gaps.md),
/// with an overlap the rules accept: the base made twice the cabinet's
/// width and the top 4mm shorter than it, so the top, centred, covers
/// 17mm of each side's 19mm end. The holes the dowels make in the sides
/// and in the top share their centres, in world coordinates.
#[test]
fn the_cabinets_dowel_holes_line_up_where_the_top_covers_only_part_of_the_sides() {
    let folder = copy_example("clothing_cabinet");
    let carcass = folder.join("carcass.stave");
    let text = std::fs::read_to_string(&carcass).unwrap();
    let edited = text
        .replacen(
            "  length: cabinet_width\n",
            "  length: cabinet_width * 2\n",
            1,
        )
        .replacen(
            "  length: cabinet_width + 2 * overhang\n",
            "  length: cabinet_width * 2 - 4mm\n",
            1,
        );
    assert_ne!(edited, text);
    std::fs::write(&carcass, edited).unwrap();
    // The back of a cabinet twice as wide needs a sheet wider than 1220mm.
    let stock = folder.join("stock.stave");
    let text = std::fs::read_to_string(&stock).unwrap();
    let edited = text.replacen(
        "thickness: 6mm, max_width: 1220mm",
        "thickness: 6mm, max_width: 2440mm",
        1,
    );
    assert_ne!(edited, text);
    std::fs::write(&stock, edited).unwrap();
    let opened = stave_app::open(&folder).unwrap();
    let _ = std::fs::remove_dir_all(&folder);
    let design = design(&opened);
    let top = part(design, "top");
    let top_holes = holes(design, top);
    for side in ["side_l", "side_r"] {
        let side = part(design, side);
        // The side's holes in its top end.
        let side_holes: Vec<WorldRod> = holes(design, side)
            .into_iter()
            .filter(|hole| hole.axis == Axis::Up && hole.end == side.cut_box().max().up)
            .collect();
        assert_eq!(side_holes.len(), 4, "{}", side.name);
        for hole in &side_holes {
            let matching = top_holes
                .iter()
                .find(|other| other.axis == Axis::Up && other.centre == hole.centre);
            let Some(matching) = matching else {
                panic!(
                    "{}'s hole at {:?} has no hole in the top: {top_holes:?}",
                    side.name, hole.centre
                );
            };
            eprintln!(
                "{}: hole centred at across {} deep {}, up {}..{}; top's at across {} deep {}, up {}..{}",
                side.name,
                hole.centre[0],
                hole.centre[1],
                hole.start,
                hole.end,
                matching.centre[0],
                matching.centre[1],
                matching.start,
                matching.end
            );
            assert_eq!(matching.diameter, hole.diameter);
            // Each hole runs 15mm from the joint line, one each way.
            assert_eq!(hole.end, matching.start);
        }
    }
    // The top covers 17mm of side_l's end, from 2mm in: the pins are
    // centred on that strip, 10.5mm from the cabinet's left end.
    let side_l = holes(design, part(design, "side_l"));
    assert!(side_l.iter().any(|hole| hole.centre[0] == mm("10.5")));
}

#[test]
fn a_dowel_running_into_another_dowel_is_reported_at_the_joint() {
    insta::assert_snapshot!(rendered("tests/broken/dowels_cross_dowels"));
}

#[test]
fn a_dowel_running_into_a_tenon_is_reported_at_the_joint() {
    insta::assert_snapshot!(rendered("tests/broken/dowel_into_tenon"));
}

#[test]
fn a_peg_longer_than_its_hole_is_reported_at_the_joint() {
    insta::assert_snapshot!(rendered("tests/broken/pin_past_its_hole"));
}

/// A copy of an example in a fresh folder, to edit.
fn copy_example(example: &str) -> PathBuf {
    let target =
        std::env::temp_dir().join(format!("stave-members-{example}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&target);
    copy_dir(&workspace().join("examples").join(example), &target);
    target
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        let target = to.join(entry.file_name());
        if path.is_dir() {
            copy_dir(&path, &target);
        } else {
            std::fs::copy(&path, &target).unwrap();
        }
    }
}

#[test]
fn a_mirrored_assembly_mirrors_its_joints_members() {
    let opened = open(
        "component Post {
  part post : pine_19 {
    length: 100mm
    width:  60mm
    place {
      orient(length: up, width: back)
      against(base.top)
      flush(base.left, inset: 50mm)
      flush(base.front, inset: 20mm)
    }
  }
  joint post_dowels : dowel(post, into: base, count: 1)
}
assembly post_l : Post { }
assembly post_r = mirror(post_l, about: mid(base, across))",
    );
    let design = design(&opened);
    let centre = |joint: &str| {
        let joint = design.joints().iter().find(|j| j.name == joint).unwrap();
        let member = &joint.pairs[0].members[0];
        assert!(matches!(member.id.owner, BodyOwner::Joint { joint: id, .. } if id == joint.id));
        match member.shapes() {
            [MemberShape::Cylinder { centre, .. }] => *centre,
            other => panic!("one cylinder: {other:?}"),
        }
    };
    // The post on the left is 50mm in from the base's left end; its
    // mirror image 50mm in from the right end of the 600mm base.
    assert_eq!(centre("post_l.post_dowels"), [mm("59.5"), mm("50")]);
    assert_eq!(centre("post_r.post_dowels"), [mm("540.5"), mm("50")]);
    // Both posts' pins have a hole under them in the base.
    let mut base: Vec<_> = holes(design, part(design, "base"))
        .iter()
        .map(|hole| hole.centre)
        .collect();
    base.sort();
    assert_eq!(base, vec![[mm("59.5"), mm("50")], [mm("540.5"), mm("50")]]);
}
