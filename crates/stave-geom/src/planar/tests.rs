//! The planar solver against today's cup hinge code, the numbers the
//! hinge oracle (`stave-app/tests/oracle_hinge.rs`, once
//! `tools/hinge-sim/linkage.py`) records, and mechanisms with closed-form
//! answers.

use glam::DVec2;
use stave_lang::units::{Angle, Length, Rational};

use super::*;

/// Nanometres from hundredths of a millimetre.
const fn hundredths(value: i64) -> i64 {
    value * 10_000
}

/// A point from nanometres.
fn point(across: i64, depth: i64) -> PlanarPoint {
    PlanarPoint::new(Length::from_nm(across), Length::from_nm(depth))
}

/// A point from whole millimetres.
fn point_mm(across: i64, depth: i64) -> PlanarPoint {
    PlanarPoint::new(Length::from_mm(across), Length::from_mm(depth))
}

/// A name.
fn name(text: &str) -> MemberName {
    MemberName::new(text).unwrap()
}

/// A plane point in `f64` nanometres from millimetres.
fn at_mm(across: f64, depth: f64) -> DVec2 {
    DVec2::new(across * 1e6, depth * 1e6)
}

/// A turning drive value in degrees (hundredths of a degree: 0.05deg is 5).
fn degrees_hundredths(value: i64) -> DriveValue {
    DriveValue::Turn(Angle::from_microdeg(value * 10_000))
}

/// The pins of `CUP_HINGE_110` (the inner link, then the outer), each
/// `(arm pin, boss pin)` in hundredths of a millimetre from the cup's
/// centre on the door's back face, `across` toward the middle of the
/// door, `depth` into the door: the same literal as
/// `std/hardware/hinges.stave`'s `euro_hinge`.
const CUP_PINS: [((i64, i64), (i64, i64)); 2] =
    [((1350, 1090), (-880, 733)), ((1328, 356), (-954, 139))];

/// The members of a cup hinge declaration.
struct CupHinge {
    cup: MemberId,
    inner: MemberId,
    outer: MemberId,
    door: MemberId,
    path: Path,
}

/// The cup hinge with every pin moved by `(across, depth)` nanometres,
/// opening to `to`; the door rides on the cup.
fn cup_hinge(across: i64, depth: i64, to: Angle) -> CupHinge {
    let mut declaration = Declaration::new();
    let arm = declaration.member(name("arm"), Holding::Held).unwrap();
    let cup = declaration.member(name("cup"), Holding::Free).unwrap();
    let inner = declaration
        .member(name("inner_link"), Holding::Free)
        .unwrap();
    let outer = declaration
        .member(name("outer_link"), Holding::Free)
        .unwrap();
    let door = declaration
        .member(name("door"), Holding::Carried(cup))
        .unwrap();
    let place = |(x, y): (i64, i64)| point(across + hundredths(x), depth + hundredths(y));
    for (link, (arm_pin, boss_pin)) in [inner, outer].into_iter().zip(CUP_PINS) {
        declaration.constrain(Constraint::Pivot {
            first: arm,
            second: link,
            at: place(arm_pin),
        });
        declaration.constrain(Constraint::Pivot {
            first: cup,
            second: link,
            at: place(boss_pin),
        });
    }
    let drive = Drive::Turn {
        member: cup,
        from: Angle::ZERO,
        to,
    };
    let mechanism = Mechanism::new(declaration, drive).unwrap();
    CupHinge {
        cup,
        inner,
        outer,
        door,
        path: Path::new(mechanism).unwrap(),
    }
}

/// The general solver poses the cup hinge closed exactly as declared, and
/// at every 0.1deg to 110deg turns the cup (and the door, carried on it)
/// by exactly the drive's angle. (Until stage 8 this test also compared
/// every pose with the old cup-hinge linkage code, which is deleted; the
/// oracle tests below hold the numbers.)
#[test]
fn the_cup_hinge_turns_the_door_with_its_cup() {
    let hinge = cup_hinge(22_500_000, -19_000_000, Angle::from_deg(110));
    let closed = hinge.path.pose(degrees_hundredths(0)).unwrap();
    assert!(
        closed
            .members()
            .iter()
            .all(|pose| *pose == PlanarPose::REST)
    );
    for k in 1..=1100 {
        let theta = (f64::from(k) / 10.0).to_radians();
        let pose = hinge.path.pose_at(DriveAmount::radians(theta)).unwrap();
        let cup = pose.member(hinge.cup).unwrap();
        assert_eq!(cup.angle(), theta);
        assert_eq!(pose.member(hinge.door), Some(cup));
        for link in [hinge.inner, hinge.outer] {
            assert!(pose.member(link).is_some(), "{k}");
        }
    }
}

/// The doc frame of the hinge oracle: millimetres in the plane across the
/// hinge axis, door closed, `x` from the cabinet side's inside face toward
/// the middle of the door, `y` from the door's back face toward its front.
/// The cup's centre is `cup_x` from the side; the pins are fixed to it.
fn oracle_hinge(cup_x_tenths: i64, to: Angle) -> CupHinge {
    cup_hinge(cup_x_tenths * 100_000, 0, to)
}

/// Where a point of the door (doc frame, mm) is at `hundredths` of a
/// degree, in mm.
fn moved(hinge: &CupHinge, hundredths_of_degree: i64, at: DVec2) -> DVec2 {
    let pose = hinge
        .path
        .pose(degrees_hundredths(hundredths_of_degree))
        .unwrap();
    pose.member(hinge.door).unwrap().apply(at) / 1e6
}

