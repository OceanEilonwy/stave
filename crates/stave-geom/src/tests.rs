use glam::Vec3;
use stave_lang::placement::{Axis, Direction, PartAxis};
use stave_lang::units::Length;
use stave_model::solved::{
    End, Envelope, Hardware, HardwareBase, HardwareData, HardwareId, LocalFace, Orientation, Part,
    PartId, PartOrigin, Point, Pose, Shape, Size3, Station, Stock, WorldBox,
};
use stave_schema::species::Species;
use stave_schema::words::{Grain, Measure, StockForm};

use super::*;

fn mm(value: i64) -> Length {
    Length::from_mm(value)
}

fn size(length: i64, width: i64, thickness: i64) -> Size3 {
    Size3 {
        length: mm(length),
        width: mm(width),
        thickness: mm(thickness),
    }
}

fn point(across: i64, up: i64, deep: i64) -> Point {
    Point {
        across: mm(across),
        up: mm(up),
        deep: mm(deep),
    }
}

fn frame(length: Direction, width: Direction) -> Orientation {
    Orientation::from_length_and_width(length, width).unwrap()
}

fn part(orientation: Orientation, origin: Point, cut: Size3, shape: Shape) -> Part {
    Part {
        id: PartId::from_usize(0),
        name: "test".into(),
        assembly: None,
        origin: PartOrigin::Declared,
        stock: Stock {
            name: "oak_19".into(),
            species: Species::WhiteOak,
            form: StockForm::Solid,
            thickness: Some(cut.thickness),
            diameter: None,
            width: None,
            max_width: None,
            max_length: None,
        },
        drawn: cut,
        cut,
        drawn_offset: Size3::default(),
        pose: Pose {
            origin,
            orientation,
        },
        shape,
        grain: Grain::Length,
        reference_face: LocalFace {
            axis: PartAxis::Thickness,
            end: End::Start,
        },
        reference_edge: LocalFace {
            axis: PartAxis::Width,
            end: End::Start,
        },
        cuts: Vec::new(),
        joints: Vec::new(),
        note: None,
    }
}

fn expected(world: WorldBox) -> Aabb {
    Aabb::around([render::point(world.min()), render::point(world.max())]).unwrap()
}

fn assert_close(a: Aabb, b: Aabb) {
    assert!(
        (a.min - b.min).abs().max_element() < 1e-3 && (a.max - b.max).abs().max_element() < 1e-3,
        "{a:?} != {b:?}"
    );
}

/// Every triangle's winding agrees with its vertex normals, and the mesh
/// encloses a positive volume: together, the normals face outward.
fn assert_outward(mesh: &Mesh) {
    for &[a, b, c] in mesh.indices.as_chunks::<3>().0 {
        let vertex = |i: u32| mesh.vertices[i as usize];
        let [a, b, c] = [vertex(a), vertex(b), vertex(c)];
        let geometric = (Vec3::from(b.position) - Vec3::from(a.position))
            .cross(Vec3::from(c.position) - Vec3::from(a.position));
        let normals = Vec3::from(a.normal) + Vec3::from(b.normal) + Vec3::from(c.normal);
        assert!(geometric.dot(normals) > 0.0, "a triangle faces inward");
    }
    assert!(mesh.signed_volume() > 0.0, "the mesh is inside out");
}

/// For a box, every face normal also points away from the centre.
fn assert_normals_leave_centre(geometry: &Geometry) {
    let centre = geometry.bounds.centre();
    for vertex in &geometry.mesh.vertices {
        let outward = Vec3::from(vertex.position) - centre;
        assert!(outward.dot(Vec3::from(vertex.normal)) > 0.0);
    }
}

#[test]
fn a_side_board_matches_its_cut_box() {
    // The cabinet's left side: length up, width back.
    let side = part(
        frame(Direction::Up, Direction::Back),
        point(0, 19, 0),
        size(1662, 560, 19),
        Shape::Board,
    );
    let geometry = super::part(&side, Extent::Cut);
    assert_close(geometry.bounds, expected(side.cut_box()));
    assert_eq!(geometry.mesh.vertices.len(), 24);
    assert_eq!(geometry.mesh.triangle_count(), 12);
    assert_eq!(geometry.edges.segments.len(), 12);
    assert_outward(&geometry.mesh);
    assert_normals_leave_centre(&geometry);
    let volume = 1662.0 * 560.0 * 19.0;
    assert!((geometry.mesh.signed_volume() - volume).abs() / volume < 1e-4);
}

