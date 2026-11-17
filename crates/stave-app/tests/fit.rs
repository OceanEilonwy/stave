//! Fit and geometry (layer 4) and scene poses: what a user sees for each
//! broken design of these checks, and what the examples' joints do once
//! their cuts are made.

mod common;

use std::path::Path;

use stave_app::Opened;
use stave_geom::collide::{self, WorldSolid};
use stave_geom::solid::Solid;
use stave_lang::render_diagnostics;

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

fn example(name: &str) -> Opened {
    stave_app::open(&workspace().join("examples").join(name)).unwrap()
}

#[test]
fn a_tenon_that_bottoms_out_in_its_mortise() {
    insta::assert_snapshot!(rendered("tests/broken/tenon_too_long"));
}

#[test]
fn a_shelf_thicker_than_its_dado() {
    insta::assert_snapshot!(rendered("tests/broken/shelf_misses_dado"));
}

#[test]
fn a_knob_in_the_way_of_another_part() {
    insta::assert_snapshot!(rendered("tests/broken/knob_hits_top"));
}

#[test]
fn a_joint_whose_parts_never_touch() {
    insta::assert_snapshot!(rendered("tests/broken/joint_parts_apart"));
}

#[test]
fn a_pitch_that_runs_past_the_end() {
    insta::assert_snapshot!(rendered("tests/broken/pitch_runs_past_end"));
}

#[test]
fn a_lid_that_hits_the_wall_in_a_scene() {
    insta::assert_snapshot!(rendered("tests/broken/scene_pose_clash"));
}

#[test]
fn a_drawer_too_wide_for_its_slides() {
    insta::assert_snapshot!(rendered("tests/broken/drawer_does_not_fit"));
}

#[test]
fn a_dovetail_tail_wider_than_its_socket_overlaps_it() {
    insta::assert_snapshot!(rendered("tests/broken/dovetail_too_wide"));
}

#[test]
fn an_outline_that_crosses_itself_is_a_type_mismatch() {
    insta::assert_snapshot!(rendered("tests/broken/outline_crosses_itself"));
}

/// The cut solids of two parts of a design, placed.
fn solids(opened: &Opened, a: &str, b: &str) -> (WorldSolid, WorldSolid) {
    let design = common::design(opened);
    let solid = |name: &str| {
        let part = common::part(design, name);
        WorldSolid::of_part(part, &Solid::of(part, design))
    };
    (solid(a), solid(b))
}

#[test]
fn every_tenon_and_shelf_of_the_examples_sits_in_its_housing() {
    // The tenons reach into the legs, the shelves into the sides' dados:
    // their cut boxes overlap, their cut solids only touch.
    for (folder, a, b) in [
        ("side_table", "apron_front", "leg[front_left]"),
        ("bookshelf", "shelf[0]", "side_l"),
        ("box", "bottom", "front"),
        ("stool", "leg[front_left]", "seat"),
        ("stool", "rung[front]", "leg[front_left]"),
    ] {
        let opened = example(folder);
        let design = common::design(&opened);
        let boxes = (
            common::part(design, a).cut_box(),
            common::part(design, b).cut_box(),
        );
        assert!(
            collide::boxes_overlap(&boxes.0, &boxes.1),
            "{folder}: {a} doesn't reach into {b}"
        );
        let (x, y) = solids(&opened, a, b);
        assert!(collide::overlaps(&x, &y).is_empty(), "{folder}: {a} in {b}");
        assert!(collide::touches(&x, &y), "{folder}: {a} doesn't touch {b}");
    }
}

#[test]
fn the_examples_check_clean_with_their_cuts_made() {
    for folder in [
        "bookshelf",
        "box",
        "clothing_cabinet",
        "nightstand",
        "side_table",
        "stool",
        "wall_shelf",
    ] {
        let opened = example(folder);
        assert!(
            opened.diagnostics.is_empty(),
            "{folder}:\n{}",
            render_diagnostics(&opened.sources, &opened.diagnostics)
        );
    }
}