/// The oracle's sampled angles: every 0.05deg to `last` degrees, as
/// hundredths of a degree.
fn oracle_angles(last: i64) -> impl Iterator<Item = i64> {
    (0..=last * 20).map(|k| k * 5)
}

/// Oracle: the links are 22.58mm and 22.92mm pin to pin, and from closed
/// to past 110deg the two circles the boss lies on stay at least a
/// millimetre from touching: 1.02mm at their closest (at 5.4deg), 11.48mm
/// at 130deg.
#[test]
fn oracle_the_links_reach_past_110_degrees_with_a_millimetre_to_spare() {
    let lengths = CUP_PINS.map(|((ax, ay), (bx, by))| {
        f64::from(i32::try_from(ax - bx).unwrap()).hypot(f64::from(i32::try_from(ay - by).unwrap()))
            / 100.0
    });
    assert!((lengths[0] - 22.584).abs() < 0.01, "{lengths:?}");
    assert!((lengths[1] - 22.923).abs() < 0.01, "{lengths:?}");
    let hinge = oracle_hinge(50, Angle::from_deg(130));
    let margin = |hundredths: i64| {
        hinge
            .path
            .pose(degrees_hundredths(hundredths))
            .unwrap()
            .margin()
            .unwrap()
            / 1e6
    };
    let (closest, at) = oracle_angles(110)
        .map(|hundredths| (margin(hundredths), hundredths))
        .fold((f64::INFINITY, 0), |a, b| if b.0 < a.0 { b } else { a });
    assert!((closest - 1.018).abs() < 0.01, "{closest:.3}mm at {at}");
    assert!((at - 540).abs() <= 5, "{closest:.3}mm at {at}");
    assert_eq!(oracle_angles(130).find(|&h| margin(h) < 1.0), None);
    assert!((margin(13_000) - 11.480).abs() < 0.01);
}

/// Oracle: the example cabinet's door (19mm, 18mm overlay, C = 5, so the
/// cup's centre 4.5mm from the side): its front and back hinge corners at
/// five angles, moving forward and out, as the doc's table has them.
#[test]
fn oracle_the_front_edge_moves_forward_and_out() {
    let hinge = oracle_hinge(45, Angle::from_deg(110));
    let (edge, front, back) = (-18.0, 19.0, 0.0);
    let expected = [
        (0, (-18.0, 19.0), (-18.0, 0.0)),
        (3_000, (-12.1145, 33.0220), (-2.6145, 16.5675)),
        (6_000, (-13.9273, 21.7794), (2.5272, 12.2794)),
        (9_000, (-12.4590, 9.7304), (6.5410, 9.7304)),
        (11_000, (-9.0292, 2.9528), (8.8250, 9.4512)),
    ];
    for (hundredths, front_now, back_now) in expected {
        for (corner, (x, y)) in [(front, front_now), (back, back_now)] {
            let now = moved(&hinge, hundredths, at_mm(edge, corner));
            assert!(
                (now.x - x).abs() < 0.01 && (now.y - y).abs() < 0.01,
                "{hundredths}: {now}, expected ({x}, {y})"
            );
        }
    }
}

/// Oracle: at 110deg the door's back hinge corner stands 8.64mm into the
/// opening (D = 0, C = 3: the edge 15.5mm over the side).
#[test]
fn oracle_the_door_stands_8_6mm_into_the_opening_at_110_degrees() {
    let hinge = oracle_hinge(50, Angle::from_deg(110));
    let protrusion = moved(&hinge, 11_000, at_mm(-15.5, 0.0)).x;
    assert!((protrusion - 8.641).abs() < 0.01, "{protrusion:.3}mm");
}

/// Oracle: the swing-out the script measured for the catalogue's minimum
/// reveal table (boring distance 3 to 7mm by rows, door 15 to 24mm by
/// columns): how far the door's hinge edge, rounded over by 1mm, swings
/// out past where it was from closed to 110deg, to 0.01mm in every cell.
#[test]
fn oracle_the_hinge_edge_swings_out_as_the_script_measured() {
    const MEASURED: [[f64; 10]; 5] = [
        [
            0.5684, 0.6204, 0.6746, 0.7308, 0.7891, 0.8496, 0.9122, 0.9769, 1.0438, 1.8921,
        ],
        [
            0.5672, 0.6190, 0.6730, 0.7291, 0.7874, 0.8477, 0.9102, 0.9748, 1.0415, 1.1194,
        ],
        [
            0.5659, 0.6176, 0.6715, 0.7275, 0.7856, 0.8458, 0.9082, 0.9726, 1.0391, 1.1079,
        ],
        [
            0.5646, 0.6162, 0.6700, 0.7259, 0.7839, 0.8440, 0.9061, 0.9705, 1.0369, 1.1054,
        ],
        [
            0.5633, 0.6149, 0.6685, 0.7243, 0.7821, 0.8421, 0.9041, 0.9683, 1.0346, 1.1030,
        ],
    ];
    // On the catalogue's overlay (C + 12.5mm) the cup's centre is always
    // 5mm from the side.
    let hinge = oracle_hinge(50, Angle::from_deg(110));
    let poses: Vec<PlanarPose> = oracle_angles(110)
        .map(|h| {
            hinge
                .path
                .pose(degrees_hundredths(h))
                .unwrap()
                .member(hinge.door)
                .unwrap()
        })
        .collect();
    for (row, cup_inset) in [3.0, 4.0, 5.0, 6.0, 7.0].into_iter().enumerate() {
        for (column, thickness) in (15..=24).map(f64::from).enumerate() {
            let edge = -(cup_inset + 12.5);
            let corner = at_mm(edge + 1.0, thickness - 1.0);
            let leftmost = poses
                .iter()
                .map(|pose| pose.apply(corner).x / 1e6 - 1.0)
                .fold(f64::INFINITY, f64::min);
            let reveal = edge - leftmost;
            let measured = MEASURED[row][column];
            assert!(
                (reveal - measured).abs() < 0.01,
                "door {thickness}mm, C {cup_inset}: {reveal:.4}mm, the script had {measured}mm"
            );
        }
    }
}

