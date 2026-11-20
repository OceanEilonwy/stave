//! Executable spec: shaped parts (grammar, "Shaped parts"): a leg tapered
//! on its inside faces and an apron with an arch sawn into its lower edge,
//! on the tapered table example and small projects.

mod common;

use std::path::Path;

use common::{design, mm, open, part};
use stave_geom::solid::Solid;
use stave_lang::placement::PartAxis;
use stave_lang::units::Length;
use stave_model::solved::{CutKind, CutPlace, CutSource, Depth, Mark, Part, SolvedDesign};

fn tapered_table() -> stave_app::Opened {
    let path = Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/tapered_table"
    ));
    stave_app::open(path).unwrap()
}

/// The part's own shape cuts: the axis each is marked across, and its
/// marks.
fn shape_cuts(part: &Part) -> Vec<(PartAxis, Vec<Mark>)> {
    part.cuts
        .iter()
        .filter(|cut| cut.source == CutSource::Shape { part: part.id } || is_mirrored(part, cut))
        .map(|cut| {
            let CutPlace::Side { across_axis, .. } = cut.place else {
                panic!("{}: a shape is marked on the part's side", part.name);
            };
            let CutKind::Outline {
                marks,
                depth,
                from_depth,
            } = &cut.kind
            else {
                panic!("{}: a shape is an outline", part.name);
            };
            assert_eq!(*depth, Depth::Through, "{}", part.name);
            assert_eq!(*from_depth, Length::ZERO, "{}", part.name);
            (across_axis, marks.clone())
        })
        .collect()
}

/// A mirror image's shape cuts name its source's part.
fn is_mirrored(part: &Part, cut: &stave_model::solved::Cut) -> bool {
    matches!(cut.source, CutSource::Shape { .. })
        && matches!(cut.place, CutPlace::Side { from, .. } if from == part.id)
}

/// Twice the area of each angled prism the geometry made in a part,
/// with the axis it is cut through.
fn prisms(part: &Part, design: &SolvedDesign) -> Vec<(PartAxis, i128)> {
    let solid = Solid::of(part, design);
    assert!(solid.misshapen().is_empty(), "{}", part.name);
    assert!(solid.skipped().is_empty(), "{}", part.name);
    solid
        .prisms()
        .iter()
        .map(|prism| (prism.face_axis(), prism.outline().twice_area().abs()))
        .collect()
}

fn nm2(mm_a: i128, mm_b: i128) -> i128 {
    mm_a * 1_000_000 * mm_b * 1_000_000
}

#[test]
fn the_tapered_table_checks_with_its_tapers_and_arches() {
    let opened = tapered_table();
    assert!(
        opened.diagnostics.is_empty(),
        "{:?}",
        opened
            .diagnostics
            .iter()
            .map(|d| &d.message)
            .collect::<Vec<_>>()
    );
    let design = design(&opened);
    for name in [
        "leg_front_left",
        "leg_front_right",
        "leg_back_left",
        "leg_back_right",
    ] {
        let leg = part(design, name);
        assert_eq!(shape_cuts(leg).len(), 2, "{name}");
        // The cut list sizes the square blank.
        assert_eq!(leg.cut.width, mm("45"), "{name}");
        assert_eq!(leg.cut.thickness, mm("45"), "{name}");
    }
    for name in ["apron_front", "apron_back"] {
        assert_eq!(shape_cuts(part(design, name)).len(), 1, "{name}");
    }
    for name in ["apron_left", "apron_right", "top"] {
        assert!(shape_cuts(part(design, name)).is_empty(), "{name}");
    }
}