#[test]
fn a_door_whose_hinge_edge_swings_into_a_wall() {
    insta::assert_snapshot!(rendered("tests/broken/door_hits_side"));
}

#[test]
fn a_door_too_far_out_for_its_cup_hinges() {
    insta::assert_snapshot!(rendered("tests/broken/hinge_overlay"));
}

/// `broken/hinge_overlay` with its doors overlaying the sides by
/// `overlay`, and `euro_hinge` replaced by `kind` when one is given (a
/// copy of it under another name, appended to the design).
fn overlay_design(overlay: &str, kind: Option<(&str, String)>) -> stave_app::Opened {
    let mut design =
        std::fs::read_to_string(workspace().join("tests/broken/hinge_overlay/project.stave"))
            .unwrap()
            .replace(
                "overlay(left: 9mm, right: 9mm)",
                &format!("overlay(left: {overlay}, right: {overlay})"),
            );
    if let Some((name, text)) = kind {
        design = design.replace(
            "euro_hinge(panel, on: hinge_side)",
            &format!("{name}(panel, on: hinge_side)"),
        ) + "\n"
            + &text;
    }
    common::open_files(&[("project.stave", &design)])
}

/// `euro_hinge`'s text from `std/hardware/hinges.stave`, renamed.
fn euro_hinge_as(name: &str) -> String {
    let library = std::fs::read_to_string(workspace().join("std/hardware/hinges.stave")).unwrap();
    let start = library.find("kind euro_hinge : hinge {").unwrap();
    let length = library[start..].find("\n}\n").unwrap() + 3;
    library[start..start + length].replace("kind euro_hinge", &format!("kind {name}"))
}

/// The help for `error[adjust]` is the general search: each value of the
/// kind's lists, one at a time, with the hinge placed again. It prints
/// only listed values that bring the arm within its 2mm, never the exact
/// amount that would (8.5mm, 5.5mm).
#[test]
fn the_help_only_offers_values_the_kind_allows() {
    let help = |opened: &stave_app::Opened| -> Vec<String> {
        opened
            .diagnostics
            .iter()
            .filter(|d| d.code == stave_lang::Code::Adjust)
            .filter_map(|d| d.help.clone())
            .collect()
    };
    // 9mm: the arm would sit 8.5mm off; only a 9mm plate brings it back.
    let nine = overlay_design("9mm", None);
    assert_eq!(
        help(&nine).first().map(String::as_str),
        Some("use plate_height: 9mm, from the kind's list")
    );
    // 12mm: 5.5mm off; plates of 3.5mm to 7.5mm would do, and 6mm is
    // the one the kind lists. No boring distance comes near.
    let twelve = overlay_design("12mm", None);
    assert_eq!(
        help(&twelve).first().map(String::as_str),
        Some("use plate_height: 6mm, from the kind's list")
    );
    // 14mm: 3.5mm off; a 3mm boring distance or a 1.5mm or 3mm plate.
    let fourteen = overlay_design("14mm", None);
    assert_eq!(
        help(&fourteen).first().map(String::as_str),
        Some(
            "use cup_inset: 3mm or plate_height: 1.5mm or plate_height: 3mm, from the kind's lists"
        )
    );
    // Taking the 6mm plate the help offered checks clean.
    let fixed = overlay_design(
        "12mm",
        Some((
            "tall_plate_hinge",
            euro_hinge_as("tall_plate_hinge").replace(
                "param plate_height = 0mm within",
                "param plate_height = 6mm within",
            ),
        )),
    );
    assert!(
        fixed.diagnostics.is_empty(),
        "{:?}",
        common::diagnostics(&fixed)
    );
}