/// The cup hinge reaches no dead point before 180deg, as today's exact
/// check says; and the offset never moves faster than its speed bound.
#[test]
fn the_cup_hinge_opens_to_180_degrees_within_its_speed_bound() {
    let hinge = cup_hinge(0, 0, Angle::from_deg(180));
    let speed = hinge.path.speed(hinge.cup).unwrap();
    let mut last = hinge.path.pose_at(DriveAmount::radians(0.0)).unwrap();
    for k in 1..=1800 {
        let theta = (f64::from(k) / 10.0).to_radians();
        let pose = hinge.path.pose_at(DriveAmount::radians(theta)).unwrap();
        let step = (pose.member(hinge.cup).unwrap().offset()
            - last.member(hinge.cup).unwrap().offset())
        .length();
        assert!(
            step <= speed.offset * 0.1f64.to_radians() * 1.000_001,
            "{k}"
        );
        last = pose;
    }
}

/// A slider-crank: a 30mm crank turning about the origin, an 80mm rod,
/// a slider on a rail along `across`. The slider is at
/// `r cos θ + √(l² − r² sin² θ)` at every angle of a whole turn.
#[test]
fn a_slider_crank_follows_its_closed_form() {
    let (radius, rod) = (30.0e6, 80.0e6);
    let mut declaration = Declaration::new();
    let ground = declaration.member(name("frame"), Holding::Held).unwrap();
    let crank = declaration.member(name("crank"), Holding::Free).unwrap();
    let link = declaration.member(name("rod"), Holding::Free).unwrap();
    let slider = declaration.member(name("slider"), Holding::Free).unwrap();
    declaration.constrain(Constraint::Pivot {
        first: crank,
        second: ground,
        at: point_mm(0, 0),
    });
    declaration.constrain(Constraint::Pivot {
        first: crank,
        second: link,
        at: point_mm(30, 0),
    });
    declaration.constrain(Constraint::Pivot {
        first: link,
        second: slider,
        at: point_mm(110, 0),
    });
    declaration.constrain(Constraint::Rail {
        runner: slider,
        guide: ground,
        direction: PlanarDirection::ACROSS,
    });
    let drive = Drive::Turn {
        member: crank,
        from: Angle::ZERO,
        to: Angle::from_deg(360),
    };
    let path = Path::new(Mechanism::new(declaration, drive).unwrap()).unwrap();
    for degrees in 0..=360 {
        let theta = f64::from(degrees).to_radians();
        let pose = path
            .pose(DriveValue::Turn(Angle::from_deg(i64::from(degrees))))
            .unwrap();
        let expected = radius * theta.cos() + (rod * rod - (radius * theta.sin()).powi(2)).sqrt();
        let slider = pose.member(slider).unwrap();
        assert_eq!(slider.angle(), 0.0);
        assert!(
            (slider.apply(at_mm(110.0, 0.0)) - DVec2::new(expected, 0.0)).length() < 1e-3,
            "{degrees}"
        );
        let crank_pin = pose.member(crank).unwrap().apply(at_mm(30.0, 0.0));
        let rod = pose.member(link).unwrap();
        assert!((rod.apply(at_mm(30.0, 0.0)) - crank_pin).length() < 1e-3);
        assert!((rod.apply(at_mm(110.0, 0.0)) - slider.apply(at_mm(110.0, 0.0))).length() < 1e-3);
    }
}

/// A slider-crank whose rod (20mm) is shorter than its crank (30mm) locks
/// where the crank pin is 20mm off the rail, at asin(2/3) = 41.81deg; it
/// is refused 0.1mm before, at asin(19.9/30) = 41.56deg (the first sample
/// past it, 41.6deg), naming the rod and the slider.
#[test]
fn a_dead_point_is_found_and_named() {
    let mut declaration = Declaration::new();
    let ground = declaration.member(name("frame"), Holding::Held).unwrap();
    let crank = declaration.member(name("crank"), Holding::Free).unwrap();
    let link = declaration.member(name("rod"), Holding::Free).unwrap();
    let slider = declaration.member(name("slider"), Holding::Free).unwrap();
    declaration.constrain(Constraint::Pivot {
        first: crank,
        second: ground,
        at: point_mm(0, 0),
    });
    declaration.constrain(Constraint::Pivot {
        first: crank,
        second: link,
        at: point_mm(30, 0),
    });
    declaration.constrain(Constraint::Pivot {
        first: link,
        second: slider,
        at: point_mm(50, 0),
    });
    declaration.constrain(Constraint::Rail {
        runner: slider,
        guide: ground,
        direction: PlanarDirection::ACROSS,
    });
    let drive = Drive::Turn {
        member: crank,
        from: Angle::ZERO,
        to: Angle::from_deg(90),
    };
    let error = Path::new(Mechanism::new(declaration, drive).unwrap()).unwrap_err();
    let MechanismError::DeadPoint { members, at } = &error else {
        panic!("{error}");
    };
    assert_eq!(members, &[name("rod"), name("slider")]);
    let limit = (19.9f64 / 30.0).asin().to_degrees();
    let DriveValue::Turn(angle) = *at else {
        panic!("{at}");
    };
    let degrees = f64::from(i32::try_from(angle.microdeg()).unwrap()) / 1e6;
    assert!(
        degrees >= limit && degrees < limit + 0.05,
        "{degrees} against {limit}"
    );
    assert_eq!(error.kind(), MechanismErrorKind::Motion);
    assert_eq!(error.to_string(), "`rod` and `slider` lock at 41.6deg");
}