#[test]
fn every_leg_is_tapered_on_its_own_inside_faces() {
    let opened = tapered_table();
    let design = design(&opened);
    // Each leg's inside faces point toward the middle of the table.
    for (name, inside) in [
        ("leg_front_left", ["right", "back"]),
        ("leg_front_right", ["left", "back"]),
        ("leg_back_left", ["right", "front"]),
        ("leg_back_right", ["left", "front"]),
    ] {
        let leg = part(design, name);
        let mut faces: Vec<&str> = shape_cuts(leg)
            .iter()
            .map(|(axis, marks)| {
                // The taper starts on the face it planes: the far end of
                // the axis (45mm) here, or the near end (0mm).
                let direction = leg.pose.orientation.direction(*axis);
                if marks[0].across == mm("45") {
                    direction.word()
                } else {
                    assert_eq!(marks[0].across, Length::ZERO, "{name}");
                    direction.opposite().word()
                }
            })
            .collect();
        faces.sort_unstable();
        let mut wanted = inside.to_vec();
        wanted.sort_unstable();
        assert_eq!(faces, wanted, "{name}");
    }
}

#[test]
fn a_taper_runs_from_where_the_leg_stays_square_to_its_foot() {
    let opened = tapered_table();
    let design = design(&opened);
    let leg = part(design, "leg_front_left");
    // Standing up, its length starts at the foot. It stays square for
    // 125mm below the top (741mm long), then loses 20mm on each inside face
    // by the foot.
    for (_, marks) in shape_cuts(leg) {
        assert_eq!(
            marks,
            vec![
                Mark {
                    along: mm("616"),
                    across: mm("45"),
                },
                Mark {
                    along: Length::ZERO,
                    across: mm("45"),
                },
                Mark {
                    along: Length::ZERO,
                    across: mm("25"),
                },
            ]
        );
    }
    // Drawn as two triangles, one through each axis across the length.
    let mut made = prisms(leg, design);
    made.sort_unstable();
    assert_eq!(
        made,
        vec![
            (PartAxis::Width, nm2(616, 20)),
            (PartAxis::Thickness, nm2(616, 20)),
        ]
    );
}

#[test]
fn a_taper_on_two_opposite_faces_is_shared_between_them() {
    let opened = open(
        "stock oak_45_sq { species: white_oak, form: solid, thickness: 45mm, width: 45mm }

part leg : oak_45_sq {
  length: 700mm
  shape:  tapered(to: 25mm, toward: bottom, on: [left, right])
  place {
    orient(length: up, width: right)
    against(base.top)
    flush(base.left)
    flush(base.front)
  }
}
",
    );
    let design = design(&opened);
    let leg = part(design, "leg");
    let cuts = shape_cuts(leg);
    assert_eq!(cuts.len(), 2);
    let mut feet: Vec<Length> = cuts
        .iter()
        .map(|(axis, marks)| {
            assert_eq!(*axis, PartAxis::Width);
            marks[2].across
        })
        .collect();
    feet.sort_unstable();
    // 10mm off each side leaves 25mm, centred.
    assert_eq!(feet, vec![mm("10"), mm("35")]);
}

#[test]
fn an_arch_rises_to_its_middle_and_leaves_its_ends_straight() {
    let opened = tapered_table();
    let design = design(&opened);
    let apron = part(design, "apron_front");
    let cuts = shape_cuts(apron);
    let [(axis, marks)] = cuts.as_slice() else {
        panic!("one arch: {cuts:?}");
    };
    // Standing on edge with its width up: the arch is across its width,
    // sawn through its thickness, rising from its lower edge.
    assert_eq!(*axis, PartAxis::Width);
    assert_eq!(marks.len(), 25);
    // The drawn apron is 760mm long between the legs, starting 20mm into
    // its cut length (the tenon); 40mm stays straight at each end.
    let first = marks.first().unwrap();
    let last = marks.last().unwrap();
    assert_eq!((first.along, first.across), (mm("60"), Length::ZERO));
    assert_eq!((last.along, last.across), (mm("740"), Length::ZERO));
    let middle = &marks[12];
    assert_eq!((middle.along, middle.across), (mm("400"), mm("30")));
    // Every corner is on or just inside its circle, never above the top.
    assert!(marks.iter().all(|mark| mark.across <= mm("30")));
    assert!(
        marks
            .windows(2)
            .take(12)
            .all(|pair| pair[0].across <= pair[1].across)
    );
    let made = prisms(apron, design);
    assert_eq!(made.len(), 1, "{made:?}");
}