/// The example cabinet's doors overlay its sides by 18mm, 0.5mm past the
/// hinge's nominal 17.5mm: the arm is set 0.5mm out on its plate (its
/// setting across is -0.5mm, away from the middle of the door), within its
/// 2mm, and the design checks clean at that setting, with the arm drawn
/// where the door needs it (the dump's `arm 20.6..38.5`).
#[test]
fn a_design_inside_the_adjustment_checks_at_the_setting_it_needs() {
    use stave_lang::units::Length;
    use stave_model::defs::FrameAxis;
    use stave_model::solved::Rules;
    let opened = stave_app::open(&workspace().join("examples/clothing_cabinet")).unwrap();
    assert!(
        opened.diagnostics.is_empty(),
        "{:?}",
        common::diagnostics(&opened)
    );
    let design = common::design(&opened);
    let model = opened.model.as_ref().unwrap();
    for name in [
        "door_l.hinges[lower]",
        "door_l.hinges[upper]",
        "door_r.hinges[lower]",
        "door_r.hinges[upper]",
    ] {
        let hinge = common::hardware(design, name);
        assert_eq!(hinge.rules, Rules::Hold, "{name}");
        let kind = model.defs.kind(hinge.kind_id).unwrap();
        let settings: Vec<(&str, FrameAxis, Length)> = hinge
            .settings
            .iter()
            .map(|setting| {
                (
                    kind.bodies[setting.body.body.index()].name.as_str(),
                    setting.axis,
                    setting.offset,
                )
            })
            .collect();
        assert_eq!(
            settings,
            [("arm", FrameAxis::Across, Length::from_nm(-500_000))],
            "{name}"
        );
    }
}

/// A member's own mount on an axis no `adjust` covers must put it exactly
/// where the kind draws it: without its `adjust`, `euro_hinge` takes only
/// the nominal 17.5mm overlay, and a plate mounted 1mm deeper than it is
/// drawn is off its place in depth.
#[test]
fn an_axis_without_adjust_must_be_exact() {
    let adjust = "  adjust arm on plate: across within 2mm\n";
    let fixed = euro_hinge_as("fixed_hinge");
    assert!(fixed.contains(adjust));
    let fixed = fixed.replace(adjust, "");
    // At the nominal overlay it fits.
    let nominal = overlay_design("17.5mm", Some(("fixed_hinge", fixed.clone())));
    assert!(
        nominal.diagnostics.is_empty(),
        "{:?}",
        common::diagnostics(&nominal)
    );
    // Half a millimetre more is off its place.
    let more = overlay_design("18mm", Some(("fixed_hinge", fixed)));
    assert!(
        common::diagnostics(&more).iter().any(|m| m
            == "adjust: door.hinges[bottom]: the plate would sit 0.5mm off its place across; it doesn't adjust that way"),
        "{:?}",
        common::diagnostics(&more)
    );
    // Depth has no adjust in the kind at all.
    let depth = "      depth:  from(face(on, toward: piece))\n";
    let deeper = euro_hinge_as("deep_plate_hinge");
    assert!(deeper.contains(depth));
    let deeper = deeper.replace(
        depth,
        "      depth:  from(face(on, toward: piece), inset: 1mm)\n",
    );
    let off = overlay_design("17.5mm", Some(("deep_plate_hinge", deeper)));
    assert!(
        common::diagnostics(&off).iter().any(|m| m
            == "adjust: door.hinges[bottom]: the plate would sit 1mm off its place in depth; it doesn't adjust that way"),
        "{:?}",
        common::diagnostics(&off)
    );
}

