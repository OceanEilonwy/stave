//! The cup hinge's linkage, checked through the whole pipeline: the
//! numbers `docs/hardware/cup-hinge-linkage.md` records, on small cabinets
//! (one door on two `euro_hinge`s) built for each door thickness, boring
//! distance and side screw setting.
//!
//! These replace the Python design check that used to live in
//! the deleted `tools/hinge-sim/linkage.py`: the same cases and the same expected
//! numbers, now measured on the design Stave solves rather than on a copy
//! of the mechanism. Where the script printed a number, the test asserts
//! it to 0.01mm (the script and Stave share the pins exactly; only float
//! rounding differs). [`refit_the_cup_hinge`] replaces
//! `tools/hinge-sim/fit.py`: it is a tool, run by hand.
//!
//! Frame of every number here, as in the doc: in the plane across the
//! hinge axis, door closed, millimetres; `x` across the door from the
//! cabinet side's inside face toward the middle of the door, `y` from the
//! door's back face toward its front.

mod common;

use glam::{DVec2, DVec3};
use stave_app::Opened;
use stave_geom::mechanism::MechanismPath;
use stave_geom::render;
use stave_lang::placement::Direction;
use stave_lang::render_diagnostics;
use stave_lang::units::Length;
use stave_model::mechanism::{FramePoint, Holding, Mechanism, MechanismEntry, MemberIndex};
use stave_model::solved::{Point, SolvedDesign, WorldBox};

/// A tenth of a millimetre, in nanometres.
const TENTH: i64 = 100_000;

/// A length in tenths of a millimetre, as written in a `.stave` file.
fn written(tenths: i64) -> String {
    let sign = if tenths < 0 { "-" } else { "" };
    let magnitude = tenths.abs();
    format!("{sign}{}.{}mm", magnitude / 10, magnitude % 10)
}

/// One door on two cup hinges: a 19mm cabinet side (and its mirror) on a
/// base, the door covering the opening by `overlay` (tenths of a
/// millimetre) on both sides, `thickness` millimetres thick, its hinges'
/// cups `cup_inset` (tenths) from its edge on a 0mm plate.
fn cabinet(thickness: i64, cup_inset: i64, overlay: i64) -> String {
    format!(
        "stave 1

project \"Hinge check\" {{
  units: mm
}}

stock side_19 {{ species: birch, form: plywood, thickness: 19mm }}
stock door_stock {{ species: birch, form: plywood, thickness: {thickness}mm }}

part base : side_19 {{
  length: 600mm
  width:  400mm
  place {{
    ground
  }}
}}

part side_l : side_19 {{
  length: 700mm
  width:  400mm
  place {{
    orient(length: up, width: back)
    against(base.top)
    flush(base.left)
    flush(base.back)
  }}
}}

part side_r = mirror(side_l, about: mid(base, across))

region bay = opening(
  left:   side_l.right,
  right:  side_r.left,
  bottom: base.top,
  top:    side_l.top,
  front:  base.front,
  back:   base.back,
)

component Door {{
  param opening    : Region
  param hinge_side : Part

  region face = cover(opening, overlay(left: {overlay}, right: {overlay}), thickness: door_stock.thickness)

  part panel : door_stock {{
    length: face.up
    width:  face.across
    place {{
      orient(length: up, width: right)
      flush(face.left)
      flush(face.bottom)
      flush(face.front)
    }}
  }}

  hardware hinges[bottom, top] : euro_hinge(panel, on: hinge_side, cup_inset: {cup_inset}) {{
    place {{
      spaced(ends(inset: 80mm), from: panel.bottom, to: panel.top)
    }}
  }}
}}

assembly door : Door {{
  opening:    bay
  hinge_side: side_l
}}
",
        overlay = written(overlay),
        cup_inset = written(cup_inset),
    )
}

/// The catalogue's overlay on a 0mm plate with the side screw in the
/// middle: boring distance + 12.5mm (tenths of a millimetre).
fn catalogue_overlay(cup_inset: i64) -> i64 {
    cup_inset + 125
}

/// Opens [`cabinet`].
fn open_cabinet(thickness: i64, cup_inset: i64, overlay: i64) -> Opened {
    common::open_files(&[("project.stave", &cabinet(thickness, cup_inset, overlay))])
}

/// Opens [`cabinet`] and asserts it checks clean.
fn open_clean(thickness: i64, cup_inset: i64, overlay: i64) -> Opened {
    let opened = open_cabinet(thickness, cup_inset, overlay);
    assert!(
        opened.diagnostics.is_empty(),
        "door {thickness}mm, cup inset {}, overlay {}:\n{}",
        written(cup_inset),
        written(overlay),
        render_diagnostics(&opened.sources, &opened.diagnostics)
    );
    opened
}

/// A world point in `f64` nanometres.
fn world(point: Point) -> DVec3 {
    DVec3::new(
        render::nm(point.across),
        render::nm(point.up),
        render::nm(point.deep),
    )
}

