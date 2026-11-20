//! The whole pipeline over the corpus: every example and standard module
//! checks clean, every broken design produces exactly its expected
//! diagnostic at the layers that exist, and the clothing cabinet's numbers
//! are the ones worked out by hand.

mod common;

use std::path::{Path, PathBuf};

use common::{extent, mm, part, ranges};
use stave_lang::placement::{Axis, Direction, Face, PartAxis};
use stave_lang::{Code, Fix, Severity, render_diagnostics};
use stave_model::mechanism::{DriveValue, MotionShape};
use stave_model::solved::{Contact, CutKind, CutPlace, Depth, End, PartOrigin, RoleValue, Shape};

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Every folder under `examples/` and `std/` that holds `.stave` files.
fn module_folders() -> Vec<PathBuf> {
    let mut folders = Vec::new();
    for top in ["examples", "std"] {
        let mut stack = vec![workspace().join(top)];
        while let Some(folder) = stack.pop() {
            let mut has_stave = false;
            for entry in std::fs::read_dir(&folder).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "stave") {
                    has_stave = true;
                }
            }
            if has_stave {
                folders.push(folder);
            }
        }
    }
    folders.sort();
    folders
}

#[test]
fn every_example_and_standard_module_checks_clean() {
    let folders = module_folders();
    assert!(folders.len() >= 8, "the corpus went missing");
    let mut projects = Vec::new();
    for folder in folders {
        let opened = stave_app::open(&folder).unwrap();
        assert!(
            opened.diagnostics.is_empty(),
            "{}:\n{}",
            folder.display(),
            render_diagnostics(&opened.sources, &opened.diagnostics)
        );
        if opened.design.is_some() {
            projects.push(folder.file_name().unwrap().to_string_lossy().into_owned());
        }
    }
    assert_eq!(
        projects,
        [
            "adjustable_bookcase",
            "bookshelf",
            "box",
            "chest_of_drawers",
            "clothing_cabinet",
            "clothing_cabinet_assemblies",
            "clothing_cabinet_from_box",
            "face_frame",
            "joint_samples",
            "nightstand",
            "picture_frame",
            "side_table",
            "side_table_from_box",
            "stool",
            "wall_shelf",
        ],
        "every example project solves with no diagnostics"
    );
}

struct Expectation {
    severity: Severity,
    code: Code,
    message: String,
    /// `// pending: <reason>`: the check isn't built yet.
    pending: Option<String>,
    /// `// fix: patch "..." { ... }`: the diagnostic's first fix.
    fix: Option<Fix>,
}

fn broken_designs() -> Vec<(PathBuf, Expectation)> {
    let root = workspace().join("tests/broken");
    let mut folders: Vec<PathBuf> = std::fs::read_dir(&root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_dir())
        .collect();
    folders.sort();
    folders
        .into_iter()
        .map(|folder| {
            let text = std::fs::read_to_string(folder.join("project.stave")).unwrap();
            let mut lines = text.lines();
            let first = lines.next().unwrap();
            let (severity, rest) = match first.strip_prefix("// expect: error[") {
                Some(rest) => (Severity::Error, rest),
                None => (
                    Severity::Warning,
                    first.strip_prefix("// expect: warning[").unwrap(),
                ),
            };
            let (code, message) = rest.split_once("] ").unwrap();
            let code = Code::from_str_code(code).unwrap();
            let mut next = lines.next();
            let pending = next
                .and_then(|line| line.strip_prefix("// pending: "))
                .map(str::to_owned);
            if pending.is_some() {
                next = lines.next();
            }
            let fix = next
                .and_then(|line| line.strip_prefix("// fix: "))
                .map(|fix| {
                    Fix::parse(fix).unwrap_or_else(|error| panic!("{}: {error}", folder.display()))
                });
            (
                folder,
                Expectation {
                    severity,
                    code,
                    message: message.to_owned(),
                    pending,
                    fix,
                },
            )
        })
        .collect()
}