/// A full-extension slide: the middle member runs on the cabinet member,
/// the drawer member on the middle one, and the middle one travels half
/// as far as the drawer. Driven by the drawer, or the other way round.
#[test]
fn a_full_extension_slide_has_a_middle_member() {
    let slide = |drive_drawer: bool| {
        let mut declaration = Declaration::new();
        let cabinet = declaration
            .member(name("cabinet_member"), Holding::Held)
            .unwrap();
        let middle = declaration
            .member(name("middle_member"), Holding::Free)
            .unwrap();
        let drawer = declaration
            .member(name("drawer_member"), Holding::Free)
            .unwrap();
        declaration.constrain(Constraint::Rail {
            runner: middle,
            guide: cabinet,
            direction: PlanarDirection::DEPTH,
        });
        declaration.constrain(Constraint::Rail {
            runner: drawer,
            guide: middle,
            direction: PlanarDirection::DEPTH,
        });
        let half = Ratio::new(Rational::new(1, 2).unwrap()).unwrap();
        let twice = Ratio::new(Rational::from_int(2)).unwrap();
        let (drive, constraint) = if drive_drawer {
            let couple = Constraint::Couple {
                follower: middle,
                leader: drawer,
                ratio: half,
            };
            let drive = Drive::Travel {
                member: drawer,
                from: Length::ZERO,
                to: Length::from_mm(450),
            };
            (drive, couple)
        } else {
            let couple = Constraint::Couple {
                follower: drawer,
                leader: middle,
                ratio: twice,
            };
            let drive = Drive::Travel {
                member: middle,
                from: Length::ZERO,
                to: Length::from_mm(225),
            };
            (drive, couple)
        };
        declaration.constrain(constraint);
        let path = Path::new(Mechanism::new(declaration, drive).unwrap()).unwrap();
        (path, middle, drawer)
    };
    let (path, middle, drawer) = slide(true);
    for (travel, middle_at) in [(0, 0.0), (100, 50.0), (450, 225.0)] {
        let pose = path
            .pose(DriveValue::Travel(Length::from_mm(travel)))
            .unwrap();
        let middle = pose.member(middle).unwrap();
        let drawer = pose.member(drawer).unwrap();
        assert_eq!((middle.angle(), drawer.angle()), (0.0, 0.0));
        assert!((middle.offset() - at_mm(0.0, middle_at)).length() < 1e-6);
        assert!(
            (drawer.offset() - at_mm(0.0, f64::from(i32::try_from(travel).unwrap()))).length()
                < 1e-6
        );
    }
    assert!(matches!(
        path.pose(DriveValue::Travel(Length::from_mm(451))),
        Err(MechanismError::OutOfRange { .. })
    ));
    let (path, middle, drawer) = slide(false);
    let pose = path.pose(DriveValue::Travel(Length::from_mm(225))).unwrap();
    assert!((pose.member(middle).unwrap().offset() - at_mm(0.0, 225.0)).length() < 1e-6);
    assert!((pose.member(drawer).unwrap().offset() - at_mm(0.0, 450.0)).length() < 1e-6);
}

/// A butt hinge is one pivot: the leaf turns about the pin and nothing
/// else moves.
#[test]
fn a_butt_hinge_turns_about_its_pin() {
    let mut declaration = Declaration::new();
    let fixed = declaration
        .member(name("fixed_leaf"), Holding::Held)
        .unwrap();
    let leaf = declaration.member(name("leaf"), Holding::Free).unwrap();
    declaration.constrain(Constraint::Pivot {
        first: leaf,
        second: fixed,
        at: point_mm(0, 0),
    });
    let drive = Drive::Turn {
        member: leaf,
        from: Angle::ZERO,
        to: Angle::from_deg(-95),
    };
    let path = Path::new(Mechanism::new(declaration, drive).unwrap()).unwrap();
    let pose = path.pose(DriveValue::Turn(Angle::from_deg(-90))).unwrap();
    let leaf = pose.member(leaf).unwrap();
    assert!((leaf.apply(at_mm(40.0, 0.0)) - at_mm(0.0, -40.0)).length() < 1e-6);
    assert_eq!(pose.member(fixed), Some(PlanarPose::REST));
    assert_eq!(pose.margin(), None);
}