/// The eight corners of a box.
fn corners(region: &WorldBox) -> [DVec3; 8] {
    let (low, high) = (world(region.min()), world(region.max()));
    [0, 1, 2, 3, 4, 5, 6, 7].map(|k| {
        DVec3::new(
            if k & 1 == 0 { low.x } else { high.x },
            if k & 2 == 0 { low.y } else { high.y },
            if k & 4 == 0 { low.z } else { high.z },
        )
    })
}

/// A world direction as a unit vector in the frame of [`world`].
fn unit(direction: Direction) -> DVec3 {
    let sign = if direction.is_positive() { 1.0 } else { -1.0 };
    world(Point::default().with(direction.axis(), Length::from_nm(1))) * sign
}

/// A point of a mechanism's plane in nanometres, as `DVec2`.
fn planar(point: FramePoint) -> DVec2 {
    DVec2::new(render::nm(point.across), render::nm(point.depth))
}

/// The door's swing, in the doc's frame (millimetres).
struct Swing {
    /// The door's mechanism, run through its range.
    path: MechanismPath,
    /// The member the door moves with (the cup).
    moving: MemberIndex,
    /// Each link's pin in the held arm and its pin in the moving cup, in
    /// the mechanism's plane (nanometres from its origin), closed.
    links: Vec<(DVec2, DVec2)>,
    /// The doc's origin (the side's inside face, the door's back face) in
    /// the mechanism's plane, nanometres from the plane's own origin.
    origin: DVec2,
    /// The door's hinge edge, front and back faces in the doc's frame.
    edge: f64,
    back: f64,
    front: f64,
}

impl Swing {
    /// The swing of the door of an opened [`cabinet`]: the only place
    /// these tests read how the pipeline represents a hinged motion.
    fn of(design: &SolvedDesign) -> Swing {
        let door = design
            .assemblies
            .iter()
            .find(|a| a.name == "door")
            .expect("the door assembly");
        let mechanism = door
            .mechanism
            .as_ref()
            .expect("the door moves on its hinges' mechanism");
        let path = MechanismPath::new(mechanism).expect("the mechanism runs");
        let moving = mechanism.moving();
        let frame = mechanism.frame();
        let (origin, x, y) = (
            world(frame.origin()),
            unit(frame.across()),
            unit(frame.depth()),
        );
        let plane = |region: &WorldBox| {
            corners(region).map(|c| DVec2::new((c - origin).dot(x), (c - origin).dot(y)))
        };
        let side = plane(&common::part(design, "side_l").drawn_box());
        let panel = plane(&common::part(design, "door.panel").drawn_box());
        let fold = |points: &[DVec2], pick: fn(&DVec2) -> f64, max: bool| {
            points.iter().map(pick).fold(
                if max {
                    f64::NEG_INFINITY
                } else {
                    f64::INFINITY
                },
                if max { f64::max } else { f64::min },
            )
        };
        let x = |p: &DVec2| p.x;
        let y = |p: &DVec2| p.y;
        // The side lies at x <= 0: its inside face is its largest x. The
        // door's back face rests on the side's front.
        let origin = DVec2::new(fold(&side, x, true), fold(&panel, y, false));
        Swing {
            path,
            moving,
            links: links(mechanism),
            origin,
            edge: (fold(&panel, x, false) - origin.x) / 1e6,
            back: 0.0,
            front: (fold(&panel, y, true) - origin.y) / 1e6,
        }
    }

    /// Where a point of the door (doc frame, mm, closed) is when the door
    /// has opened by `degrees`.
    fn moved(&self, degrees: f64, point: DVec2) -> DVec2 {
        let pose = self
            .path
            .planar_pose(self.moving, degrees.to_radians())
            .expect("the mechanism reaches");
        let plane = point * 1e6 + self.origin;
        (pose.apply(plane) - self.origin) / 1e6
    }

    /// How far the links are from a dead point at `degrees` (mm): the two
    /// circles the boss must lie on, about `aᵢ − R(θ)·bᵢ` with the links'
    /// lengths, kept apart from touching inside and outside.
    fn reach_margin(&self, degrees: f64) -> f64 {
        let rotation = DVec2::from_angle(degrees.to_radians());
        let [(c1, l1), (c2, l2)] = [0, 1].map(|link| {
            let (arm, boss) = *self.links.get(link).expect("two links");
            (arm - rotation.rotate(boss), (boss - arm).length())
        });
        let distance = (c2 - c1).length();
        (l1 + l2 - distance).min(distance - (l1 - l2).abs()) / 1e6
    }
}