#[test]
fn a_mirrored_board_still_faces_outward() {
    let source = frame(Direction::Up, Direction::Back);
    let mirrored = source.mirrored(Axis::Across);
    assert_ne!(source.is_reflection(), mirrored.is_reflection());
    let side = part(
        mirrored,
        point(900, 19, 0),
        size(1662, 560, 19),
        Shape::Board,
    );
    let geometry = super::part(&side, Extent::Cut);
    assert_close(geometry.bounds, expected(side.cut_box()));
    assert!(geometry.bounds.min.x > 880.0 && geometry.bounds.max.x <= 900.0 + 1e-3);
    assert_outward(&geometry.mesh);
    assert_normals_leave_centre(&geometry);
}

#[test]
fn every_frame_faces_outward() {
    for length in Direction::ALL {
        for width in Direction::ALL {
            let Ok(orientation) = Orientation::from_length_and_width(length, width) else {
                continue;
            };
            for orientation in [orientation, orientation.mirrored(Axis::Deep)] {
                let board = part(
                    orientation,
                    point(10, 20, 30),
                    size(300, 100, 19),
                    Shape::Board,
                );
                let geometry = super::part(&board, Extent::Cut);
                assert_close(geometry.bounds, expected(board.cut_box()));
                assert_outward(&geometry.mesh);
                assert_normals_leave_centre(&geometry);
            }
        }
    }
}

#[test]
fn the_drawn_extent_leaves_out_what_joints_add() {
    // A divider 862 drawn, 880 cut: 9mm tongues at both ends.
    let mut divider = part(
        frame(Direction::Right, Direction::Back),
        point(19, 519, 0),
        size(880, 554, 19),
        Shape::Board,
    );
    divider.drawn = size(862, 554, 19);
    divider.drawn_offset = size(9, 0, 0);
    divider.pose.origin.across = mm(10);
    let cut = super::part(&divider, Extent::Cut);
    let drawn = super::part(&divider, Extent::Drawn);
    assert_close(cut.bounds, expected(divider.cut_box()));
    assert_close(drawn.bounds, expected(divider.drawn_box()));
    assert!((drawn.bounds.min.x - 19.0).abs() < 1e-3);
}

fn bun_foot(smooth: bool) -> Part {
    let mut foot = part(
        frame(Direction::Down, Direction::Right),
        point(60, 0, 60),
        size(96, 70, 70),
        Shape::Turned {
            profile: "bun_foot".into(),
            stations: vec![
                Station {
                    at: mm(0),
                    diameter: mm(70),
                },
                Station {
                    at: mm(34),
                    diameter: mm(70),
                },
                Station {
                    at: mm(80),
                    diameter: mm(40),
                },
            ],
            smooth,
            measure: Measure::Diameter,
        },
    );
    foot.drawn = size(80, 70, 70);
    foot.drawn_offset = size(16, 0, 0);
    // Station 0 at the base's underside (up = 0): the cut box starts 16mm
    // higher, inside the base.
    foot.pose.origin.up = mm(16);
    foot
}

#[test]
fn a_turned_foot_is_a_lathe_shape() {
    for smooth in [false, true] {
        let foot = bun_foot(smooth);
        let drawn = super::part(&foot, Extent::Drawn);
        // Bounds: the drawn length downward from 0, 70 across.
        assert!((drawn.bounds.max.y - 0.0).abs() < 1e-3);
        assert!((drawn.bounds.min.y + 80.0).abs() < 1e-3);
        assert!((drawn.bounds.size().x - 70.0).abs() < 0.1);
        assert!((drawn.bounds.centre().x - 95.0).abs() < 1e-3);
        assert!((drawn.bounds.centre().z + 95.0).abs() < 1e-3);
        assert_outward(&drawn.mesh);
        // Volume close to the straight-line profile: a cylinder and a
        // frustum (the smooth curve bulges a little).
        let pi = std::f32::consts::PI;
        let cylinder = pi * 35.0 * 35.0 * 34.0;
        let frustum = pi * 46.0 / 3.0 * (35.0 * 35.0 + 35.0 * 20.0 + 20.0 * 20.0);
        let ratio = drawn.mesh.signed_volume() / (cylinder + frustum);
        assert!((0.97..1.06).contains(&ratio), "volume ratio {ratio}");
        // Sane counts: a few thousand vertices at most.
        let segments = usize::from(LATHE_SEGMENTS);
        assert!(drawn.mesh.vertices.len() < 64 * segments);
        assert!(drawn.mesh.triangle_count() >= 4 * segments);
        // Outlines: both ends, plus the corner at 34mm when unsmoothed.
        let rings = drawn.edges.segments.len() / segments;
        assert_eq!(rings, if smooth { 2 } else { 3 }, "smooth: {smooth}");

        let cut = super::part(&foot, Extent::Cut);
        assert_close(cut.bounds, expected(foot.cut_box()));
        assert_outward(&cut.mesh);
    }
}