/// A lid stay with its slot in the stay: the lid turns on its pivot, the
/// stay turns on its own, and the lid's pin runs out along the stay's
/// slot. A slot 150mm long lets the lid open to 90deg; one 100mm long
/// stops it where the pin is 300mm from the stay's pivot, near 68.7deg.
#[test]
fn a_lid_stay_runs_its_pin_along_its_slot() {
    let lid_stay = |slot_length: i64| {
        let mut declaration = Declaration::new();
        let cabinet = declaration.member(name("cabinet"), Holding::Held).unwrap();
        let lid = declaration.member(name("lid"), Holding::Free).unwrap();
        let stay = declaration.member(name("stay"), Holding::Free).unwrap();
        declaration.constrain(Constraint::Pivot {
            first: lid,
            second: cabinet,
            at: point_mm(0, 0),
        });
        declaration.constrain(Constraint::Pivot {
            first: stay,
            second: cabinet,
            at: point_mm(100, -200),
        });
        declaration.constrain(Constraint::Slot {
            pin: lid,
            guide: stay,
            from: point_mm(100, 0),
            to: point_mm(100, slot_length),
        });
        let drive = Drive::Turn {
            member: lid,
            from: Angle::ZERO,
            to: Angle::from_deg(90),
        };
        (
            Path::new(Mechanism::new(declaration, drive).unwrap()),
            lid,
            stay,
        )
    };
    let (path, lid, stay) = lid_stay(150);
    let path = path.unwrap();
    for degrees in 0..=90 {
        let pose = path
            .pose(DriveValue::Turn(Angle::from_deg(degrees)))
            .unwrap();
        let (lid, stay) = (pose.member(lid).unwrap(), pose.member(stay).unwrap());
        let pin = lid.apply(at_mm(100.0, 0.0));
        let pivot = at_mm(100.0, -200.0);
        assert!((stay.apply(pivot) - pivot).length() < 1e-6);
        let slot_start = stay.apply(at_mm(100.0, 0.0));
        let slot = stay.rotate(at_mm(0.0, 150.0)).normalize();
        assert!(slot.perp_dot(pin - slot_start).abs() < 1e-3, "{degrees}");
    }
    let (path, _, _) = lid_stay(100);
    let error = path.unwrap_err();
    let MechanismError::SlotEnd { pin, guide, at } = &error else {
        panic!("{error}");
    };
    assert_eq!((pin, guide), (&name("lid"), &name("stay")));
    let DriveValue::Turn(angle) = *at else {
        panic!("{at}");
    };
    assert!(
        angle > Angle::from_deg(68) && angle < Angle::from_microdeg(69_500_000),
        "{angle}"
    );
}

/// A stay with its pin in a slot in the lid: the pin stays 78.1mm from
/// the stay's pivot and on the lid's slot, and at 90deg it is 18.10mm up
/// the lid's line.
#[test]
fn a_stay_pin_runs_in_a_slot_in_the_lid() {
    let mut declaration = Declaration::new();
    let cabinet = declaration.member(name("cabinet"), Holding::Held).unwrap();
    let lid = declaration.member(name("lid"), Holding::Free).unwrap();
    let stay = declaration.member(name("stay"), Holding::Free).unwrap();
    declaration.constrain(Constraint::Pivot {
        first: lid,
        second: cabinet,
        at: point_mm(0, 0),
    });
    declaration.constrain(Constraint::Pivot {
        first: stay,
        second: cabinet,
        at: point_mm(0, -60),
    });
    declaration.constrain(Constraint::Slot {
        pin: stay,
        guide: lid,
        from: point_mm(50, 0),
        to: point_mm(10, 0),
    });
    let drive = Drive::Turn {
        member: lid,
        from: Angle::ZERO,
        to: Angle::from_deg(90),
    };
    let path = Path::new(Mechanism::new(declaration, drive).unwrap()).unwrap();
    let reach = 6100f64.sqrt() * 1e6;
    for degrees in 0..=90 {
        let theta = f64::from(degrees).to_radians();
        let pose = path
            .pose(DriveValue::Turn(Angle::from_deg(i64::from(degrees))))
            .unwrap();
        let pin = pose.member(stay).unwrap().apply(at_mm(50.0, 0.0));
        assert!(((pin - at_mm(0.0, -60.0)).length() - reach).abs() < 1e-3);
        assert!(
            DVec2::from_angle(theta).perp_dot(pin).abs() < 1e-3,
            "{degrees}"
        );
    }
    let pose = path.pose(DriveValue::Turn(Angle::from_deg(90))).unwrap();
    let pin = pose.member(stay).unwrap().apply(at_mm(50.0, 0.0)) / 1e6;
    assert!((pin - DVec2::new(0.0, 18.1025)).length() < 1e-3, "{pin}");
}

/// A four-bar with its coupler pin closed above the line from the crank
/// pin to the rocker's pivot keeps it above; its mirror image keeps it
/// below. The branch is the closed pose's.
#[test]
fn the_branch_is_the_closed_poses() {
    let four_bar = |joint_depth: i64| {
        let mut declaration = Declaration::new();
        let ground = declaration.member(name("frame"), Holding::Held).unwrap();
        let crank = declaration.member(name("crank"), Holding::Free).unwrap();
        let coupler = declaration.member(name("coupler"), Holding::Free).unwrap();
        let rocker = declaration.member(name("rocker"), Holding::Free).unwrap();
        for (first, second, at) in [
            (crank, ground, point_mm(0, 0)),
            (crank, coupler, point_mm(30, 0)),
            (coupler, rocker, point_mm(60, joint_depth)),
            (rocker, ground, point_mm(100, 0)),
        ] {
            declaration.constrain(Constraint::Pivot { first, second, at });
        }
        let drive = Drive::Turn {
            member: crank,
            from: Angle::ZERO,
            to: Angle::from_deg(40),
        };
        let path = Path::new(Mechanism::new(declaration, drive).unwrap()).unwrap();
        (path, crank, coupler, joint_depth)
    };
    for (path, crank, coupler, depth) in [four_bar(70), four_bar(-70)] {
        let rest = at_mm(60.0, f64::from(i32::try_from(depth).unwrap()));
        for degrees in 0..=40 {
            let pose = path
                .pose(DriveValue::Turn(Angle::from_deg(degrees)))
                .unwrap();
            let crank_pin = pose.member(crank).unwrap().apply(at_mm(30.0, 0.0));
            let joint = pose.member(coupler).unwrap().apply(rest);
            let side = (at_mm(100.0, 0.0) - crank_pin).perp_dot(joint - crank_pin);
            assert_eq!(
                side.signum(),
                f64::from(i32::try_from(depth.signum()).unwrap()),
                "{degrees}"
            );
        }
    }
}