/// An `adjust` names two members with a body and an axis of the frame on
/// which exactly one of them has its own mount, once per member and axis
/// (grammar, "Adjustment").
#[test]
fn an_adjust_names_two_members_and_an_axis_one_of_them_is_mounted_on() {
    let adjust = "  adjust arm on plate: across within 2mm\n";
    let with = |replacement: &str| {
        let kind = euro_hinge_as("odd_hinge");
        assert!(kind.contains(adjust));
        overlay_design(
            "17.5mm",
            Some(("odd_hinge", kind.replace(adjust, replacement))),
        )
    };
    let cases = [
        (
            "  adjust arm on plat: across within 2mm\n",
            "unknown-name: unknown name 'plat'",
        ),
        (
            "  adjust arm on plate: sideways within 2mm\n",
            "type-mismatch: an adjust is along along, across or depth, found 'sideways'",
        ),
        (
            "  adjust arm on plate: along within 2mm\n",
            "missing-field: adjust arm on plate: neither has a mount on 'along', so nothing would ever be set",
        ),
        (
            "  adjust plate on plate: across within 2mm\n",
            "duplicate-name: 'plate' is named twice in one adjust",
        ),
        (
            "  adjust arm on plate: across within 2mm\n  adjust cup on plate: across within 1mm\n",
            "duplicate-name: 'plate' is adjusted twice across",
        ),
    ];
    for (replacement, expected) in cases {
        let opened = with(replacement);
        let found = common::diagnostics(&opened);
        assert!(found.iter().any(|m| m == expected), "{expected}: {found:?}");
    }
    // An adjust in a joint kind is not an entry it has.
    let opened = common::open("kind odd_joint : joint {\n  adjust a on b: across within 1mm\n}");
    let found = common::diagnostics(&opened);
    assert!(
        found
            .iter()
            .any(|m| m == "unknown-field: a joint kind has no field 'adjust'"),
        "{found:?}"
    );
}

/// Before the mechanism was text, a built-in cap stopped every cup hinge
/// at 110deg. Now a kind that copies `euro_hinge`'s mechanism word for
/// word and asks it to open 125deg is stopped by the mechanism itself: its
/// links don't lock before 180deg, but past 112.1deg its inner link runs
/// into the door, which is why the kind's stop is at 110deg. The motion
/// sweep that swings doors finds it, as it finds any body in the way.
#[test]
fn a_cup_hinge_opens_at_most_110_degrees() {
    let library = std::fs::read_to_string(workspace().join("std/hardware/hinges.stave")).unwrap();
    let start = library.find("kind euro_hinge : hinge {").unwrap();
    let length = library[start..].find("\n}\n").unwrap() + 3;
    let opening = "param opening_angle = 110deg within 85deg..110deg";
    let euro_hinge = &library[start..start + length];
    assert!(euro_hinge.contains(opening) && euro_hinge.contains("mechanism {"));
    let wide_hinge = euro_hinge
        .replace("kind euro_hinge", "kind wide_hinge")
        .replace(opening, "param opening_angle = 125deg within 85deg..125deg");
    let design =
        std::fs::read_to_string(workspace().join("tests/broken/hinge_overlay/project.stave"))
            .unwrap()
            .replace(
                "overlay(left: 9mm, right: 9mm)",
                "overlay(left: 17.5mm, right: 17.5mm)",
            )
            .replace(
                "euro_hinge(panel, on: hinge_side)",
                "wide_hinge(panel, on: hinge_side)",
            )
            + "\n"
            + &wide_hinge;
    let opened = common::open_files(&[("project.stave", &design)]);
    let messages = common::diagnostics(&opened);
    assert!(
        messages.iter().any(|m| m
            == "motion: door.hinges[bottom]'s inner_link hits door.panel when door is opened past 112.1deg"),
        "{messages:?}"
    );
}