/// Every broken design produces exactly its expected diagnostic, with a
/// help line, and nothing else. A design marked `// pending:` is for a
/// check that isn't built yet (fit, motion, ranges, buildability): until
/// it is, the design must pass cleanly and solve.
#[test]
fn broken_designs_produce_exactly_their_expected_diagnostic() {
    let mut checked = 0;
    for (folder, expected) in broken_designs() {
        let opened = stave_app::open(&folder).unwrap();
        let rendered = render_diagnostics(&opened.sources, &opened.diagnostics);
        if let Some(reason) = &expected.pending {
            assert!(
                opened.diagnostics.is_empty(),
                "{} is pending ({reason}), but reports:\n{rendered}",
                folder.display(),
            );
            assert!(opened.design.is_some(), "{}: no design", folder.display());
            continue;
        }
        checked += 1;
        let [only] = opened.diagnostics.as_slice() else {
            panic!(
                "{}: expected exactly one diagnostic, got:\n{rendered}",
                folder.display()
            );
        };
        assert_eq!(
            (only.severity, only.code),
            (expected.severity, expected.code),
            "{}:\n{rendered}",
            folder.display()
        );
        assert_eq!(
            only.message,
            expected.message,
            "{}:\n{rendered}",
            folder.display()
        );
        assert!(
            only.help.is_some(),
            "{}: every diagnostic has a help line",
            folder.display()
        );
        if let Some(fix) = &expected.fix {
            assert_eq!(
                only.fixes.first(),
                Some(fix),
                "{}: the first fix\n{rendered}",
                folder.display()
            );
        }
    }
    assert!(checked >= 27, "only {checked} broken designs checked");
}

#[test]
fn diagnostics_are_identical_run_to_run() {
    for (folder, _) in broken_designs() {
        let first = stave_app::open(&folder).unwrap();
        let second = stave_app::open(&folder).unwrap();
        assert_eq!(first.diagnostics, second.diagnostics);
    }
    let cabinet = workspace().join("examples/clothing_cabinet");
    let first = stave_app::open(&cabinet).unwrap();
    let second = stave_app::open(&cabinet).unwrap();
    assert_eq!(first.design, second.design);
}