/// A member's freedoms must come to exactly one.
#[test]
fn a_mechanism_needs_exactly_one_freedom() {
    let mut declaration = Declaration::new();
    let arm = declaration.member(name("arm"), Holding::Held).unwrap();
    let cup = declaration.member(name("cup"), Holding::Free).unwrap();
    let link = declaration.member(name("link"), Holding::Free).unwrap();
    declaration.constrain(Constraint::Pivot {
        first: arm,
        second: link,
        at: point_mm(10, 0),
    });
    declaration.constrain(Constraint::Pivot {
        first: cup,
        second: link,
        at: point_mm(0, 0),
    });
    let drive = Drive::Turn {
        member: cup,
        from: Angle::ZERO,
        to: Angle::from_deg(90),
    };
    let error = Mechanism::new(declaration, drive).unwrap_err();
    assert_eq!(error, MechanismError::Freedom { found: 2 });
    assert_eq!(error.kind(), MechanismErrorKind::Mechanism);
}

/// One freedom, but no order of closed-form steps: a ternary member hung
/// on two links from the frame (a four-bar of its own) and the driven
/// member hung from it and a third link.
#[test]
fn a_mechanism_with_no_order_of_steps_is_refused() {
    let mut declaration = Declaration::new();
    let ground = declaration.member(name("frame"), Holding::Held).unwrap();
    let left = declaration.member(name("left"), Holding::Free).unwrap();
    let right = declaration.member(name("right"), Holding::Free).unwrap();
    let plate = declaration.member(name("plate"), Holding::Free).unwrap();
    let driven = declaration.member(name("driven"), Holding::Free).unwrap();
    let last = declaration.member(name("last"), Holding::Free).unwrap();
    for (first, second, at) in [
        (left, ground, point_mm(0, 0)),
        (left, plate, point_mm(0, 50)),
        (right, ground, point_mm(100, 0)),
        (right, plate, point_mm(100, 50)),
        (plate, driven, point_mm(50, 80)),
        (driven, last, point_mm(50, 150)),
        (last, ground, point_mm(150, 150)),
    ] {
        declaration.constrain(Constraint::Pivot { first, second, at });
    }
    let drive = Drive::Turn {
        member: driven,
        from: Angle::ZERO,
        to: Angle::from_deg(10),
    };
    let error = Mechanism::new(declaration, drive).unwrap_err();
    assert_eq!(
        error,
        MechanismError::NoOrder {
            left: ["left", "right", "plate", "driven", "last"]
                .map(name)
                .to_vec()
        }
    );
    assert_eq!(
        error.to_string(),
        "this mechanism can't be solved one step at a time from its drive; left over: \
         `left`, `right`, `plate`, `driven` and `last`"
    );
}

/// What a declaration may not say.
#[test]
fn malformed_declarations_are_refused() {
    let turn = |member| Drive::Turn {
        member,
        from: Angle::ZERO,
        to: Angle::from_deg(10),
    };
    // A drive on a held member.
    let mut declaration = Declaration::new();
    let frame = declaration.member(name("frame"), Holding::Held).unwrap();
    let leaf = declaration.member(name("leaf"), Holding::Free).unwrap();
    declaration.constrain(Constraint::Pivot {
        first: leaf,
        second: frame,
        at: point_mm(0, 0),
    });
    assert_eq!(
        Mechanism::new(declaration.clone(), turn(frame)).unwrap_err(),
        MechanismError::DriveHeld {
            member: name("frame")
        }
    );
    // An empty range.
    let empty = Drive::Turn {
        member: leaf,
        from: Angle::from_deg(5),
        to: Angle::from_deg(5),
    };
    assert!(matches!(
        Mechanism::new(declaration.clone(), empty),
        Err(MechanismError::EmptyRange { .. })
    ));
    // A travel with no rail.
    let travel = Drive::Travel {
        member: leaf,
        from: Length::ZERO,
        to: Length::from_mm(10),
    };
    assert_eq!(
        Mechanism::new(declaration.clone(), travel).unwrap_err(),
        MechanismError::TravelWithoutRail {
            member: name("leaf")
        }
    );
    // An entry within one piece, and between two held members.
    let mut carried = declaration.clone();
    let tab = carried.member(name("tab"), Holding::Carried(leaf)).unwrap();
    carried.constrain(Constraint::Pivot {
        first: tab,
        second: leaf,
        at: point_mm(5, 0),
    });
    assert_eq!(
        Mechanism::new(carried, turn(leaf)).unwrap_err(),
        MechanismError::OnePiece {
            first: name("tab"),
            second: name("leaf")
        }
    );
    let mut held = declaration.clone();
    let other = held.member(name("other"), Holding::Held).unwrap();
    held.constrain(Constraint::Pivot {
        first: other,
        second: frame,
        at: point_mm(5, 0),
    });
    assert!(matches!(
        Mechanism::new(held, turn(leaf)),
        Err(MechanismError::BothHeld { .. })
    ));
    // A carrier that isn't declared yet.
    let mut early = Declaration::new();
    assert!(matches!(
        early.member(name("tab"), Holding::Carried(leaf)),
        Err(MechanismError::UnknownMember { .. })
    ));
    // A link whose two pins are one point.
    let mut short = Declaration::new();
    let frame = short.member(name("frame"), Holding::Held).unwrap();
    let crank = short.member(name("crank"), Holding::Free).unwrap();
    let rod = short.member(name("rod"), Holding::Free).unwrap();
    let slider = short.member(name("slider"), Holding::Free).unwrap();
    short.constrain(Constraint::Pivot {
        first: crank,
        second: frame,
        at: point_mm(0, 0),
    });
    short.constrain(Constraint::Pivot {
        first: crank,
        second: rod,
        at: point_mm(30, 0),
    });
    short.constrain(Constraint::Pivot {
        first: rod,
        second: slider,
        at: point_mm(30, 0),
    });
    short.constrain(Constraint::Rail {
        runner: slider,
        guide: frame,
        direction: PlanarDirection::ACROSS,
    });
    assert_eq!(
        Mechanism::new(short, turn(crank)).unwrap_err(),
        MechanismError::LinkHasNoLength {
            member: name("rod")
        }
    );
    assert_eq!(MemberName::new(""), None);
    assert_eq!(PlanarDirection::new(Length::ZERO, Length::ZERO), None);
    assert_eq!(Ratio::new(Rational::ZERO), None);
}