/// Work pack stage 5: a 165° hinge is one new kind in `std:hardware` and
/// no Rust. Its corpus design checks clean; its door opens to 165deg on
/// the kind's own linkage; no member it moves goes into any wood over the
/// whole swing (the same depth test as `focus::no_cup_hinge_puts_steel_in_
/// wood`); and a pose of 170deg is refused with the kind's stop message.
#[test]
fn a_165_degree_hinge_is_only_a_kind() {
    use stave_geom::collide::{CONTACT_TOLERANCE, Placement};
    use stave_geom::mechanism::MechanismPath;
    use stave_geom::sweep::{Mover, Path, deepest_overlap};
    use stave_lang::units::Angle;
    use stave_model::mechanism::DriveValue;
    use stave_model::solved::{BodyCarrier, PoseValue};

    let folder = workspace().join("tests/designs/wide_hinge");
    let opened = stave_app::open(&folder).unwrap();
    assert!(
        opened.diagnostics.is_empty(),
        "{}",
        render_diagnostics(&opened.sources, &opened.diagnostics)
    );
    let design = common::design(&opened);
    let door = design.assemblies.iter().find(|a| a.name == "door").unwrap();
    let mechanism = door.mechanism.as_ref().unwrap();
    // Both hinges move the door alike, on the kind's linkage, to 165deg.
    assert_eq!(mechanism.hardware().len(), 2);
    assert_eq!(
        mechanism.drive().extent(),
        DriveValue::Turn(Angle::from_deg(165))
    );
    let posed = MechanismPath::new(mechanism).unwrap();
    assert!((posed.amount(PoseValue::Open) - Angle::from_deg(165).to_radians_f64()).abs() < 1e-12);

    // No steel in wood anywhere in the swing: every member the mechanism
    // moves, and every member on the door, against every part.
    let path = Path::new(mechanism).unwrap();
    let inside =
        |part_assembly: Option<stave_model::solved::AssemblyId>| part_assembly == Some(door.id);
    let mut checked = 0;
    for hardware in &design.hardware {
        for member in &hardware.members {
            let mover = match member.carrier {
                BodyCarrier::Link(link) => Mover::Member(&path, link),
                BodyCarrier::Part(part) if inside(design.part(part).unwrap().assembly) => {
                    Mover::Rigid(&path)
                }
                BodyCarrier::Part(_) | BodyCarrier::Region(_) => Mover::Fixed(Placement::REST),
            };
            let steel = WorldSolid::of_body(member);
            for part in &design.parts {
                let wood = WorldSolid::of_part(part, &Solid::of(part, design));
                let part_mover = if inside(part.assembly) {
                    Mover::Rigid(&path)
                } else {
                    Mover::Fixed(Placement::REST)
                };
                let found = deepest_overlap(&steel, mover, &wood, part_mover, 2000);
                assert!(
                    found.is_none_or(|depth| depth.depth <= CONTACT_TOLERANCE),
                    "{}'s {} in {}: {found:?}",
                    hardware.name,
                    member.name,
                    part.name
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 0);

    // Asked to open further than its stop, the kind says why not.
    let text = std::fs::read_to_string(folder.join("project.stave"))
        .unwrap()
        .replace("pose door = open", "pose door = 170deg");
    let opened = common::open_files(&[("project.stave", &text)]);
    assert_eq!(
        common::diagnostics(&opened),
        ["motion: door can't open 170deg: the arm's limiter stops the door here"]
    );
}

#[test]
fn two_doors_on_one_divider_hit_each_other() {
    insta::assert_snapshot!(rendered("tests/broken/doors_hit_each_other"));
}

#[test]
fn a_door_that_swings_into_a_knob() {
    insta::assert_snapshot!(rendered("tests/broken/knob_hits_door"));
}

#[test]
fn a_drawer_that_catches_on_a_rail() {
    insta::assert_snapshot!(rendered("tests/broken/drawer_hits_divider"));
}

#[test]
fn a_lid_that_drops_onto_the_back() {
    insta::assert_snapshot!(rendered("tests/broken/lid_hits_back"));
}

#[test]
fn a_tenon_pushed_in_the_wrong_way() {
    insta::assert_snapshot!(rendered("tests/broken/tenon_wrong_way"));
}

#[test]
fn a_chidori_without_flats_cannot_come_apart() {
    insta::assert_snapshot!(rendered("tests/broken/chidori_no_flats"));
}

/// The project text of a broken design.
fn broken_text(name: &str) -> String {
    std::fs::read_to_string(
        workspace()
            .join("tests/broken")
            .join(name)
            .join("project.stave"),
    )
    .unwrap()
}

#[test]
fn the_chidori_comes_apart_as_the_simulation_proves() {
    // As oracle_chidori checks: with its flats and its rounding the node
    // comes apart (a quarter turn back, out along the diagonal, the second
    // stick lifted off); without the flats the lock can't slide out, and
    // without the rounding it doesn't fit in the holes at all.
    let order = |opened: &Opened| {
        opened
            .diagnostics
            .iter()
            .filter(|d| d.code == stave_lang::Code::Order)
            .count()
    };
    let samples = example("joint_samples");
    assert_eq!(
        order(&samples),
        0,
        "{}",
        render_diagnostics(&samples.sources, &samples.diagnostics)
    );
    let with_flats = broken_text("chidori_no_flats").replace(
        "round(diameter: round_diameter)",
        "round(diameter: round_diameter, flats: flats, flats_angle: 45deg)",
    );
    let opened = common::open_files(&[("project.stave", &with_flats)]);
    assert!(
        opened.diagnostics.is_empty(),
        "{}",
        render_diagnostics(&opened.sources, &opened.diagnostics)
    );
    let no_flats = common::open_files(&[("project.stave", &broken_text("chidori_no_flats"))]);
    assert_eq!(order(&no_flats), 1);
    let unrounded = broken_text("chidori_no_flats").replace(
        "cut rounding: crossing(lock, first) {\n    round(diameter: round_diameter)\n  }",
        "",
    );
    let opened = common::open_files(&[("project.stave", &unrounded)]);
    let errors: Vec<_> = opened.diagnostics.iter().filter(|d| d.is_error()).collect();
    assert_eq!(
        errors.len(),
        1,
        "{}",
        render_diagnostics(&opened.sources, &opened.diagnostics)
    );
    assert_eq!(errors[0].code, stave_lang::Code::Overlap);
}

#[test]
fn every_motion_of_the_examples_is_clear_through_its_travel() {
    // The cabinet's doors and drawer, the nightstand's drawer and the box's
    // lid (to 180 degrees) are swept from rest and from every scene.
    for folder in ["clothing_cabinet", "nightstand", "box"] {
        let opened = example(folder);
        let design = common::design(&opened);
        assert!(
            design.assemblies.iter().any(|a| a.mechanism.is_some()),
            "{folder}"
        );
        assert!(opened.diagnostics.is_empty(), "{folder}");
    }
}

#[test]
fn a_door_overlay_into_the_top_is_one_error_at_the_mount() {
    insta::assert_snapshot!(rendered("tests/broken/door_overlay_into_top"));
}

#[test]
fn double_doors_with_no_gap_are_told_the_gap_they_need() {
    insta::assert_snapshot!(rendered("tests/broken/double_doors_no_gap"));
}

/// The one cut the geometry can't make yet: anything but a hole or a round
/// tenon in a round or turned part. It is a warning, the part is drawn
/// uncut there, and its fit is checked as drawn.
#[test]
fn a_notch_in_a_round_part_is_not_supported() {
    let opened = common::open(
        "stock dowel_30 { species: beech, form: dowel, diameter: 30mm }
part rail : dowel_30 {
  length: 200mm
  place {
    against(base.top)
    flush(base.left)
    flush(base.front)
  }
}
part post : pine_19 {
  length: 100mm
  width:  60mm
  place {
    orient(length: up, width: back)
    against(base.top)
    against(rail.right)
    flush(base.front)
  }
}
kind flatten : joint {
  cut housing: crossing(piece, into) {
    notch(depth: 5mm)
  }
}
joint seat : flatten(rail, into: post)",
    );
    let found = common::diagnostics(&opened);
    assert_eq!(
        found,
        ["not-supported: cutting a notch in a round or turned part is not supported yet"],
        "{found:?}"
    );
    let note = &opened.diagnostics[0].notes;
    assert_eq!(
        note,
        &["seat: rail is drawn uncut there, and its fit is checked as drawn".to_owned()]
    );
}

/// Every message of a design, with the notes a merged diagnostic carries
/// (the other mistakes its line causes).
fn messages_and_notes(opened: &stave_app::Opened) -> Vec<String> {
    opened
        .diagnostics
        .iter()
        .flat_map(|d| std::iter::once(d.message.clone()).chain(d.notes.iter().cloned()))
        .collect()
}

/// A cup is checked against its own door like any body: it sits in the
/// door only because its kind bores the hole. `broken/hinge_link_in_wood`
/// with the catalogue hinge's pins is a clean design; take the kind's
/// `cut bore:` out and the cup (and the arm's knuckle that reaches into
/// it) overlap the door.
#[test]
fn a_cup_sits_in_its_bore_and_nowhere_else() {
    let design =
        std::fs::read_to_string(workspace().join("tests/broken/hinge_link_in_wood/project.stave"))
            .unwrap()
            .replace("depth: 2.89mm", "depth: 1.39mm");
    let bored = common::open_files(&[("project.stave", &design)]);
    assert!(
        bored.diagnostics.is_empty(),
        "{:?}",
        common::diagnostics(&bored)
    );
    let cut = "  cut bore: crossing(piece, cup) { drill(diameter: 35mm, depth: 12.8mm) }\n";
    assert!(design.contains(cut));
    let unbored = common::open_files(&[("project.stave", &design.replace(cut, ""))]);
    let messages = messages_and_notes(&unbored);
    assert!(
        messages
            .iter()
            .any(|m| m.contains("door.hinges[bottom]'s cup and door.panel overlap")),
        "{messages:?}"
    );
    assert!(
        unbored
            .diagnostics
            .iter()
            .all(|d| d.code == stave_lang::Code::Overlap),
        "{messages:?}"
    );
}

/// A butt hinge's leaf is a body like any other: the box example's lid is
/// clean, and with a copy of `butt_hinge` that cuts no mortises its leaves
/// overlap the lid and the back they should be housed in.
#[test]
fn a_leaf_without_its_mortise_overlaps() {
    let read = |name: &str| std::fs::read_to_string(workspace().join(name)).unwrap();
    let library = read("std/hardware/hinges.stave");
    let start = library.find("kind butt_hinge : hinge {").unwrap();
    let length = library[start..].find("\n}\n").unwrap() + 3;
    let butt_hinge = &library[start..start + length];
    let mortises = "  cut leaf_mortise: crossing(piece, leaf) { notch(depth: 1.5mm) }\n  cut fixed_leaf_mortise: crossing(on, fixed_leaf) { notch(depth: 1.5mm) }\n";
    assert!(butt_hinge.contains(mortises));
    let bare = butt_hinge
        .replace("kind butt_hinge", "kind bare_butt_hinge")
        .replace(mortises, "");
    let lid = read("examples/box/lid.stave");
    let housed = common::open_files(&[
        ("box.stave", &read("examples/box/box.stave")),
        ("lid.stave", &lid),
    ]);
    assert!(
        housed.diagnostics.is_empty(),
        "{:?}",
        common::diagnostics(&housed)
    );
    let unhoused = common::open_files(&[
        ("box.stave", &read("examples/box/box.stave")),
        (
            "lid.stave",
            &(lid.replace("butt_hinge(length: 40mm)", "bare_butt_hinge(length: 40mm)")
                + "\n"
                + &bare),
        ),
    ]);
    let messages = messages_and_notes(&unhoused);
    assert!(
        messages
            .iter()
            .any(|m| m.contains("'s leaf and lid.panel overlap")),
        "{messages:?}"
    );
    assert!(
        messages.iter().any(|m| m.contains("'s fixed_leaf and")),
        "{messages:?}"
    );
}