/// The hand checks of the clothing cabinet (900 wide, 1700 tall, 560 deep,
/// 19mm oak). Every number below was worked out from the design files by
/// hand before this test was written.
#[test]
fn the_clothing_cabinet_is_where_the_design_says() {
    let opened = stave_app::open(&workspace().join("examples/clothing_cabinet")).unwrap();
    let design = common::design(&opened);

    // The base is the ground part: 900 x 560 x 19, lying flat at the origin.
    let base = part(design, "base");
    assert_eq!(
        extent(base),
        ranges([("0", "900"), ("0", "19"), ("0", "560")])
    );
    assert_eq!(base.cut.length, mm("900"));

    // The sides stand on the base, flush with its left/right and back:
    // 1700 - 19 - 19 = 1662 tall.
    let side_l = part(design, "side_l");
    assert_eq!(
        extent(side_l),
        ranges([("0", "19"), ("19", "1681"), ("0", "560")])
    );
    assert_eq!(side_l.pose.orientation.length(), Direction::Up);
    assert_eq!(side_l.pose.orientation.thickness(), Direction::Right);
    let side_r = part(design, "side_r");
    assert_eq!(
        extent(side_r),
        ranges([("881", "900"), ("19", "1681"), ("0", "560")])
    );
    assert_eq!(side_r.origin, PartOrigin::Mirror { source: side_l.id });
    assert_eq!(side_r.pose.orientation.thickness(), Direction::Left);

    // The top sits on the sides, centred, flush at the back: 940 x 580,
    // overhanging 20mm at both ends and at the front.
    let top = part(design, "top");
    assert_eq!(
        extent(top),
        ranges([("-20", "920"), ("1681", "1700"), ("-20", "560")])
    );
    assert_eq!((top.cut.length, top.cut.width), (mm("940"), mm("580")));

    // The back fits between the sides (862) and the rabbets add 9mm each side.
    let back = part(design, "back");
    assert_eq!(
        extent(back),
        ranges([("19", "881"), ("19", "1681"), ("554", "560")])
    );
    assert_eq!(
        (back.cut.length, back.cut.width, back.cut.thickness),
        (mm("1662"), mm("880"), mm("6"))
    );
    assert_eq!(back.drawn_offset.width, mm("9"));

    // The divider is drawer_zone (500) above the base; dados add 9 each end.
    let divider = part(design, "divider");
    assert_eq!(
        extent(divider),
        ranges([("19", "881"), ("519", "538"), ("0", "554")])
    );
    assert_eq!(
        (divider.cut.length, divider.cut.width),
        (mm("880"), mm("554"))
    );
    let shelf = part(design, "shelf");
    assert_eq!(
        extent(shelf),
        ranges([("19", "881"), ("1502", "1521"), ("0", "554")])
    );

    // The rod hangs 50 below the shelf, half way between the side's front
    // and the back's front: deep centre 277.
    let rod = part(design, "rod");
    assert_eq!(
        extent(rod),
        ranges([("19", "881"), ("1424", "1452"), ("263", "291")])
    );
    assert_eq!(rod.cut.length, mm("880"));
    assert_eq!(rod.shape, Shape::Round { diameter: mm("28") });

    // Feet: turned, 80 long, 70 across, at the base's corners inset 60,
    // standing down from its bottom; the round tenon adds 16 at the top.
    let foot = part(design, "foot[back_right]");
    assert_eq!(
        extent(foot),
        ranges([("770", "840"), ("-80", "0"), ("430", "500")])
    );
    assert_eq!(foot.cut.length, mm("96"));
    assert_eq!(foot.pose.orientation.length(), Direction::Down);
    assert_eq!(foot.pose.origin.up, mm("16"));
    assert_eq!(
        extent(part(design, "foot[front_left]")),
        ranges([("60", "130"), ("-80", "0"), ("60", "130")])
    );
    assert_eq!(design.floor(), Some(mm("-80")));

    // Openings and bays.
    let region = |name: &str| {
        design
            .regions()
            .iter()
            .find(|r| r.name == name)
            .unwrap()
            .bounds
    };
    let opening = region("front_opening");
    assert_eq!(
        (
            opening.min().across,
            opening.max().across,
            opening.min().up,
            opening.max().up,
            opening.max().deep
        ),
        (mm("19"), mm("881"), mm("19"), mm("1681"), mm("554"))
    );
    assert_eq!(
        (
            region("bay[drawers]").max().up,
            region("bay[doors]").min().up
        ),
        (mm("519"), mm("538"))
    );
    assert_eq!(region("drawer_bay[top]").min().up, mm("269"));
    assert_eq!(region("door_bay[right]").min().across, mm("450"));

    // Door bays: overlays 18 left and right, -1.5 at the top, 8 at the
    // bottom, and the meeting edge -0.75.
    let door = region("door_l.door");
    assert_eq!(
        (
            door.min().across,
            door.max().across,
            door.min().up,
            door.max().up,
            door.min().deep
        ),
        (mm("1"), mm("449.25"), mm("530"), mm("1679.5"), mm("-19"))
    );
    assert_eq!(
        extent(part(design, "door_l.stile[hinge]")),
        ranges([("1", "61"), ("530", "1679.5"), ("-19", "0")])
    );
    assert_eq!(
        extent(part(design, "door_l.stile[latch]")),
        ranges([("389.25", "449.25"), ("530", "1679.5"), ("-19", "0")])
    );
    let rail = part(design, "door_l.rail[top]");
    assert_eq!(
        extent(rail),
        ranges([("61", "389.25"), ("1619.5", "1679.5"), ("-19", "0")])
    );
    assert_eq!(rail.cut.length, mm("348.25"));
    let panel = part(design, "door_l.panel");
    assert_eq!(
        extent(panel),
        ranges([("61", "389.25"), ("590", "1619.5"), ("-12.5", "-6.5")])
    );
    assert_eq!(
        (panel.cut.length, panel.cut.width),
        (mm("1045.5"), mm("344.25"))
    );

    // The right door is the left door's mirror image about across = 450.
    let mirrored = part(design, "door_r.stile[hinge]");
    assert_eq!(
        extent(mirrored),
        ranges([("839", "899"), ("530", "1679.5"), ("-19", "0")])
    );
    assert!(matches!(
        mirrored.origin,
        PartOrigin::MirroredAssembly { .. }
    ));
    assert_eq!(
        mirrored.cuts.len(),
        part(design, "door_l.stile[hinge]").cuts.len()
    );

    // Drawers: fronts in the drawer bays with overlays 18/8 and a 1.5 gap
    // between them; boxes 220 tall, 12.7 in from the opening for slides.
    assert_eq!(
        extent(part(design, "drawer[bottom].front")),
        ranges([("1", "899"), ("11", "268.25"), ("-19", "0")])
    );
    assert_eq!(
        extent(part(design, "drawer[top].front")),
        ranges([("1", "899"), ("269.75", "527"), ("-19", "0")])
    );
    assert_eq!(
        extent(part(design, "drawer[bottom].side[left]")),
        ranges([("31.7", "43.7"), ("34", "254"), ("0", "500")])
    );
    assert_eq!(
        extent(part(design, "drawer[top].side[right]")),
        ranges([("856.3", "868.3"), ("284", "504"), ("0", "500")])
    );
    let bottom = part(design, "drawer[bottom].bottom");
    assert_eq!(
        extent(bottom),
        ranges([("43.7", "856.3"), ("44", "50"), ("12", "488")])
    );
    assert_eq!(
        (bottom.cut.length, bottom.cut.width),
        (mm("824.6"), mm("488"))
    );
    assert_eq!(
        part(design, "drawer[top].sub_front").cut.length,
        mm("824.6")
    );

    // Hardware.
    let hardware = |name: &str| design.hardware().iter().find(|h| h.name == name).unwrap();
    // A slide is two members in the 12.7mm gap: the drawer member against
    // the drawer side (31.7), the cabinet member against the opening's side
    // (19).
    let slide = hardware("drawer[bottom].slides[left]");
    assert_eq!(slide.mount_face, Face::Left);
    let drawer_member = common::member_bounds(slide, "drawer_member");
    assert_eq!(
        (drawer_member.min().across, drawer_member.max().across),
        (mm("25.35"), mm("31.7"))
    );
    let cabinet_member = common::member_bounds(slide, "cabinet_member");
    assert_eq!(
        (cabinet_member.min().across, cabinet_member.max().across),
        (mm("19"), mm("25.35"))
    );
    // A hinge: the cup (35 across, 13 deep) sunk into the back of the hinge
    // stile (across 1..61, deep -19..0), its rim 5 in from the stile's edge
    // at 1; 100 above the door's bottom (530).
    let hinge = hardware("door_l.hinges[lower]");
    assert_eq!(hinge.mount_face, Face::Back);
    let cup = common::member_bounds(hinge, "cup");
    assert_eq!(
        (
            cup.min().up,
            cup.min().across,
            cup.max().across,
            cup.min().deep,
            cup.max().deep
        ),
        (mm("630"), mm("6"), mm("41"), mm("-12.8"), mm("0"))
    );
    // Its plate (50 x 36 x 11) on the inside face of side_l (19), centred on
    // the cup (647.5) and on the 37mm screw line back from side_l's front:
    // never in side_l (across 0..19).
    let plate = common::member_bounds(hinge, "plate");
    assert_eq!(
        (
            plate.min().across,
            plate.max().across,
            plate.min().up,
            plate.max().up,
            plate.min().deep,
            plate.max().deep
        ),
        (
            mm("19"),
            mm("30"),
            mm("622.5"),
            mm("672.5"),
            mm("19"),
            mm("55")
        )
    );
    let side_r_id = side_r.id;
    let mirrored_hinge = hardware("door_r.hinges[upper]");
    let mirrored_plate = common::member_bounds(mirrored_hinge, "plate");
    assert_eq!(
        (mirrored_plate.min().across, mirrored_plate.max().across),
        (mm("870"), mm("881"))
    );
    assert_eq!(
        common::role(mirrored_hinge, "on"),
        Some(RoleValue::Part(side_r_id)),
        "re-bound to side_r"
    );
    let knob = common::member_bounds(hardware("door_l.handle"), "knob");
    assert_eq!((knob.min().deep, knob.min().up), (mm("-49"), mm("1091.75")));

    // Motions.
    let assembly = |name: &str| design.assemblies().iter().find(|a| a.name == name).unwrap();
    // The doors move on their cup hinges' mechanism, in the kind's frame:
    // the plane across the hinge edge through the cup's centre on the
    // stile's back face (across 23.5, deep 0), `across` toward the middle
    // of the door and `depth` out of its front.
    let mechanism = |name: &str| assembly(name).mechanism.as_ref().unwrap();
    let frame = mechanism("door_l").frame();
    assert_eq!(
        (frame.axis(), frame.origin().across, frame.origin().deep),
        (Axis::Up, mm("23.5"), mm("0"))
    );
    assert_eq!(
        (frame.across(), frame.depth()),
        (Direction::Right, Direction::Front)
    );
    assert_eq!(
        mechanism("door_l").drive().extent(),
        DriveValue::Turn(stave_lang::units::Angle::from_deg(110))
    );
    let mirrored = mechanism("door_r").frame();
    assert_eq!(
        (mirrored.origin().across, mirrored.across()),
        (mm("876.5"), Direction::Left)
    );
    assert!(matches!(
        mechanism("drawer[top]").shape(),
        MotionShape::Travel {
            toward: Direction::Front,
            ..
        }
    ));

    // Joints and cuts.
    let dados = design
        .joints()
        .iter()
        .find(|j| j.name == "shelf_dados")
        .unwrap();
    assert_eq!(dados.pairs.len(), 4);
    assert!(
        dados
            .pairs
            .iter()
            .all(|p| p.contact == Contact::Butt { axis: Axis::Across })
    );
    let notch = side_l
        .cuts
        .iter()
        .find(|c| matches!(&c.place, CutPlace::Crossing { other, .. } if *other == divider.id))
        .unwrap();
    assert_eq!(
        notch.kind,
        CutKind::Notch {
            depth: Depth::Length(mm("9"))
        }
    );
    assert_eq!(
        (notch.face.axis, notch.face.end),
        (PartAxis::Thickness, End::End)
    );
    let cup = part(design, "door_l.stile[hinge]")
        .cuts
        .iter()
        .find(|c| matches!(c.kind, CutKind::Drill { diameter, .. } if diameter == mm("35")))
        .unwrap();
    // Centred 22.5mm from the stile's edge: cup_inset and half the cup.
    let CutPlace::Member { footprint, .. } = cup.place else {
        panic!("the cup's bore is not a hardware cut: {:?}", cup.place);
    };
    assert_eq!(
        (footprint.min.width, footprint.max.width),
        (mm("5"), mm("40"))
    );

    // Requires hold and scenes are there.
    assert!(design.requires.iter().all(|r| r.holds));
    assert_eq!(
        design
            .scenes
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>(),
        ["rest", "all_open", "drawer_bench"]
    );
    assert_eq!(design.parts().len(), 34);
}