#[test]
fn a_dowel_is_a_cylinder() {
    let rod = part(
        frame(Direction::Right, Direction::Back),
        point(10, 1424, 263),
        size(880, 28, 28),
        Shape::Round { diameter: mm(28) },
    );
    let geometry = super::part(&rod, Extent::Cut);
    assert_close(geometry.bounds, expected(rod.cut_box()));
    assert_outward(&geometry.mesh);
    let volume = std::f32::consts::PI * 14.0 * 14.0 * 880.0;
    assert!((geometry.mesh.signed_volume() / volume - 1.0).abs() < 0.01);
}

fn knob(orientation: Orientation, origin: Point, envelope: Envelope) -> Hardware {
    let pose = Pose {
        origin,
        orientation,
    };
    Hardware {
        id: HardwareId::from_usize(0),
        name: "handle".into(),
        kind: "knob".into(),
        base: HardwareBase::Pull,
        assembly: None,
        piece: PartId::from_usize(0),
        mount_face: stave_lang::placement::Face::Front,
        envelope,
        pose,
        bounds: pose.world_box(envelope.size()),
        fixed: None,
        data: HardwareData::Pull {
            centres: None,
            diameter: Some(mm(26)),
            screw_diameter: mm(4),
        },
        mechanism: None,
        model: None,
    }
}

#[test]
fn hardware_envelopes_fill_their_bounds() {
    // A knob on a door's front face: thickness stands off toward the front.
    let standing_off = Orientation::new(Direction::Up, Direction::Right, Direction::Front).unwrap();
    let cylinder = knob(
        standing_off,
        point(406, 1091, -19),
        Envelope::Cylinder {
            diameter: mm(26),
            length: mm(30),
        },
    );
    let geometry = hardware(&cylinder);
    assert_close(geometry.bounds, expected(cylinder.bounds));
    assert_outward(&geometry.mesh);
    // The axis stands off the face: along +z, from -(-19) = 19 to 49.
    assert!((geometry.bounds.min.z - 19.0).abs() < 1e-3);
    assert!((geometry.bounds.max.z - 49.0).abs() < 1e-3);

    let mirrored = knob(
        standing_off.mirrored(Axis::Across),
        point(406, 1091, -19),
        Envelope::Box {
            length: mm(50),
            width: mm(65),
            thickness: mm(20),
        },
    );
    let geometry = hardware(&mirrored);
    assert_close(geometry.bounds, expected(mirrored.bounds));
    assert_outward(&geometry.mesh);
    assert_normals_leave_centre(&geometry);
}

#[test]
fn rays_enter_boxes() {
    let bounds = Aabb {
        min: Vec3::new(-1.0, -1.0, -1.0),
        max: Vec3::new(1.0, 1.0, 1.0),
    };
    assert_eq!(
        bounds.ray_entry(Vec3::new(0.0, 0.0, 5.0), Vec3::NEG_Z),
        Some(4.0)
    );
    assert_eq!(
        bounds.ray_entry(Vec3::new(3.0, 0.0, 5.0), Vec3::NEG_Z),
        None
    );
    assert_eq!(bounds.ray_entry(Vec3::ZERO, Vec3::X), Some(0.0));
}

// ---------------------------------------------------------------------------
// Cut solids
// ---------------------------------------------------------------------------

use stave_model::solved::{
    Cut, CutKind, CutPlace, CutSource, Depth, LocalBox, LocalPoint, ProjectInfo, SolvedDesign,
};
use stave_schema::words::OutputUnit;

/// A design holding only `part`, for the parts a cut is measured against.
fn design_of(part: &Part) -> SolvedDesign {
    SolvedDesign {
        project: ProjectInfo {
            title: "test".into(),
            goal: None,
            units: OutputUnit::Millimetre,
            resolution: Length::from_nm(500_000),
            style: None,
            tools: None,
            methods: Vec::new(),
        },
        parts: vec![part.clone()],
        hardware: Vec::new(),
        fasteners: Vec::new(),
        joints: Vec::new(),
        regions: Vec::new(),
        assemblies: Vec::new(),
        scenes: Vec::new(),
        requires: Vec::new(),
        overruns: Vec::new(),
    }
}

fn local_point(length: i64, width: i64, thickness: i64) -> LocalPoint {
    LocalPoint {
        length: mm(length),
        width: mm(width),
        thickness: mm(thickness),
    }
}