/// A pose asked for of the other kind of drive, or past its range, is
/// refused.
#[test]
fn poses_outside_the_range_are_refused() {
    let hinge = cup_hinge(0, 0, Angle::from_deg(110));
    for at in [
        DriveValue::Turn(Angle::from_deg(111)),
        DriveValue::Turn(Angle::from_deg(-1)),
        DriveValue::Travel(Length::ZERO),
    ] {
        let error = hinge.path.pose(at).unwrap_err();
        assert_eq!(error.kind(), MechanismErrorKind::Motion);
        assert!(matches!(error, MechanismError::OutOfRange { .. }));
    }
    assert!(hinge.path.pose_at(DriveAmount::radians(2.0)).is_err());
}

/// The pins of `wide_hinge_165` in `std/hardware/hinges.stave`, as
/// [`CUP_PINS`]: the inner link, then the outer, each `(arm pin, cup pin)`
/// in hundredths of a millimetre in the kind's frame.
const WIDE_PINS: [((i64, i64), (i64, i64)); 2] =
    [((1262, 687), (-1163, 110)), ((1278, 845), (-1109, 213))];

/// A four-bar cup hinge from its pins (kind's frame, hundredths of a
/// millimetre), the cup turning from closed to `to`: run and proven.
fn four_bar(pins: [((i64, i64), (i64, i64)); 2], to: Angle) -> Result<Path, MechanismError> {
    let mut declaration = Declaration::new();
    let arm = declaration.member(name("arm"), Holding::Held).unwrap();
    let cup = declaration.member(name("cup"), Holding::Free).unwrap();
    let inner = declaration
        .member(name("inner_link"), Holding::Free)
        .unwrap();
    let outer = declaration
        .member(name("outer_link"), Holding::Free)
        .unwrap();
    let place = |(x, y): (i64, i64)| point(hundredths(x), hundredths(y));
    for (link, (arm_pin, cup_pin)) in [inner, outer].into_iter().zip(pins) {
        declaration.constrain(Constraint::Pivot {
            first: arm,
            second: link,
            at: place(arm_pin),
        });
        declaration.constrain(Constraint::Pivot {
            first: cup,
            second: link,
            at: place(cup_pin),
        });
    }
    let drive = Drive::Turn {
        member: cup,
        from: Angle::ZERO,
        to,
    };
    Path::new(Mechanism::new(declaration, drive)?)
}

/// A slider-crank turned 0 to 90deg: a crank of `radius` nanometres pinned
/// at the origin, its pin `pin_microdeg` round from `across` when closed,
/// and a rod to a slider on a rail along `across` through the origin, the
/// rod `clearance` nanometres longer than the crank. Also gives how far
/// the rail comes inside the rod's circle at its closest, from the exact
/// points.
fn slider_crank(radius: f64, pin_microdeg: i64, clearance: f64) -> (Mechanism, f64) {
    let angle = f64::from(i32::try_from(pin_microdeg).unwrap()) / 1e6;
    let (sin, cos) = angle.to_radians().sin_cos();
    let pin = ((radius * cos).round(), (radius * sin).round());
    let crank = pin.0.hypot(pin.1);
    let rod = crank + clearance;
    let slider = (pin.0 + (rod * rod - pin.1 * pin.1).sqrt()).round();
    let nm = |value: f64| Length::from_nm(value as i64);
    let mut declaration = Declaration::new();
    let frame = declaration.member(name("frame"), Holding::Held).unwrap();
    let crank_member = declaration.member(name("crank"), Holding::Free).unwrap();
    let link = declaration.member(name("rod"), Holding::Free).unwrap();
    let runner = declaration.member(name("slider"), Holding::Free).unwrap();
    declaration.constrain(Constraint::Pivot {
        first: crank_member,
        second: frame,
        at: point_mm(0, 0),
    });
    declaration.constrain(Constraint::Pivot {
        first: crank_member,
        second: link,
        at: PlanarPoint::new(nm(pin.0), nm(pin.1)),
    });
    declaration.constrain(Constraint::Pivot {
        first: link,
        second: runner,
        at: PlanarPoint::new(nm(slider), Length::ZERO),
    });
    declaration.constrain(Constraint::Rail {
        runner,
        guide: frame,
        direction: PlanarDirection::ACROSS,
    });
    let drive = Drive::Turn {
        member: crank_member,
        from: Angle::ZERO,
        to: Angle::from_deg(90),
    };
    let rod = (pin.0 - slider).hypot(pin.1);
    (Mechanism::new(declaration, drive).unwrap(), rod - crank)
}