/// Each link of a four-bar mechanism (a member pinned once to a held
/// member and once to the moving one): its two pins, held first.
fn links(mechanism: &Mechanism) -> Vec<(DVec2, DVec2)> {
    let moving = mechanism.moving();
    let held = |index: MemberIndex| {
        mechanism
            .members()
            .get(mechanism.piece_of(index).index())
            .is_some_and(|member| member.holding == Holding::Held)
    };
    let pivots: Vec<(MemberIndex, MemberIndex, FramePoint)> = mechanism
        .entries()
        .iter()
        .filter_map(|entry| match *entry {
            MechanismEntry::Pivot { first, second, at } => Some((first, second, at)),
            MechanismEntry::Slot { .. }
            | MechanismEntry::Rail { .. }
            | MechanismEntry::Couple { .. } => None,
        })
        .collect();
    let pin = |link: MemberIndex, other: &dyn Fn(MemberIndex) -> bool| {
        pivots.iter().find_map(|(a, b, at)| {
            ((*a == link && other(*b)) || (*b == link && other(*a))).then(|| planar(*at))
        })
    };
    (0..mechanism.members().len())
        .filter_map(MemberIndex::new)
        .filter(|index| {
            *index != moving
                && mechanism
                    .members()
                    .get(index.index())
                    .is_some_and(|member| member.holding == Holding::Free)
        })
        .filter_map(|link| Some((pin(link, &held)?, pin(link, &|m| m == moving)?)))
        .collect()
}

/// The angles the script sampled, every 0.05deg from 0 to `last` degrees.
fn angles(last: u32) -> impl Iterator<Item = f64> {
    (0..=last * 20).map(|k| f64::from(k) * 0.05)
}

/// The door thicknesses the script checked for "nothing in wood".
const DOORS: [i64; 5] = [16, 18, 19, 20, 22];
/// The boring distances, tenths of a millimetre.
const CUP_INSETS: [i64; 5] = [30, 40, 50, 60, 70];

/// Closed, the door is exactly where it is placed: no float is involved
/// at 0deg, and the door's hinge edge overlays the side by exactly the
/// overlay, its back face exactly on the side's front.
#[test]
fn closed_is_exact() {
    for thickness in DOORS {
        for cup_inset in CUP_INSETS {
            let overlay = catalogue_overlay(cup_inset);
            let opened = open_clean(thickness, cup_inset, overlay);
            let design = common::design(&opened);
            let swing = Swing::of(design);
            assert_eq!(
                swing.path.planar_pose(swing.moving, 0.0),
                Some(stave_geom::planar::PlanarPose::REST)
            );
            assert_eq!(
                swing.path.matrix(swing.moving, 0.0),
                Some(glam::DMat4::IDENTITY)
            );
            let side = common::part(design, "side_l").drawn_box();
            let panel = common::part(design, "door.panel").drawn_box();
            assert_eq!(
                side.max().across.nm() - panel.min().across.nm(),
                overlay * TENTH
            );
            assert_eq!(panel.max().deep, side.min().deep);
            assert_eq!(
                panel.max().deep.nm() - panel.min().deep.nm(),
                Length::from_mm(thickness).nm()
            );
        }
    }
}

/// The links are 22.58mm and 22.92mm pin to pin, and from closed to past
/// 110deg the two circles the boss lies on stay at least a millimetre
/// from touching: 1.02mm at their closest (at 5.4deg), and still more
/// than a millimetre at 130deg, where the script stopped looking.
#[test]
fn the_links_reach_past_110_degrees_with_a_millimetre_to_spare() {
    let opened = open_clean(19, 50, catalogue_overlay(50));
    let swing = Swing::of(common::design(&opened));
    let lengths = [0, 1].map(|link| {
        let (arm, boss) = swing.links[link];
        (boss - arm).length() / 1e6
    });

    assert!((lengths[0] - 22.584).abs() < 0.01, "{lengths:?}");
    assert!((lengths[1] - 22.923).abs() < 0.01, "{lengths:?}");
    let (closest, at) = angles(110)
        .map(|degrees| (swing.reach_margin(degrees), degrees))
        .fold((f64::INFINITY, 0.0), |a, b| if b.0 < a.0 { b } else { a });
    assert!(closest >= 1.0, "{closest:.3}mm at {at}deg");
    assert!((closest - 1.018).abs() < 0.01, "{closest:.3}mm at {at}deg");
    assert!((at - 5.4).abs() < 0.06, "{closest:.3}mm at {at}deg");
    let first_dead = angles(130).find(|&degrees| swing.reach_margin(degrees) < 1.0);
    assert_eq!(
        first_dead, None,
        "a millimetre from a dead point before 130deg"
    );
    assert!((swing.reach_margin(130.0) - 11.480).abs() < 0.01);
}

/// The catalogue's overlay, boring distance + 12.5mm on a 0mm plate, holds
/// for every boring distance by construction: the cup's centre is always
/// 5mm in from the side's inside face, `C + 17.5mm` from the door's edge.
#[test]
fn the_overlay_is_the_catalogues_for_every_boring_distance() {
    for cup_inset in CUP_INSETS {
        let overlay = catalogue_overlay(cup_inset);
        let opened = open_clean(19, cup_inset, overlay);
        let design = common::design(&opened);
        let side = common::part(design, "side_l").drawn_box();
        let panel = common::part(design, "door.panel").drawn_box();
        for hinge in ["door.hinges[bottom]", "door.hinges[top]"] {
            let cup = common::hardware(design, hinge).bounds;
            let doubled_centre = cup.min().across.nm() + cup.max().across.nm();
            let (face, edge) = (side.max().across.nm(), panel.min().across.nm());
            assert_eq!(
                doubled_centre - 2 * face,
                2 * 50 * TENTH,
                "{hinge}: the cup's centre 5mm in from the side"
            );
            assert_eq!(
                doubled_centre - 2 * edge,
                2 * (cup_inset + 175) * TENTH,
                "{hinge}: the cup's centre C + 17.5mm from the door's edge"
            );
            assert_eq!(face - edge, overlay * TENTH, "{hinge}: overlay C + 12.5mm");
        }
    }
}