fn hardware_cut(face: LocalFace, place: CutPlace, kind: CutKind) -> Cut {
    Cut {
        source: CutSource::Hardware {
            hardware: HardwareId::from_usize(0),
        },
        face,
        place,
        kind,
    }
}

fn face(axis: PartAxis, end: End) -> LocalFace {
    LocalFace { axis, end }
}

/// A 100 × 50 × 20 board with the given cuts; its mesh encloses exactly
/// the solid's blocks.
fn cut_board(orientation: Orientation, cuts: Vec<Cut>) -> Geometry {
    let mut board = part(orientation, point(0, 0, 0), size(100, 50, 20), Shape::Board);
    board.cuts = cuts;
    let solid = solid::Solid::of(&board, &design_of(&board));
    let geometry = cut_part(&board, &solid);
    let expected = (solid.blocks().volume() as f64) * 1e-18;
    let volume = f64::from(geometry.mesh.signed_volume());
    assert!(
        (volume - expected).abs() <= expected * 1e-5,
        "volume {volume} != {expected}"
    );
    geometry
}

#[test]
fn a_plain_cut_board_is_twelve_triangles_and_edges() {
    let geometry = cut_board(frame(Direction::Right, Direction::Back), Vec::new());
    assert_eq!(geometry.mesh.triangle_count(), 12);
    assert_eq!(geometry.edges.segments.len(), 12);
    assert_outward(&geometry.mesh);
}

#[test]
fn a_dado_outlines_only_where_the_surface_bends() {
    // A housing straight across the width, in the top face.
    let dado = hardware_cut(
        face(PartAxis::Thickness, End::End),
        CutPlace::Recess {
            footprint: LocalBox {
                min: local_point(40, 0, 14),
                max: local_point(60, 50, 20),
            },
        },
        CutKind::Notch {
            depth: Depth::Length(mm(6)),
        },
    );
    for orientation in [
        frame(Direction::Right, Direction::Back),
        frame(Direction::Right, Direction::Back).mirrored(Axis::Across),
    ] {
        let geometry = cut_board(orientation, vec![dado.clone()]);
        assert_outward(&geometry.mesh);
        // The cross-section is an eight-sided outline swept across the
        // width: eight long edges and eight at each end, no seams.
        assert_eq!(geometry.edges.segments.len(), 24);
    }
}

#[test]
fn a_tenon_and_a_mortise_keep_their_volume() {
    let tenon = hardware_cut(
        face(PartAxis::Length, End::End),
        CutPlace::End {
            toward: PartId::from_usize(0),
        },
        CutKind::Tenon {
            length: mm(30),
            width: mm(30),
            thickness: mm(8),
        },
    );
    let pocket = hardware_cut(
        face(PartAxis::Thickness, End::End),
        CutPlace::Recess {
            footprint: LocalBox {
                min: local_point(10, 10, 0),
                max: local_point(40, 40, 20),
            },
        },
        CutKind::Pocket {
            width: mm(20),
            thickness: mm(10),
            depth: Depth::Length(mm(12)),
        },
    );
    let geometry = cut_board(frame(Direction::Up, Direction::Back), vec![tenon, pocket]);
    assert_outward(&geometry.mesh);
}

#[test]
fn a_hole_opens_in_its_face() {
    let mut board = part(
        frame(Direction::Right, Direction::Back),
        point(0, 0, 0),
        size(100, 50, 20),
        Shape::Board,
    );
    board.cuts = vec![hardware_cut(
        face(PartAxis::Thickness, End::End),
        CutPlace::Mount {
            centre: local_point(50, 25, 20),
            centres: None,
        },
        CutKind::Drill {
            diameter: mm(10),
            depth: Depth::Length(mm(8)),
            count: 1,
        },
    )];
    let solid = solid::Solid::of(&board, &design_of(&board));
    let geometry = cut_part(&board, &solid);
    assert_outward(&geometry.mesh);
    // The box less the hole's prism (a polygon of LATHE_SEGMENTS sides).
    let sides = f64::from(LATHE_SEGMENTS);
    let polygon = sides / 2.0 * 25.0 * (std::f64::consts::TAU / sides).sin();
    let expected = 100.0 * 50.0 * 20.0 - polygon * 8.0;
    let volume = f64::from(geometry.mesh.signed_volume());
    assert!((volume - expected).abs() < 0.5, "{volume} != {expected}");
    // The opening is outlined: a ring of LATHE_SEGMENTS segments on top of
    // the box's twelve edges.
    assert_eq!(
        geometry.edges.segments.len(),
        12 + usize::from(LATHE_SEGMENTS)
    );
}