/// A 500mm crank whose rod passes 0.09997mm inside its reach at the top of
/// the crank's turn, at 45.025deg: between the samples at 45deg and
/// 45.05deg, where the rail is 0.1mm and 17.6nm inside the rod's circle.
/// Sampling alone passes it; the proof between samples refuses it, naming
/// the rod and the slider. The proof bounds how fast the margin can change
/// by the crank pin's speed (500mm a radian), so it stops a little before
/// the dip, at the first value it can't prove clear (44.97deg).
#[test]
fn a_dead_point_between_two_samples_is_caught() {
    let (mechanism, least) = slider_crank(500e6, 44_975_000, 99_970.0);
    let margin = crate::render::nm(DEAD_POINT_MARGIN);
    assert!(least < margin, "{least}");
    let sampled = path::least_sampled_margin(&mechanism).unwrap();
    assert!(
        sampled >= margin && sampled < margin + 20.0,
        "sampling alone passes it: {sampled}"
    );
    let error = Path::new(mechanism).unwrap_err();
    let MechanismError::NearDeadPoint { members, at } = &error else {
        panic!("{error}");
    };
    assert_eq!(members, &[name("rod"), name("slider")]);
    let DriveValue::Turn(angle) = *at else {
        panic!("{at}");
    };
    assert!(
        angle >= Angle::from_microdeg(44_900_000) && angle <= Angle::from_microdeg(45_050_000),
        "{angle}"
    );
    assert_eq!(error.kind(), MechanismErrorKind::Motion);
    assert_eq!(
        error.to_string(),
        format!("`rod` and `slider` may come within 0.1mm of locking near {at}")
    );
}

/// A slider-crank (a circle and a line) is proven over its whole range:
/// the same 500mm crank with its rod 0.5mm past its reach, and a 30mm
/// crank with an 80mm rod.
#[test]
fn a_slider_crank_is_proven_between_samples() {
    let (mechanism, least) = slider_crank(500e6, 44_975_000, 500_000.0);
    let proof = Path::new(mechanism).unwrap().proof();
    let proven = proof.margin.unwrap();
    assert!(proven >= crate::render::nm(DEAD_POINT_MARGIN), "{proof:?}");
    assert!(proven <= least, "{proof:?} against {least}");
    let (mechanism, _) = slider_crank(30e6, 0, 50e6);
    let proof = Path::new(mechanism).unwrap().proof();
    assert!(proof.margin.unwrap() >= 0.1e6, "{proof:?}");
}

/// The 110deg cup hinge is proven clear of its dead points over its whole
/// range, never needing to halve; the least margin proven is under the
/// 1.02mm the oracle measures at the samples, and no finer sampling finds
/// less.
#[test]
fn the_110_degree_cup_hinge_is_proven_over_its_range() {
    let path = four_bar(CUP_PINS, Angle::from_deg(110)).unwrap();
    let proof = path.proof();
    let proven = proof.margin.unwrap();
    assert!(proven >= crate::render::nm(DEAD_POINT_MARGIN), "{proof:?}");
    assert!(proven < 1.02e6, "{proof:?}");
    assert_eq!(proof.halvings, 0, "{proof:?}");
    assert!(finely_sampled_margin(&path, 110) >= proven);
}

/// The 165deg wide-angle hinge, whose links pass within about 0.19mm of a
/// dead point, is proven over its whole range too, and no finer sampling
/// finds less than the proof.
#[test]
fn the_165_degree_hinge_is_proven_over_its_range() {
    let path = four_bar(WIDE_PINS, Angle::from_deg(165)).unwrap();
    let proof = path.proof();
    let proven = proof.margin.unwrap();
    assert!(proven >= crate::render::nm(DEAD_POINT_MARGIN), "{proof:?}");
    assert!(proven < 0.2e6, "{proof:?}");
    let sampled = finely_sampled_margin(&path, 165);
    assert!(sampled >= proven && sampled < proven + 1_000.0, "{sampled}");
}

/// The least margin of any step every 0.002deg from closed to `degrees`.
fn finely_sampled_margin(path: &Path, degrees: i32) -> f64 {
    (0..=degrees * 500)
        .map(|k| {
            let theta = (f64::from(k) / 500.0).to_radians();
            let pose = path.pose_at(DriveAmount::radians(theta)).unwrap();
            pose.margin().unwrap()
        })
        .fold(f64::INFINITY, f64::min)
}

/// The proof is the same run to run: the same path, the same error.
#[test]
fn the_proof_is_the_same_run_to_run() {
    let wide = || four_bar(WIDE_PINS, Angle::from_deg(165));
    assert_eq!(wide(), wide());
    let caught = || Path::new(slider_crank(500e6, 44_975_000, 99_970.0).0);
    assert_eq!(caught(), caught());
    assert!(caught().is_err());
}
