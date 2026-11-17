//! Mechanisms (grammar, "Mechanisms" and "How a mechanism is solved"):
//! every piece of hardware moves on the mechanism its kind declares,
//! solved by one general solver; several items on one assembly move it as
//! one mechanism.

mod common;

use std::path::Path;

use stave_geom::mechanism::MechanismPath;
use stave_lang::placement::Axis;
use stave_lang::placement::Direction;
use stave_lang::units::{Angle, Length};
use stave_model::mechanism::MotionShape;
use stave_model::mechanism::{DriveValue, Holding, Mechanism, MemberIndex};
use stave_model::solved::{BodyCarrier, PoseValue, SolvedDesign};

fn workspace() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

/// The mechanism of the one assembly named `name`.
fn mechanism<'d>(design: &'d SolvedDesign, name: &str) -> &'d Mechanism {
    design
        .assemblies
        .iter()
        .find(|a| a.name == name)
        .and_then(|a| a.mechanism.as_ref())
        .unwrap_or_else(|| panic!("no mechanism for {name}"))
}

/// The member of a mechanism with this name (the first, for several
/// alike).
fn member(mechanism: &Mechanism, name: &str) -> MemberIndex {
    let index = mechanism
        .members()
        .iter()
        .position(|m| m.name == name)
        .unwrap_or_else(|| panic!("no member {name}"));
    MemberIndex::new(index).unwrap()
}

/// Two butt hinges drive the lid; the stay has no drive and follows it,
/// and its stop ends the lid's motion upright, sooner than the hinges'
/// 180deg.
#[test]
fn a_lid_stay_follows_its_lid_and_stops_it() {
    let opened = stave_app::open(&workspace().join("tests/designs/lid_stay")).unwrap();
    assert!(
        opened.diagnostics.is_empty(),
        "{:?}",
        common::diagnostics(&opened)
    );
    let design = common::design(&opened);
    let lid = mechanism(design, "lid");
    // The hinges first (they drive), then the stay that follows.
    let names: Vec<&str> = lid
        .hardware()
        .iter()
        .map(|id| design.hardware[id.index()].name.as_str())
        .collect();
    assert_eq!(names, ["lid.hinges[left]", "lid.hinges[right]", "lid.stay"]);
    assert_eq!(lid.drive().extent(), DriveValue::Turn(Angle::from_deg(90)));
    assert_eq!(
        lid.stop().map(|stop| stop.message.as_str()),
        Some("the stay's slot holds the lid upright")
    );
    // The stay's bracket under the lid rides on the hinge's leaf; its arm
    // is placed by the mechanism.
    let bracket = lid
        .members()
        .iter()
        .find(|m| m.name == "leaf" && m.hardware == lid.hardware()[2])
        .unwrap();
    assert_eq!(bracket.holding, Holding::Carried(lid.moving()));
    let stay = common::hardware(design, "lid.stay");
    let arm = stay.members.iter().find(|m| m.name == "arm").unwrap();
    let BodyCarrier::Link(arm_index) = arm.carrier else {
        panic!("the arm is placed by the mechanism");
    };
    // Open, the arm's slotted end has slid up the slot: the arm turned.
    let path = MechanismPath::new(lid).unwrap();
    assert!(matches!(path.shape(), MotionShape::Turn { .. }));
    let open = path.amount(PoseValue::Open);
    assert!((open - std::f64::consts::FRAC_PI_2).abs() < 1e-9);
    let pose = path.planar_pose(arm_index, open).unwrap();
    assert!(pose.angle().abs() > 0.1, "{pose:?}");
    // The lid's mechanism reaches as far as the stay lets it.
    assert_eq!(
        mechanism(design, "lid").drive().extent(),
        DriveValue::Turn(Angle::from_deg(90))
    );
}

