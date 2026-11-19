//! Showing joint members (docs/reviews/2026-11-17-checks-gaps.md, stage
//! 4): a joint pair's members (a dowel's pins, a screwed joint's screws)
//! are drawn, pulled apart, played in focus mode, selected, inspected,
//! listed in the parts tree and posed for the problems about them.

use std::path::{Path, PathBuf};

use glam::Vec3;
use stave_app::check::{self, Checked, Outcome};
use stave_app::focus::{self, Options, Target};
use stave_app::inspect::{self, TreeEntry};
use stave_app::session::{Command, Event, ProblemIndex, Selection, Session};
use stave_app::view::{self, Item, SceneGraph};
use stave_model::solved::{PairMember, SolvedDesign};
use stave_scene::{Look, Material, NodeId};

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn checked(folder: &str) -> Checked {
    match check::run(&workspace().join(folder)) {
        Outcome::Checked(checked) => *checked,
        Outcome::CantOpen { .. } => panic!("{folder} can't be opened"),
    }
}

fn drawable(checked: &Checked) -> (&SolvedDesign, &SceneGraph) {
    checked.drawable().expect("it solves")
}

/// The member a joint lays out on the pair whose first part is `piece`.
fn member(design: &SolvedDesign, joint: &str, piece: &str) -> PairMember {
    design
        .pair_members()
        .find(|(member, _)| {
            let found = &design.joints()[member.joint.index()];
            found.name == joint
                && design
                    .part(found.pair(member.pair).unwrap().roles[0].part)
                    .unwrap()
                    .name
                    == piece
        })
        .map(|(member, _)| member)
        .unwrap_or_else(|| panic!("no member of '{joint}' on {piece}"))
}

fn node_of(graph: &SceneGraph, item: Item) -> NodeId {
    graph
        .node_of(item)
        .unwrap_or_else(|| panic!("no node for {item:?}"))
}

fn node_of_part(graph: &SceneGraph, design: &SolvedDesign, name: &str) -> NodeId {
    node_of(graph, Item::Part(design.part_named(name).unwrap().id))
}

/// Where a node's mesh is, posed.
fn centre(graph: &SceneGraph, posed: &stave_scene::Posed, node: NodeId) -> Vec3 {
    graph.bounds_of(posed, &[node]).unwrap().centre()
}

#[test]
fn the_cabinets_dowels_are_drawn_in_beech_and_its_screws_in_steel() {
    let checked = checked("examples/clothing_cabinet");
    let (design, graph) = drawable(&checked);
    let pins = node_of(graph, Item::Member(member(design, "top_dowels", "side_l")));
    let pins = graph.node(pins).unwrap();
    assert_eq!(pins.name, "top_dowels's pin between side_l and top");
    assert!(matches!(
        pins.material,
        Material::Wood(wood) if wood.species == stave_schema::species::Species::Beech
    ));
    // Four 8mm pins, 30mm long: their box is 8mm across, 30mm high.
    let size = pins.bounds().size();
    assert!((size.x - 8.0).abs() < 0.01 && (size.y - 30.0).abs() < 0.01);
    let screws = graph
        .nodes
        .iter()
        .find(|node| node.name == "drawer[top].front_screws's screw")
        .expect("the top drawer's front screws");
    assert_eq!(screws.material, Material::Metal);
}

#[test]
fn pulled_apart_a_pin_goes_with_the_part_that_carries_it() {
    let checked = checked("examples/bookshelf");
    let (design, graph) = drawable(&checked);
    let pins = node_of(graph, Item::Member(member(design, "kick_dowels", "kick")));
    let kick = node_of_part(graph, design, "kick");
    let side = node_of_part(graph, design, "side_l");
    assert_eq!(graph.explode_offset(pins), graph.explode_offset(kick));
    // The kick is held between the sides, which come off its ends: its
    // pins stay in its ends and come out of the sides' holes.
    let apart = graph.explode_offset(side) - graph.explode_offset(pins);
    assert!(apart.length() > 30.0, "{apart:?}");
}