/// The door's hinge edge, front corner, as the left door of the cabinet
/// opens on its cup hinges' linkage: exactly where it is placed when
/// closed, then pushed forward and out, as
/// `oracle_hinge::the_front_edge_moves_forward_and_out` measures for the
/// same hinge (19mm door, 18mm overlay, boring distance
/// 5mm).
#[test]
fn the_cabinet_doors_move_on_their_hinges_linkage() {
    use stave_geom::mechanism::MechanismPath;
    let opened = stave_app::open(&workspace().join("examples/clothing_cabinet")).unwrap();
    let design = common::design(&opened);
    let door = design
        .assemblies()
        .iter()
        .find(|a| a.name == "door_l")
        .unwrap();
    let path = MechanismPath::new(door.mechanism.as_ref().expect("door_l moves")).unwrap();
    let moving = path.moving();
    // The stile's front corner at its hinge edge: across 1, deep -19.
    let corner = glam::DVec3::new(1e6, 647.5e6, -19e6);
    let expected = [
        (0, 1.0, -19.0),
        (30, 6.89, -33.02),
        (60, 5.07, -21.78),
        (90, 6.54, -9.73),
        (110, 9.97, -2.95),
    ];
    for (degrees, across, deep) in expected {
        let theta = f64::from(degrees).to_radians();
        let moved = path.matrix(moving, theta).unwrap().transform_point3(corner);
        let (x, z) = (moved.x / 1e6, moved.z / 1e6);
        assert!(
            (x - across).abs() < 0.02 && (z - deep).abs() < 0.02,
            "{degrees}deg: ({x:.2}, {z:.2}), expected ({across}, {deep})"
        );
    }
    // Closed is exact: no float is involved at 0deg.
    assert_eq!(path.matrix(moving, 0.0), Some(glam::DMat4::IDENTITY));
}