/// A full-extension slide: the middle member runs on the cabinet member,
/// the drawer member on the middle one, and the middle one travels half
/// as far as the drawer (`couple`).
#[test]
fn a_full_extension_slide_has_a_middle_member() {
    let slide = "
/// A three-part slide: the drawer comes all the way out, the middle
/// member half way.
kind full_slide : slide {
  roles {
    piece: Part
    in:    Region
  }
  param length = 300mm within [300mm]
  clearance: 12.7mm
  travel:    length

  mount {
    face:   face(piece, nearer: in)
    across: center(piece)
    along:  from(end(piece, toward: front))
  }

  member cabinet_member {
    with:  in
    shape: box(along: 0mm..length, across: 40mm, depth: -12.7mm..-8.5mm)
  }
  member middle_member {
    shape: box(along: 0mm..length, across: 40mm, depth: -8.5mm..-4.25mm)
  }
  member drawer_member {
    with:  piece
    shape: box(along: 0mm..length, across: 40mm, depth: -4.25mm..0mm)
  }

  mechanism {
    rail middle_member, cabinet_member along along
    rail drawer_member, middle_member along along
    couple middle_member to drawer_member: 1/2
    drive: travel(drawer_member, from: 0mm, to: -length)
  }
}

part side_l : pine_19 {
  length: 300mm
  width:  300mm
  place {
    orient(length: up, width: back)
    against(base.top)
    flush(base.left)
    flush(base.front)
  }
}

part side_r = mirror(side_l, about: mid(base, across))

region bay = opening(
  left:   side_l.right,
  right:  side_r.left,
  bottom: base.top,
  top:    side_l.top,
  front:  base.front,
  back:   base.back,
)

component Drawer {
  param opening : Region

  part side[left, right] : pine_12 {
    length: 300mm
    width:  100mm
    place {
      orient(length: back, width: up)
      spaced(ends(inset: 12.7mm), from: opening.left, to: opening.right)
      flush(opening.bottom, inset: 10mm)
      flush(opening.front)
    }
  }

  hardware slides[left, right] : full_slide(side[same], in: opening)
}

assembly drawer : Drawer {
  opening: bay
}
";
    let opened = common::open(slide);
    assert!(
        opened.diagnostics.is_empty(),
        "{:?}",
        common::diagnostics(&opened)
    );
    let design = common::design(&opened);
    let drawer = mechanism(design, "drawer");
    assert_eq!(
        drawer.drive().extent(),
        DriveValue::Travel(Length::from_mm(300))
    );
    let path = MechanismPath::new(drawer).unwrap();
    // Not one rail: the drawer member runs on a member that moves.
    assert_eq!(path.shape(), MotionShape::General);
    let open = path.amount(PoseValue::Open);
    let moved = |name: &str| {
        let matrix = path.matrix(member(drawer, name), open).unwrap();
        // World nanometres: front is toward smaller `deep`.
        matrix.w_axis.z / 1e6
    };
    assert!((moved("drawer_member") + 300.0).abs() < 1e-6);
    assert!((moved("middle_member") + 150.0).abs() < 1e-6);
    assert!(moved("cabinet_member").abs() < 1e-9);
    // The middle member is placed by the mechanism.
    let left = common::hardware(design, "drawer.slides[left]");
    let middle = left
        .members
        .iter()
        .find(|m| m.name == "middle_member")
        .unwrap();
    assert!(matches!(middle.carrier, BodyCarrier::Link(_)));
}

/// A slide is a rail: its drawer travels toward the front by its travel,
/// exactly; a butt hinge is a pivot: its lid turns about the pin on its
/// edge. Both are chosen by the shape of the mechanism, not by the kind.
#[test]
fn a_slide_is_a_rail_and_a_butt_hinge_a_pivot() {
    let cabinet = stave_app::open(&workspace().join("examples/clothing_cabinet")).unwrap();
    let design = common::design(&cabinet);
    let drawer = mechanism(design, "drawer[top]");
    let MotionShape::Travel { toward, travel } = MechanismPath::new(drawer).unwrap().shape() else {
        panic!("a slide travels");
    };
    assert_eq!(toward, Direction::Front);
    assert_eq!(travel, Length::from_mm(500));

    let boxed = stave_app::open(&workspace().join("examples/box")).unwrap();
    let design = common::design(&boxed);
    let lid = mechanism(design, "lid");
    let MotionShape::Turn {
        line: pivot,
        opening,
    } = MechanismPath::new(lid).unwrap().shape()
    else {
        panic!("a butt hinge turns about its pin");
    };
    assert_eq!(opening, Angle::from_deg(180));
    assert_eq!(
        (pivot.axis, pivot.free_side, pivot.swing_toward),
        (Axis::Across, Direction::Front, Direction::Up)
    );

    // The pin is on the edge, in the face the lid closes on.
    assert_eq!(pivot.through.deep, Length::from_nm(202_500_000));
    assert_eq!(pivot.through.up, Length::from_mm(110));
}

/// A scene can't ask for more than the mechanism reaches: past the stay's
/// stop the pose is refused with the stay's own words.
#[test]
fn a_pose_past_the_stop_is_refused_with_its_message() {
    let design = std::fs::read_to_string(workspace().join("tests/designs/lid_stay/project.stave"))
        .unwrap()
        .replace("pose lid = open", "pose lid = 100deg");
    let opened = common::open_files(&[("project.stave", &design)]);
    assert_eq!(
        common::diagnostics(&opened),
        ["motion: lid can't open 100deg: the stay's slot holds the lid upright"]
    );
    // As far as it reaches is fine.
    let design = design.replace("pose lid = 100deg", "pose lid = 90deg");
    let opened = common::open_files(&[("project.stave", &design)]);
    assert!(
        opened.diagnostics.is_empty(),
        "{:?}",
        common::diagnostics(&opened)
    );
}