/// The door thicknesses of the catalogue's minimum reveal table.
const REVEAL_DOORS: [i64; 10] = [15, 16, 17, 18, 19, 20, 21, 22, 23, 24];

/// Hettich Sensys 8645i, minimum reveal per door (edge radius 1mm), by
/// boring distance (rows, 3 to 7mm) and door thickness (columns).
const CATALOGUE_REVEAL: [[f64; 10]; 5] = [
    [0.2, 0.3, 0.4, 0.6, 0.8, 1.1, 1.4, 2.2, 3.0, 3.9],
    [0.2, 0.3, 0.4, 0.6, 0.8, 1.0, 1.3, 1.8, 2.6, 3.4],
    [0.2, 0.3, 0.4, 0.6, 0.8, 1.0, 1.3, 1.6, 2.2, 3.0],
    [0.2, 0.3, 0.4, 0.6, 0.7, 1.0, 1.2, 1.6, 2.0, 2.6],
    [0.2, 0.3, 0.4, 0.5, 0.7, 0.9, 1.2, 1.5, 1.9, 2.4],
];

/// The swing-out the script measured for the same table (mm).
const MEASURED_REVEAL: [[f64; 10]; 5] = [
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

/// The minimum reveal: how far the door's hinge edge (rounded over by
/// 1mm) swings out past where it was, from closed to 110deg. It is the
/// script's to 0.01mm in every cell of the table, and for 18 to 20mm
/// doors within 0.25mm of the catalogue (20mm, C = 3: 0.85mm against
/// 1.1mm); thinner doors swing out a little more than the table, thicker
/// ones less. (23mm and 24mm doors are measured, not required to be
/// clear: with C = 7 they reach the side just before 110deg, which the
/// pipeline reports; the script only measured them too.)
#[test]
fn the_hinge_edge_swings_out_as_the_catalogue_says() {
    let mut worst = 0.0f64;
    for (row, cup_inset) in CUP_INSETS.into_iter().enumerate() {
        for (column, thickness) in REVEAL_DOORS.into_iter().enumerate() {
            let opened = if thickness <= 22 {
                open_clean(thickness, cup_inset, catalogue_overlay(cup_inset))
            } else {
                open_cabinet(thickness, cup_inset, catalogue_overlay(cup_inset))
            };
            let swing = Swing::of(common::solved(&opened));
            let corner = DVec2::new(swing.edge + 1.0, swing.front - 1.0);
            let leftmost = angles(110)
                .map(|degrees| swing.moved(degrees, corner).x - 1.0)
                .fold(f64::INFINITY, f64::min);
            let reveal = swing.edge - leftmost;
            let measured = MEASURED_REVEAL[row][column];
            assert!(
                (reveal - measured).abs() < 0.01,
                "door {thickness}mm, C {}: {reveal:.4}mm, the script had {measured}mm",
                written(cup_inset)
            );
            if (18..=20).contains(&thickness) {
                worst = worst.max((reveal - CATALOGUE_REVEAL[row][column]).abs());
            }
        }
    }
    assert!(
        (worst - 0.25).abs() < 0.005,
        "largest difference for 18-20mm doors {worst:.4}mm"
    );
}

/// At 110deg the door's back hinge corner stands 8.6mm into the opening
/// (D = 0, C = 3): the catalogue's door protrusion T is 8.5mm.
#[test]
fn the_door_stands_8_6mm_into_the_opening_at_110_degrees() {
    let opened = open_clean(19, 30, catalogue_overlay(30));
    let swing = Swing::of(common::design(&opened));
    let protrusion = swing.moved(110.0, DVec2::new(swing.edge, swing.back)).x;
    assert!((protrusion - 8.641).abs() < 0.01, "{protrusion:.3}mm");
    assert!((protrusion - 8.5).abs() <= 2.0, "{protrusion:.3}mm");
}

/// The side screw's settings the script checked, tenths of a millimetre:
/// the screw moves the arm (and the door with it) across, so the door
/// overlays the side by `C + 12.5mm − screw`.
const SCREWS: [i64; 6] = [-20, -10, 0, 5, 10, 20];

/// Nothing passes through wood from closed to fully open: the door
/// against the side, the hinge's arm, knuckle and links against the door
/// (less only the cup's bore) and the side, for doors 16 to 22mm thick,
/// boring distances 3 to 7mm and the side screw anywhere in its ±2mm. The
/// pipeline's own checks say so: every design is clean.
#[test]
fn nothing_passes_through_wood_at_any_side_screw_setting() {
    for screw in SCREWS {
        for cup_inset in CUP_INSETS {
            for thickness in DOORS {
                let overlay = catalogue_overlay(cup_inset) - screw;
                let opened = open_clean(thickness, cup_inset, overlay);
                let design = common::design(&opened);
                assert!(
                    design.assemblies.iter().any(|a| a.mechanism.is_some()),
                    "the door moves"
                );
            }
        }
    }
}

/// The example cabinet's door (19mm, 18mm overlay, C = 5, so the side
/// screw at +0.5mm): its front and back hinge corners at five angles,
/// moving forward and out, as the doc's table has them.
#[test]
fn the_front_edge_moves_forward_and_out() {
    let opened = open_clean(19, 50, 180);
    let swing = Swing::of(common::design(&opened));
    assert!((swing.edge + 18.0).abs() < 1e-9);
    assert!((swing.front - 19.0).abs() < 1e-9);
    let expected = [
        (0.0, (-18.0, 19.0), (-18.0, 0.0)),
        (30.0, (-12.1145, 33.0220), (-2.6145, 16.5675)),
        (60.0, (-13.9273, 21.7794), (2.5272, 12.2794)),
        (90.0, (-12.4590, 9.7304), (6.5410, 9.7304)),
        (110.0, (-9.0292, 2.9528), (8.8250, 9.4512)),
    ];
    for (degrees, front, back) in expected {
        for (corner, (x, y)) in [(swing.front, front), (swing.back, back)] {
            let moved = swing.moved(degrees, DVec2::new(swing.edge, corner));
            assert!(
                (moved.x - x).abs() < 0.01 && (moved.y - y).abs() < 0.01,
                "{degrees}deg: ({:.4}, {:.4}), expected ({x}, {y})",
                moved.x,
                moved.y
            );
        }
    }
}

/// Refitting the cup hinge's pins: the fit that found the pins of
/// `euro_hinge` in `std/hardware/hinges.stave`, as a tool run by hand.
///
/// ```text
/// cargo test --release -p stave-app --test oracle_hinge -- --ignored --nocapture refit_the_cup_hinge
/// ```
///
/// `STAVE_REFIT_GENERATIONS` (default 300) and `STAVE_REFIT_SEED`
/// (default 1) choose the run. It is a model of the mechanism in
/// millimetres, not the pipeline: the side and the door as half-planes,
/// only the cup's bore taken out of the door, every link sampled along
/// both its faces (and half its thickness past each pin) with the bore's
/// edges at the door's face tested inside each bar, the arm's body and
/// knuckle sampled on a grid.
mod refit {
    use super::DVec2;

    /// The sizes of `euro_hinge`'s members in `std/hardware/hinges.stave`,
    /// in millimetres: the arm's body and knuckle across the door, the
    /// knuckle's front, the arm's width, the links' thickness and widths.
    const ARM_ACROSS: (f64, f64) = (-2.90, 15.00);
    const KNUCKLE_ACROSS: (f64, f64) = (8.00, 15.00);
    const KNUCKLE_FRONT: f64 = 12.40;
    const ARM_WIDTH: f64 = 16.0;
    const LINK_THICKNESS: f64 = 1.50;
    const LINK_WIDTHS: [f64; 2] = [9.0, 14.0];
    /// The pins of `euro_hinge`'s mechanism: inner arm, outer arm, inner
    /// boss, outer boss (across, depth).
    const PINS: [f64; 8] = [13.50, 10.90, 13.28, 3.56, -8.80, 7.33, -9.54, 1.39];

    const CUP_RADIUS: f64 = 17.5;
    const CUP_DEPTH: f64 = 12.8;
    const BASE_OVERLAY: f64 = 12.5;
    /// The cup's centre from the side's face (D = 0, middle of the screw).
    const CUP_X: f64 = 5.0;
    /// The cabinet side's thickness.
    const SIDE: f64 = 19.0;
    /// The back of the plate (37mm setback, 36mm plate).
    const ARM_BACK: f64 = -55.0;
    /// Steel keeps this far off the wood, so rounding the pins never puts
    /// it in.
    const STEEL_CLEAR: f64 = 0.1;
    const PROTRUSION: f64 = 8.5;
    /// The fit's grid: the extremes of side screw, boring distance and
    /// door thickness.
    const SCREWS: [f64; 3] = [-2.0, 0.0, 2.0];
    const CUP_INSETS: [f64; 3] = [3.0, 5.0, 7.0];
    const DOORS: [f64; 3] = [16.0, 19.0, 22.0];

    /// The mechanism's fixed sizes (everything but the pins), in mm.
    struct Sizes {
        arm_x: (f64, f64),
        knuckle_x: (f64, f64),
        knuckle_front: f64,
        arm_half_width: f64,
        link_thickness: f64,
        link_half_width: [f64; 2],
    }

    fn sizes() -> Sizes {
        Sizes {
            arm_x: ARM_ACROSS,
            knuckle_x: KNUCKLE_ACROSS,
            knuckle_front: KNUCKLE_FRONT,
            arm_half_width: ARM_WIDTH / 2.0,
            link_thickness: LINK_THICKNESS,
            link_half_width: LINK_WIDTHS.map(|width| width / 2.0),
        }
    }

    /// Today's pins: inner arm, outer arm, inner boss, outer boss (x, y).
    fn today() -> [f64; 8] {
        PINS
    }

    /// The cup's half-width across the door, `half_width` off its centre
    /// line along the pins.
    fn chord(half_width: f64) -> f64 {
        (CUP_RADIUS * CUP_RADIUS - half_width * half_width).sqrt()
    }

    /// Angles from 0 to `last` degrees every `step`, in radians.
    fn range(last: f64, step: f64) -> Vec<f64> {
        let count = (last / step).round() as u32;
        (0..=count)
            .map(|k| (f64::from(k) * step).to_radians())
            .collect()
    }

    /// The four-bar solved at each angle: the boss's offset after turning
    /// by the angle, and how far the links are from a dead point; the
    /// branch is the one that rests at offset zero (`angles[0]` is 0).
    fn solve(arm: [DVec2; 2], boss: [DVec2; 2], angles: &[f64]) -> Vec<(DVec2, f64)> {
        let lengths = [(boss[0] - arm[0]).length(), (boss[1] - arm[1]).length()];
        let rest_between = (arm[1] - boss[1]) - (arm[0] - boss[0]);
        let rest_side = rest_between.perp_dot(-(arm[0] - boss[0])).signum();
        angles
            .iter()
            .map(|&theta| {
                let rotation = DVec2::from_angle(theta);
                let c1 = arm[0] - rotation.rotate(boss[0]);
                let c2 = arm[1] - rotation.rotate(boss[1]);
                let between = c2 - c1;
                let distance = between.length();
                let margin = (lengths[0] + lengths[1] - distance)
                    .min(distance - (lengths[0] - lengths[1]).abs());
                let along = (lengths[0] * lengths[0] - lengths[1] * lengths[1]
                    + distance * distance)
                    / (2.0 * distance);
                let across = (lengths[0] * lengths[0] - along * along).max(0.0).sqrt();
                let unit = between / distance;
                (
                    c1 + unit * along + unit.perp() * (rest_side * across),
                    margin,
                )
            })
            .collect()
    }

    /// How deep a point is in the door's wood (> 0 in), the cup's bore
    /// alone removed, with the door turned by `theta` and moved by
    /// `offset`.
    fn in_door(p: DVec2, offset: DVec2, theta: f64, door: (f64, f64, f64), half_width: f64) -> f64 {
        let (thickness, overlay, cup_x) = door;
        let q = DVec2::from_angle(-theta).rotate(p - offset);
        let inside = q.y.min(thickness - q.y).min(q.x + overlay);
        let in_cup = (chord(half_width) - (q.x - cup_x).abs()).min(CUP_DEPTH - q.y);
        inside.min(-in_cup)
    }

    /// How deep a point is in the side's wood (x in [-19, 0], y <= 0).
    fn in_side(p: DVec2) -> f64 {
        (-p.x).min(-p.y).min(p.x + SIDE)
    }

    /// `n` values evenly from `low` to `high`.
    fn spaced(low: f64, high: f64, n: u32) -> impl Iterator<Item = f64> {
        (0..n).map(move |k| low + (high - low) * f64::from(k) / f64::from(n - 1))
    }

    /// `(violation, error)`: the violation is 0 when the pins are
    /// feasible, and only then is the error the fit to the catalogue.
    fn evaluate(v: &[f64; 8], sizes: &Sizes) -> (f64, f64) {
        let pins = |shift: f64| {
            (
                [
                    DVec2::new(v[0] + shift, v[1]),
                    DVec2::new(v[2] + shift, v[3]),
                ],
                [
                    DVec2::new(v[4] + shift, v[5]),
                    DVec2::new(v[6] + shift, v[7]),
                ],
            )
        };
        let theta = range(110.0, 0.5);
        let wide = range(125.0, 1.0);
        let mut violation = 0.0;
        // Assembles to 125 degrees, 1mm from a dead point.
        let (arm, boss) = pins(CUP_X);
        if (boss[0] - arm[0]).length().min((boss[1] - arm[1]).length()) < 8.0 {
            return (1e3, 0.0);
        }
        let margins: Vec<f64> = solve(arm, boss, &wide).iter().map(|s| s.1).collect();
        if !margins.iter().all(|m| m.is_finite()) {
            return (1e3, 0.0);
        }
        let least = margins.iter().copied().fold(f64::INFINITY, f64::min);
        violation += (1.0 - least).max(0.0) * 10.0;
        let mut worst = 0.0f64;
        for screw in SCREWS {
            let cup_x = CUP_X + screw;
            let (arm, boss) = pins(cup_x);
            let solved = solve(arm, boss, &theta);
            if !solved.iter().all(|(o, _)| o.is_finite()) {
                return (1e3, 0.0);
            }
            let at = |k: usize| (theta[k], solved[k].0);
            let moved = |k: usize, q: DVec2| {
                let (angle, offset) = at(k);
                DVec2::from_angle(angle).rotate(q) + offset
            };
            for c in CUP_INSETS {
                let overlay = c + BASE_OVERLAY - screw;
                for t in DOORS {
                    let door = (t, overlay, cup_x);
                    let mut depth = f64::NEG_INFINITY;
                    let mut steel = f64::NEG_INFINITY;
                    // The door against the side and the arm.
                    let (lo, hi) = (cup_x + sizes.arm_x.0, cup_x + sizes.arm_x.1);
                    for q in [
                        (-overlay, 0.0),
                        (-overlay, t),
                        (-overlay + 5.0, 0.0),
                        (0.0, 0.0),
                        (cup_x - CUP_RADIUS, 0.0),
                        (cup_x + CUP_RADIUS, 0.0),
                        (40.0, 0.0),
                        (400.0, 0.0),
                    ] {
                        for k in 0..theta.len() {
                            let p = moved(k, DVec2::from(q));
                            let in_arm = (p.x - lo).min(hi - p.x).min((p.y - ARM_BACK).min(-p.y));
                            depth = depth.max(in_side(p)).max(in_arm);
                        }
                    }
                    // Each link's bar, both faces, against the side and the
                    // door; then the bore's edges at the door's face inside
                    // the bar (sampling the faces alone misses a corner
                    // poking in).
                    for link in 0..2 {
                        let half_width = sizes.link_half_width[link];
                        for k in 0..theta.len() {
                            let (angle, offset) = at(k);
                            let direction = moved(k, boss[link]) - arm[link];
                            let normal = direction.perp().normalize();
                            for u in spaced(-0.04, 1.04, 55) {
                                for s in [-0.5, 0.5] {
                                    let p = arm[link]
                                        + direction * u
                                        + normal * (s * sizes.link_thickness);
                                    steel = steel
                                        .max(in_side(p))
                                        .max(in_door(p, offset, angle, door, half_width));
                                }
                            }
                            let length = direction.length();
                            let unit = direction / length;
                            let across_unit = unit.perp();
                            let half = sizes.link_thickness / 2.0;
                            for x in [cup_x - chord(half_width), cup_x + chord(half_width)] {
                                let relative = moved(k, DVec2::new(x, 0.0)) - arm[link];
                                let along = relative.dot(unit);
                                let across = relative.dot(across_unit);
                                steel = steel.max(
                                    (along + half)
                                        .min(length + half - along)
                                        .min(half - across.abs()),
                                );
                            }
                        }
                    }
                    // The arm's body and knuckle against the door.
                    for ((low, high), (bottom, top)) in [
                        (sizes.arm_x, (-8.0, 0.0)),
                        (sizes.knuckle_x, (0.0, sizes.knuckle_front)),
                    ] {
                        for x in spaced(cup_x + low, cup_x + high, 9) {
                            for y in spaced(bottom, top, 7) {
                                for k in 0..theta.len() {
                                    let (angle, offset) = at(k);
                                    steel = steel.max(in_door(
                                        DVec2::new(x, y),
                                        offset,
                                        angle,
                                        door,
                                        sizes.arm_half_width,
                                    ));
                                }
                            }
                        }
                    }
                    depth = depth.max(-(cup_x + sizes.arm_x.0));
                    // Wood against wood may touch (the door rests on the
                    // side); steel keeps STEEL_CLEAR off the wood.
                    worst = worst.max(depth).max(steel + STEEL_CLEAR);
                }
            }
        }
        violation += worst.max(0.0);
        if violation > 0.0 {
            return (violation, 0.0);
        }
        // The fit: the minimum reveal for 18-20mm doors, and the
        // protrusion.
        let (arm, boss) = pins(CUP_X);
        let solved = solve(arm, boss, &theta);
        let moved = |k: usize, q: DVec2| DVec2::from_angle(theta[k]).rotate(q) + solved[k].0;
        let mut error = 0.0;
        for (row, c) in [3.0, 4.0, 5.0, 6.0, 7.0].into_iter().enumerate() {
            let overlay = c + BASE_OVERLAY;
            for (column, t) in super::REVEAL_DOORS.into_iter().enumerate() {
                if !(18..=20).contains(&t) {
                    continue;
                }
                let corner = DVec2::new(-overlay + 1.0, t as f64 - 1.0);
                let leftmost = (0..theta.len())
                    .map(|k| moved(k, corner).x - 1.0)
                    .fold(f64::INFINITY, f64::min);
                let published = super::CATALOGUE_REVEAL[row][column];
                error += (-overlay - leftmost - published).powi(2);
            }
        }
        let overlay = 3.0 + BASE_OVERLAY;
        let protrusion = moved(theta.len() - 1, DVec2::new(-overlay, 0.0)).x;
        error += 0.1 * (protrusion - PROTRUSION).powi(2);
        (0.0, error)
    }

    /// A feasible candidate beats an infeasible one; two infeasible ones
    /// compare by how far they violate, two feasible ones by the fit.
    fn better(a: (f64, f64), b: (f64, f64)) -> bool {
        if a.0 > 0.0 || b.0 > 0.0 {
            a.0 < b.0
        } else {
            a.1 < b.1
        }
    }

    /// SplitMix64: a small seeded generator, so a run is repeatable.
    struct Random(u64);

    impl Random {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }

        /// Uniform in [0, 1).
        fn unit(&mut self) -> f64 {
            (self.next() >> 11) as f64 / (1u64 << 53) as f64
        }

        /// Uniform in [0, n).
        fn below(&mut self, n: usize) -> usize {
            (self.unit() * n as f64) as usize
        }

        /// Normal, by Box-Muller.
        fn normal(&mut self, deviation: f64) -> f64 {
            let (a, b) = (1.0 - self.unit(), self.unit());
            (-2.0 * a.ln()).sqrt() * (std::f64::consts::TAU * b).cos() * deviation
        }
    }

    fn variable(name: &str, default: u64) -> u64 {
        std::env::var(name)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    }

    /// Differential evolution with feasibility rules, started around
    /// today's pins (today's pins themselves are the first candidate).
    #[test]
    #[ignore = "a tool, run by hand: refits the cup hinge's pins"]
    fn refit_the_cup_hinge() {
        let generations = variable("STAVE_REFIT_GENERATIONS", 300);
        let mut random = Random(variable("STAVE_REFIT_SEED", 1));
        let sizes = sizes();
        // Boss pins inside the cup, 1.5mm clear of its wall and at least
        // 0.6mm in from its mouth and above its bottom; arm pins inside
        // the knuckle, 1.5mm clear of its sides.
        let (k0, k1, front) = (sizes.knuckle_x.0, sizes.knuckle_x.1, sizes.knuckle_front);
        let low = [k0 + 1.5, -6.0, k0 + 1.5, -6.0, -16.0, 0.6, -16.0, 0.6];
        let high = [
            k1 - 1.5,
            front - 1.5,
            k1 - 1.5,
            front - 1.5,
            16.0,
            CUP_DEPTH - 1.5,
            16.0,
            CUP_DEPTH - 1.5,
        ];
        let clip = |v: [f64; 8]| std::array::from_fn::<f64, 8, _>(|i| v[i].clamp(low[i], high[i]));
        let start = today();
        let size = 48;
        let mut population: Vec<[f64; 8]> = (0..size)
            .map(|i| {
                if i == 0 {
                    clip(start)
                } else {
                    clip(std::array::from_fn(|j| start[j] + random.normal(1.5)))
                }
            })
            .collect();
        let mut scores: Vec<(f64, f64)> = population.iter().map(|v| evaluate(v, &sizes)).collect();
        let best_of = |scores: &[(f64, f64)]| {
            (0..scores.len())
                .min_by(|&a, &b| scores[a].partial_cmp(&scores[b]).unwrap())
                .unwrap()
        };
        for generation in 0..generations {
            for i in 0..size {
                let mut picked = Vec::with_capacity(3);
                while picked.len() < 3 {
                    let j = random.below(size);
                    if j != i && !picked.contains(&j) {
                        picked.push(j);
                    }
                }
                let (a, b, c) = (
                    population[picked[0]],
                    population[picked[1]],
                    population[picked[2]],
                );
                let scale = 0.5 + 0.4 * random.unit();
                let mutant = clip(std::array::from_fn(|j| a[j] + scale * (b[j] - c[j])));
                let forced = random.below(8);
                let trial: [f64; 8] = std::array::from_fn(|j| {
                    if random.unit() < 0.8 || j == forced {
                        mutant[j]
                    } else {
                        population[i][j]
                    }
                });
                let score = evaluate(&trial, &sizes);
                if better(score, scores[i]) || score == scores[i] {
                    population[i] = trial;
                    scores[i] = score;
                }
            }
            let best = best_of(&scores);
            if generation % 20 == 0 || generation + 1 == generations {
                println!(
                    "generation {generation}: violation {:.3}, error {:.3}",
                    scores[best].0, scores[best].1
                );
            }
        }
        let best = best_of(&scores);
        let pins = population[best].map(|v| (v * 100.0).round() / 100.0);
        println!("arm pins (inner, outer): {:?}", &pins[..4]);
        println!("boss pins (inner, outer): {:?}", &pins[4..]);
        let (violation, error) = evaluate(&pins, &sizes);
        println!(
            "rounded to hundredths: violation {violation:.3e}, error {error:.4}; {}",
            if pins == today().map(|v| (v * 100.0).round() / 100.0) {
                "today's pins"
            } else {
                "not today's pins"
            }
        );
        assert_eq!(scores[best].0, 0.0, "no feasible pins found");
        assert!(scores[best].1 <= evaluate(&today(), &sizes).1);
    }

    /// The model is the script's: today's pins are feasible and score the
    /// fit error `tools/hinge-sim/fit.py` gave them, 0.26394.
    #[test]
    fn the_refit_scores_todays_pins_as_the_script_did() {
        let (violation, error) = evaluate(&today(), &sizes());
        assert_eq!(violation, 0.0);
        assert!((error - 0.263_943_477_209_452_3).abs() < 1e-9, "{error}");
    }
}