#[test]
fn in_focus_the_pins_ride_on_the_piece_into_their_holes() {
    let checked = checked("examples/clothing_cabinet");
    let (design, graph) = drawable(&checked);
    let target = focus::find(design, "top_dowels", Some(0)).unwrap();
    let opened = &checked.opened;
    let focus = focus::open(opened, design, graph, target).unwrap();
    let scene = graph.scenes.first().unwrap();
    let pins = node_of(graph, Item::Member(member(design, "top_dowels", "side_l")));
    let side = node_of_part(graph, design, "side_l");
    let rest = graph.rest();
    // Apart, half way and together: the pins move exactly as side_l does.
    for progress in [0.0, 0.4, 1.0] {
        let frame = focus.frame(graph, scene, progress, &Options::default());
        let moved = |node| centre(graph, &frame.posed, node) - centre(graph, &rest, node);
        assert!(
            (moved(pins) - moved(side)).length() < 0.01,
            "at {progress}: pins {:?}, side {:?}",
            moved(pins),
            moved(side)
        );
        // Drawn as they are, not faded with the rest.
        assert_eq!(frame.looks[pins.index()], Look::Normal);
        if progress < 1.0 {
            assert!(moved(side).length() > 1.0, "side_l is apart at {progress}");
        } else {
            assert!(moved(pins).length() < 0.01, "home at the end");
        }
    }
    // The panel says what the joint places and counts its holes.
    let row = |label: &str| {
        focus
            .rows
            .iter()
            .filter(|row| row.label == label)
            .map(|row| row.value.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        row("Places"),
        ["4 × pin (dowel_pin, beech), cylinder 8 mm across, 30 mm long"]
    );
    assert_eq!(
        row("Hole"),
        [
            "4 × 8 mm across, 15 mm deep, in side_l",
            "4 × 8 mm across, 15 mm deep, in top"
        ]
    );
}

#[test]
fn a_pin_selected_says_which_joint_placed_it_and_where_it_is_written() {
    let checked = checked("examples/clothing_cabinet");
    let (design, graph) = drawable(&checked);
    let pins = member(design, "top_dowels", "side_r");
    // A click on its node selects it.
    let node = node_of(graph, Item::Member(pins));
    let selection = Selection::of_item(graph.node(node).unwrap().item);
    assert_eq!(selection, Selection::Member(pins));
    assert_eq!(selection.item(), Some(Item::Member(pins)));
    let inspection = inspect::inspect(&checked, &selection).unwrap();
    assert_eq!(inspection.eyebrow, "JOINT MEMBER");
    assert_eq!(inspection.title, "top_dowels's pin");
    let row = |label: &str| {
        inspection
            .rows
            .iter()
            .find(|row| row.label == label)
            .map(|row| row.value.as_str())
            .unwrap_or_else(|| panic!("no row '{label}' in {:?}", inspection.rows))
    };
    assert_eq!(row("Placed by"), "top_dowels (dowel)");
    assert_eq!(row("Pair"), "side_r into top (dowel)");
    assert_eq!(row("Fastener"), "dowel_pin, beech");
    assert_eq!(row("Copies"), "4");
    assert_eq!(row("Each"), "cylinder 8 mm across, 30 mm long");
    assert_eq!(row("Goes in with"), "side_r");
    assert!(
        inspection
            .description
            .as_deref()
            .is_some_and(|text| text.starts_with("The pins:")),
        "{:?}",
        inspection.description
    );
    // Defined in the text: the joint's line in carcass.stave.
    let source = inspection.source.expect("where the joint is written");
    assert!(source.location.path.ends_with("carcass.stave"));
    assert_eq!(source.location.line.get(), 102);
    // It goes together with its pair.
    assert_eq!(
        inspection.connections[0].target,
        Some(Target::Joint {
            joint: pins.joint,
            pair: pins.pair
        })
    );
    assert_eq!(
        focus::targets_of(design, Item::Member(pins))
            .into_iter()
            .map(|(target, _)| target)
            .collect::<Vec<_>>(),
        [Target::Joint {
            joint: pins.joint,
            pair: pins.pair
        }]
    );
}

/// The rows of the parts tree outside every assembly, as label and detail.
fn carcass_rows(tree: &inspect::PartsTree) -> Vec<(String, Option<String>)> {
    tree.carcass
        .iter()
        .map(|entry: &TreeEntry| (entry.label.clone(), entry.detail.clone()))
        .collect()
}

#[test]
fn the_parts_tree_counts_the_pins_each_joint_places() {
    let checked = checked("examples/clothing_cabinet");
    let (design, _) = drawable(&checked);
    let tree = inspect::parts_tree(design);
    let rows = carcass_rows(&tree);
    for joint in ["base_dowels", "top_dowels"] {
        assert!(
            rows.contains(&(
                format!("{joint}'s pin[2]"),
                Some("dowel_pin × 8".to_owned())
            )),
            "{rows:?}"
        );
    }
    // Shown one by one (when one is selected), each pair's row counts its
    // own copies: four pins each, eight in all.
    let pins = tree
        .carcass
        .iter()
        .find(|entry| entry.label == "top_dowels's pin[2]")
        .unwrap();
    assert_eq!(
        pins.member_labels,
        ["top_dowels's pin[0]", "top_dowels's pin[1]"]
    );
    assert_eq!(
        pins.member_details,
        [
            Some("dowel_pin × 4".to_owned()),
            Some("dowel_pin × 4".to_owned())
        ]
    );
    // A part's rows keep the stock, grouped or one by one.
    let sides = tree
        .carcass
        .iter()
        .find(|entry| entry.label == "side_l, side_r")
        .unwrap();
    assert_eq!(sides.detail.as_deref(), Some("oak_19"));
    assert_eq!(
        sides.member_details,
        [Some("oak_19".to_owned()), Some("oak_19".to_owned())]
    );
    // A drawer's screws are listed in the drawer.
    let drawer = tree
        .assemblies
        .iter()
        .flat_map(|a| a.children.iter().chain([a]))
        .find(|a| a.label == "drawer[top]")
        .expect("the top drawer");
    assert!(
        drawer
            .entries
            .iter()
            .any(|entry| entry.label == "front_screws's screw"
                && entry.detail.as_deref() == Some("wood_screw × 2")),
        "{:?}",
        drawer.entries
    );
}

#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "a test picks out the variant it checks, and any other is a failure or not what it counts"
)]
fn started(events: &[Event]) -> Option<stave_app::session::CheckTicket> {
    events.iter().find_map(|event| match event {
        Event::CheckStarted(ticket) => Some(*ticket),
        _ => None,
    })
}