/// Twice the area of the part of a rectangle `[0, w] × [0, h]` (sampled
/// on a fine grid) inside a region, in mm².
fn sampled_area(w: f64, h: f64, inside: impl Fn(f64, f64) -> bool) -> f64 {
    let step = 0.05;
    let mut count = 0u64;
    let mut x = step / 2.0;
    while x < w {
        let mut y = step / 2.0;
        while y < h {
            if inside(x, y) {
                count += 1;
            }
            y += step;
        }
        x += step;
    }
    count as f64 * step * step
}

#[test]
fn a_slot_at_an_angle_is_drawn_cut() {
    // A 100 × 50 × 20 board, and a stick standing through its middle; a
    // slot 10mm wide runs from the middle out on the diagonal, through.
    let mut board = part(
        frame(Direction::Right, Direction::Back),
        point(0, 0, 0),
        size(100, 50, 20),
        Shape::Board,
    );
    let mut stick = part(
        frame(Direction::Up, Direction::Back),
        point(35, -100, 10),
        size(300, 30, 30),
        Shape::Board,
    );
    stick.id = PartId::from_usize(1);
    board.cuts = vec![Cut {
        source: CutSource::Hardware {
            hardware: HardwareId::from_usize(0),
        },
        face: face(PartAxis::Thickness, End::End),
        place: CutPlace::Crossing {
            other: stick.id,
            footprint: LocalBox {
                min: local_point(35, 10, 0),
                max: local_point(65, 40, 20),
            },
        },
        kind: CutKind::Slot {
            width: mm(10),
            depth: Depth::Through,
            angle: stave_lang::units::Angle::from_deg(45),
            from_centre: true,
        },
    }];
    let mut design = design_of(&board);
    design.parts.push(stick);
    for orientation in [
        frame(Direction::Right, Direction::Back),
        frame(Direction::Right, Direction::Back).mirrored(Axis::Across),
    ] {
        board.pose.orientation = orientation;
        let solid = solid::Solid::of(&board, &design);
        assert_eq!(solid.slots().len(), 1);
        let geometry = cut_part(&board, &solid);
        assert_outward(&geometry.mesh);
        let band = mesh_cut::Band::of(&solid.slots()[0]);
        let cut = sampled_area(100.0, 50.0, |x, y| {
            band.holds(Vec3::new(x as f32, y as f32, 10.0))
        });
        let expected = (100.0 * 50.0 - cut) * 20.0;
        let volume = f64::from(geometry.mesh.signed_volume());
        assert!(
            (volume - expected).abs() < expected * 0.002,
            "{volume} != {expected}"
        );
        assert!(cut > 300.0, "the slot cuts {cut}mm²");
    }
}

#[test]
fn a_round_section_keeps_its_flats() {
    let mut stick = part(
        frame(Direction::Up, Direction::Back),
        point(0, 0, 0),
        size(300, 30, 30),
        Shape::Board,
    );
    let mut other = part(
        frame(Direction::Right, Direction::Back),
        point(-100, 135, 0),
        size(300, 30, 30),
        Shape::Board,
    );
    other.id = PartId::from_usize(1);
    stick.cuts = vec![Cut {
        source: CutSource::Hardware {
            hardware: HardwareId::from_usize(0),
        },
        face: face(PartAxis::Width, End::Start),
        place: CutPlace::Crossing {
            other: other.id,
            footprint: LocalBox {
                min: local_point(135, 0, 0),
                max: local_point(165, 30, 30),
            },
        },
        kind: CutKind::Round {
            diameter: Length::from_nm(22_500_000),
            flats: Some(Length::from_nm(13_500_000)),
            flats_angle: stave_lang::units::Angle::from_deg(45),
        },
    }];
    let mut design = design_of(&stick);
    design.parts.push(other);
    let solid = solid::Solid::of(&stick, &design);
    let geometry = cut_part(&stick, &solid);
    assert_outward(&geometry.mesh);
    let angle = std::f64::consts::FRAC_PI_4;
    let section = sampled_area(30.0, 30.0, |x, y| {
        let (dx, dy) = (x - 15.0, y - 15.0);
        dx * dx + dy * dy <= 11.25 * 11.25 && (dx * angle.cos() + dy * angle.sin()).abs() <= 6.75
    });
    let expected = 270.0 * 30.0 * 30.0 + section * 30.0;
    let volume = f64::from(geometry.mesh.signed_volume());
    assert!(
        (volume - expected).abs() < 30.0 * section * 0.01,
        "{volume} != {expected}"
    );
}