fn session(folder: &str) -> Session {
    let folder = workspace().join(folder);
    let mut session = Session::new(folder.clone());
    let events = session.apply(Command::Check);
    let ticket = started(&events).expect("a check starts");
    session.finish_check(ticket, check::run(&folder));
    session
}

#[test]
fn a_selected_pin_shows_through_the_parts_it_sits_in() {
    let mut session = session("examples/clothing_cabinet");
    assert_eq!(
        session.looks(),
        None,
        "nothing selected, nothing see-through"
    );
    let (pins, side, top, door) = {
        let (design, graph) = session.drawable().unwrap();
        (
            member(design, "top_dowels", "side_l"),
            node_of_part(graph, design, "side_l"),
            node_of_part(graph, design, "top"),
            node_of_part(graph, design, "door_l.stile[hinge]"),
        )
    };
    session.apply(Command::Select(Some(Selection::Member(pins))));
    let looks = session.looks().expect("the pair's parts see-through");
    let pins_node = node_of(session.drawable().unwrap().1, Item::Member(pins));
    assert_eq!(looks[side.index()], Look::XRay);
    assert_eq!(looks[top.index()], Look::XRay);
    assert_eq!(looks[pins_node.index()], Look::Normal);
    assert_eq!(looks[door.index()], Look::Normal);
    // A part selected is drawn as usual.
    let side_id = session
        .drawable()
        .unwrap()
        .0
        .part_named("side_l")
        .unwrap()
        .id;
    session.apply(Command::Select(Some(Selection::Part(side_id))));
    assert_eq!(session.looks(), None);
}

#[test]
fn a_selection_of_pins_survives_a_recheck() {
    let mut session = session("examples/bookshelf");
    let pins = member(session.drawable().unwrap().0, "kick_dowels", "kick");
    session.apply(Command::Select(Some(Selection::Member(pins))));
    let folder = workspace().join("examples/bookshelf");
    let events = session.apply(Command::Check);
    let ticket = started(&events).expect("a check starts");
    session.finish_check(ticket, check::run(&folder));
    assert_eq!(session.selection(), Some(&Selection::Member(pins)));
}

#[test]
fn a_pin_past_its_hole_is_posed_and_highlighted() {
    let mut session = session("tests/broken/pin_past_its_hole");
    let (index, contact) = session
        .problems()
        .iter()
        .enumerate()
        .find_map(|(index, problem)| Some((index, problem.contact.clone()?)))
        .expect("the overlap has a contact");
    let member = contact
        .items
        .iter()
        .find_map(|item| match view::contact_item(*item) {
            Item::Member(member) => Some(member),
            Item::Part(_) | Item::Hardware(_) => None,
        })
        .expect("the problem is about the pegs");
    let (design, graph) = session.drawable().unwrap();
    let pegs = node_of(graph, Item::Member(member));
    // The pegs are tinted with the parts, and the shared wood is drawn.
    let highlight = view::highlight(graph, &contact);
    assert!(highlight.nodes.contains(&pegs));
    assert!(!highlight.volumes.is_empty());
    assert!(view::pose_at_contact(graph, &contact, None).is_some());
    let side = node_of_part(graph, design, "side");
    // Selected, the problem shows the pegs through the wood.
    session.apply(Command::Select(Some(Selection::Problem(ProblemIndex(
        index,
    )))));
    let looks = session.looks().expect("see-through");
    assert_eq!(looks[side.index()], Look::XRay);
    assert_eq!(looks[pegs.index()], Look::Normal);
}
