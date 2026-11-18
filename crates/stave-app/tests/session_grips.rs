//! Grips and the solver in the session, and the editor's defects fixed in
//! the last wave (`docs/editing.md`), driven through the session on copies
//! of the examples (and small projects written here) in temporary
//! folders: listing grips, finding their stops on another thread and
//! cancelling that, dragging, typing a value, picking the param a size
//! changes, the solver's drag and its refusal, making a param solved and
//! plain; a knob placed on a face, the menu without joints that can never
//! apply and with its refused entries last, a part spaced along a span,
//! the distance Hold offers, and a component not offered inside itself.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use stave_app::change::ChangeDetail;
use stave_app::edit::{Applicability, FacingPair, PartFace, Picked, Verdict};
use stave_app::editing::{EditCommand, EditEvent, Progress, Rejection, parse_name, parse_value};
use stave_app::gripping::{GripDetail, GripStops};
use stave_app::grips::{GripAction, GripSelection, PlaceInput};
use stave_app::hold::Measured;
use stave_app::naming::RepeatCount;
use stave_app::session::{Command, Event, Session};
use stave_app::solve_edit::Blocker;
use stave_lang::placement::{Axis, Face};
use stave_lang::units::Length;
use stave_model::trace::Amount;

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A fresh temporary folder named for the test.
fn scratch_folder(name: &str) -> PathBuf {
    let root =
        std::env::temp_dir().join(format!("stave-session-grips-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

/// A copy of an example in a fresh temporary folder: the project folder.
fn copy_of(example: &str, name: &str) -> PathBuf {
    let project = scratch_folder(name).join("project");
    std::fs::create_dir_all(&project).unwrap();
    for entry in std::fs::read_dir(workspace().join("examples").join(example)).unwrap() {
        let path = entry.unwrap().path();
        if path.is_file() {
            std::fs::copy(&path, project.join(path.file_name().unwrap())).unwrap();
        }
    }
    project
}

/// A project of one file written here, in a fresh temporary folder.
fn project_of(name: &str, text: &str) -> PathBuf {
    let project = scratch_folder(name).join("project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("project.stave"), text).unwrap();
    project
}

fn send(session: &mut Session, command: EditCommand) -> Vec<EditEvent> {
    session
        .apply(Command::Edit(command))
        .into_iter()
        .filter_map(|event| match event {
            Event::Edit(event) => Some(event),
            _ => None,
        })
        .collect()
}

fn mm(n: i64) -> Amount {
    Amount::Length(Length::from_mm(n).to_exact())
}

fn assembly(folder: &Path, name: &str) -> stave_model::ids::AssemblyId {
    let opened = stave_app::open(folder).unwrap();
    let model = opened.model.as_ref().unwrap();
    model.assemblies.iter().find(|a| a.name == name).unwrap().id
}

fn part(folder: &Path, name: &str) -> stave_model::ids::PartId {
    let opened = stave_app::open(folder).unwrap();
    let model = opened.model.as_ref().unwrap();
    model.parts.iter().find(|p| p.name == name).unwrap().id
}

fn region(folder: &Path, name: &str) -> stave_model::ids::RegionId {
    let opened = stave_app::open(folder).unwrap();
    let model = opened.model.as_ref().unwrap();
    model.regions.iter().find(|r| r.name == name).unwrap().id
}

fn grip_index(session: &Session, label: &str) -> usize {
    session
        .grips()
        .unwrap()
        .grips()
        .iter()
        .position(|listed| listed.grip.label == label)
        .unwrap_or_else(|| panic!("no grip {label}"))
}

fn detail(session: &Session) -> GripDetail {
    match session.change().unwrap().detail() {
        ChangeDetail::Grip(detail) => (**detail).clone(),
        other => panic!("not a drag: {other:?}"),
    }
}

fn fits(session: &Session) -> bool {
    matches!(
        session.change().unwrap().progress(),
        Progress::Tried(Verdict::Fits(_))
    )
}

fn errors(folder: &Path) -> usize {
    stave_app::open(folder).unwrap().error_count()
}

// ---------------------------------------------------------------------------
// A: grips in the session
// ---------------------------------------------------------------------------

/// The divider's grips are listed at once, without their stops; the
/// stops are found by a job on another thread and handed back grip by
/// grip; a cancelled job hands back nothing.
#[test]
fn grips_are_listed_and_their_stops_found_off_the_main_thread() {
    let folder = copy_of("clothing_cabinet_from_box", "stops");
    let mut session = Session::new(folder.clone());
    let divider = assembly(&folder, "divider");
    let events = send(
        &mut session,
        EditCommand::ListGrips(GripSelection::Assembly(divider)),
    );
    assert_eq!(events, [EditEvent::GripsChanged]);
    let height = grip_index(&session, "divider.height");
    let listed = &session.grips().unwrap().grips()[height];
    assert_eq!(listed.stops, GripStops::NotSearched);
    assert_eq!(listed.grip.axis, Some(Axis::Up));
    // What the scene's grip_place needs: the faces and the lengths.
    let Some(PlaceInput::Length {
        param,
        lengths,
        between: Some((from, to)),
        ..
    }) = &listed.grip.place
    else {
        panic!("a length grip carries its faces: {:?}", listed.grip.place);
    };
    assert_eq!(param, "height");
    assert!(lengths.contains(&("height".to_owned(), Length::from_mm(500))));
    assert_eq!((from.face, to.face), (Face::Bottom, Face::Bottom));
    // The contract's grip too.
    assert!(session.grips().unwrap().contract(height).is_some());

    // A cancelled job finds nothing.
    let job = session.grip_stops_job().unwrap().only(height);
    assert_eq!(job.searches().len(), 1);
    let cancelled = AtomicBool::new(true);
    let mut found = Vec::new();
    assert!(job.run(&cancelled, &mut |stops| found.push(stops)).is_err());
    assert!(found.is_empty());

    // On another thread.
    let found = std::thread::spawn(move || {
        let mut found = Vec::new();
        job.run(&AtomicBool::new(false), &mut |stops| found.push(stops))
            .unwrap();
        found
    })
    .join()
    .unwrap();
    assert_eq!(found.len(), 1);
    let events = session.grip_stops_found(found[0].clone());
    assert_eq!(events, [Event::Edit(EditEvent::GripStopsFound(height))]);
    let listed = &session.grips().unwrap().grips()[height];
    let GripStops::Found { below, .. } = &listed.stops else {
        panic!("{:?}", listed.stops);
    };
    assert_eq!(below.last_good, mm(180));
    let GripAction::Moves(movable) = &listed.grip.action else {
        panic!("the height moves");
    };
    assert_eq!(movable.low, mm(180));

    // An answer for an older listing is dropped.
    send(
        &mut session,
        EditCommand::ListGrips(GripSelection::Assembly(divider)),
    );
    assert!(session.grip_stops_found(found[0].clone()).is_empty());
}

/// Dragging the divider tries each value, shows the ghost, and Add writes
/// the last; the grips, listed from the old design, are dropped.
#[test]
fn a_drag_is_tried_at_each_value_and_committed_by_the_one_writer() {
    let folder = copy_of("clothing_cabinet_from_box", "drag");
    let before = std::fs::read_to_string(folder.join("cabinet.stave")).unwrap();
    let mut session = Session::new(folder.clone());
    // No grips listed: a drag is rejected.
    assert_eq!(
        send(&mut session, EditCommand::BeginDrag(0)),
        [EditEvent::Rejected(Rejection::NoGrips)]
    );
    send(
        &mut session,
        EditCommand::ListGrips(GripSelection::Assembly(assembly(&folder, "divider"))),
    );
    let height = grip_index(&session, "divider.height");
    assert_eq!(
        send(&mut session, EditCommand::BeginDrag(99)),
        [EditEvent::Rejected(Rejection::NoSuchGrip(99))]
    );
    let events = send(&mut session, EditCommand::BeginDrag(height));
    assert!(events.contains(&EditEvent::ChangeStarted), "{events:?}");
    assert!(!fits(&session));
    let events = send(&mut session, EditCommand::DragTo(mm(400)));
    assert!(events.contains(&EditEvent::DragMoved), "{events:?}");
    assert!(fits(&session));
    // The value is snapped to the resolution.
    send(
        &mut session,
        EditCommand::DragTo(Amount::Length(Length::from_nm(420_200_000).to_exact())),
    );
    assert!(fits(&session));
    let change = session.change().unwrap();
    assert!(
        change
            .patch()
            .unwrap()
            .contains("set divider.height = 420mm")
    );
    let panel = change
        .ghost()
        .and_then(|o| o.design.as_ref())
        .and_then(|d| d.part_named("divider.panel"))
        .unwrap();
    assert_eq!(panel.drawn_box().face(Face::Bottom), Length::from_mm(519));
    assert_eq!(detail(&session).changes[0].to, mm(420));
    // Cancel leaves the file as it was; the grips stay listed.
    send(&mut session, EditCommand::Cancel);
    assert_eq!(
        std::fs::read_to_string(folder.join("cabinet.stave")).unwrap(),
        before
    );
    assert!(session.grips().is_some());
    assert_eq!(
        send(&mut session, EditCommand::DragTo(mm(400))),
        [EditEvent::Rejected(Rejection::NotDragging)]
    );
    // Typed: begun and moved in one.
    send(
        &mut session,
        EditCommand::TypeGripValue {
            grip: height,
            value: mm(450),
        },
    );
    assert!(fits(&session));
    let events = send(&mut session, EditCommand::Add);
    assert!(events.contains(&EditEvent::Committed), "{events:?}");
    assert!(events.contains(&EditEvent::GripsChanged), "{events:?}");
    assert!(session.grips().is_none());
    let text = std::fs::read_to_string(folder.join("cabinet.stave")).unwrap();
    assert!(text.contains("height:      450mm"), "{text}");
    assert_eq!(errors(&folder), 0);
}

/// A size that follows two plain params waits for the user to pick one;
/// the pick inverts exactly. A size that follows nothing that can change
/// says what stops it.
#[test]
fn a_follows_grip_with_several_params_waits_for_a_pick() {
    let folder = copy_of("clothing_cabinet_from_box", "pick");
    let mut session = Session::new(folder.clone());
    send(
        &mut session,
        EditCommand::ListGrips(GripSelection::Assembly(assembly(&folder, "drawer[bottom]"))),
    );
    let width = grip_index(&session, "drawer[bottom].sub_front.length");
    let GripAction::Follows(follows) = &session.grips().unwrap().grips()[width].grip.action else {
        panic!("the box width follows");
    };
    let target = follows
        .value
        .with_raw(follows.value.raw().checked_add(mm(50).raw()).unwrap());
    send(&mut session, EditCommand::BeginDrag(width));
    send(&mut session, EditCommand::DragTo(target));
    let Progress::NeedsChoice(names) = session.change().unwrap().progress() else {
        panic!("{:?}", session.change().unwrap().progress());
    };
    let names: Vec<&str> = names.iter().map(|n| n.as_str()).collect();
    assert_eq!(names, ["overall_width", "overhang"]);
    let found = detail(&session);
    let choices: Vec<&str> = found.choices.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(choices, ["overall_width", "case.overhang"]);
    let overall_width = found.choices[0].param;
    assert_eq!(found.choices[0].change.as_ref().unwrap().to, mm(990));
    // A param that is not a choice is refused.
    let other = found.choices[1].param;
    let events = send(&mut session, EditCommand::PickParam(overall_width));
    assert!(events.contains(&EditEvent::ParamPicked), "{events:?}");
    assert!(fits(&session));
    let found = detail(&session);
    assert_eq!(found.picked, Some(overall_width));
    assert_eq!(found.changes[0].path, "overall_width");
    assert_ne!(other, overall_width);

    // The bottom's width follows the box stock's thickness only.
    send(&mut session, EditCommand::Cancel);
    let bottom = grip_index(&session, "drawer[bottom].bottom.width");
    send(&mut session, EditCommand::BeginDrag(bottom));
    send(&mut session, EditCommand::DragTo(mm(400)));
    let found = detail(&session);
    let stopped = found.stopped.unwrap();
    assert!(
        stopped.contains("maple_12.thickness (a size of the stock)"),
        "{stopped}"
    );
    assert!(matches!(
        session.change().unwrap().progress(),
        Progress::Unwritable(_)
    ));
    assert_eq!(
        send(&mut session, EditCommand::Add),
        [EditEvent::Rejected(Rejection::DoesNotFit)]
    );
}

/// The boundary between the two drawers is a grip of the split; moving it
/// writes small whole shares.
#[test]
fn the_boundary_of_a_split_is_dragged_to_small_shares() {
    let folder = copy_of("clothing_cabinet_from_box", "boundary");
    let mut session = Session::new(folder.clone());
    send(
        &mut session,
        EditCommand::ListGrips(GripSelection::Split(region(&folder, "drawer_bay[bottom]"))),
    );
    send(
        &mut session,
        EditCommand::TypeGripValue {
            grip: 0,
            value: Amount::Length(Length::from_nm(200_200_000).to_exact()),
        },
    );
    assert!(fits(&session));
    let patch = session.change().unwrap().patch().unwrap().to_owned();
    assert!(patch.contains("shares: [2, 3]"), "{patch}");
}

// ---------------------------------------------------------------------------
// B: the solver in the session
// ---------------------------------------------------------------------------

/// The side table's height made solved: dragging a leg's length lets the
/// solver choose `overall_height`, reports the change before the drag
/// ends, and refuses a length the knee-room rule blocks, naming the rule.
/// Made plain again, the range stays.
#[test]
fn a_size_of_solved_params_is_dragged_by_the_solver() {
    let folder = copy_of("side_table_from_box", "solver");
    let mut session = Session::new(folder.clone());
    let events = send(
        &mut session,
        EditCommand::MakeSolved {
            param: parse_name("overall_height").unwrap(),
            low: parse_value("300mm").unwrap(),
            high: parse_value("700mm").unwrap(),
        },
    );
    assert!(events.contains(&EditEvent::ChangeStarted), "{events:?}");
    assert!(fits(&session));
    assert!(send(&mut session, EditCommand::Add).contains(&EditEvent::Committed));
    let text = std::fs::read_to_string(folder.join("table.stave")).unwrap();
    assert!(text.contains("solved param overall_height"), "{text}");
    assert!(text.contains("= 600mm within 300mm..700mm"), "{text}");

    send(
        &mut session,
        EditCommand::ListGrips(GripSelection::Assembly(assembly(&folder, "legs"))),
    );
    let leg = grip_index(&session, "legs.leg[front_left].length");
    send(&mut session, EditCommand::BeginDrag(leg));
    send(&mut session, EditCommand::DragTo(mm(500)));
    assert!(fits(&session), "{:?}", session.change().unwrap().progress());
    let found = detail(&session);
    assert!(found.solved);
    assert_eq!(found.changes.len(), 1);
    assert_eq!(found.changes[0].path, "overall_height");
    assert_eq!(
        (found.changes[0].from, found.changes[0].to),
        (mm(600), mm(519))
    );
    let leg_part = session
        .change()
        .unwrap()
        .ghost()
        .and_then(|o| o.design.as_ref())
        .and_then(|d| d.part_named("legs.leg[front_left]"))
        .unwrap();
    assert_eq!(
        leg_part.drawn.get(stave_lang::placement::PartAxis::Length),
        Length::from_mm(500)
    );

    // Too short: the knee room blocks it.
    send(&mut session, EditCommand::DragTo(mm(400)));
    let found = detail(&session);
    assert!(
        found
            .blockers
            .iter()
            .any(|b| matches!(b, Blocker::Require { name, .. } if name == "knee_room")),
        "{:?}",
        found.blockers
    );
    let Progress::Unwritable(reason) = session.change().unwrap().progress() else {
        panic!("refused");
    };
    assert!(reason.contains("the rule 'knee_room' stops it"), "{reason}");
    send(&mut session, EditCommand::Cancel);

    // Plain again.
    send(
        &mut session,
        EditCommand::MakePlain(parse_name("overall_height").unwrap()),
    );
    assert!(fits(&session));
    send(&mut session, EditCommand::Add);
    let text = std::fs::read_to_string(folder.join("table.stave")).unwrap();
    assert!(!text.contains("solved param"), "{text}");
    assert!(text.contains("= 600mm within 300mm..700mm"), "{text}");
    assert_eq!(errors(&folder), 0);
}

// ---------------------------------------------------------------------------
// D: the defects
// ---------------------------------------------------------------------------

const STILE: &str = r#"stave 1

project "Knob test" {
  units: mm
}

param overall_width  = 400mm
param overall_height = 600mm
param overall_depth  = 300mm

region envelope = box(across: overall_width, up: overall_height, deep: overall_depth)

stock oak_19 { species: white_oak, form: solid, thickness: 19mm, max_width: 200mm, max_length: 2700mm }

part stile : oak_19 {
  length: 600mm
  width:  60mm
  place {
    orient(length: up, width: right)
    flush(envelope.bottom)
    flush(envelope.left)
    flush(envelope.front)
  }
}
"#;

/// D1: a knob chosen on a stile's face is placed: centred on the face's
/// two axes, on that face; it fits and is written.
#[test]
fn a_knob_chosen_on_a_stile_is_placed_and_fits() {
    let folder = project_of("knob", STILE);
    let mut session = Session::new(folder.clone());
    let stile = part(&folder, "stile");
    send(
        &mut session,
        EditCommand::Start(Picked::Face(PartFace {
            part: stile,
            face: Face::Front,
        })),
    );
    send(&mut session, EditCommand::CheckMenu);
    let edit = session.edit().unwrap();
    let knob = edit
        .menu()
        .iter()
        .position(|c| c.entry.label.as_str() == "knob")
        .unwrap();
    assert_eq!(edit.menu()[knob].applicability, Applicability::Applies);
    send(&mut session, EditCommand::Choose(knob));
    let chosen = session.edit().unwrap().chosen().unwrap();
    assert!(
        matches!(chosen.progress(), Progress::Tried(Verdict::Fits(_))),
        "{:?}",
        chosen.progress()
    );
    let patch = chosen.patch().unwrap();
    assert!(
        patch.contains(
            "add hardware knob_2 : knob(stile) {\n    place {\n      center(stile, across)\n      center(stile, up)\n    }\n  } in \"project.stave\""
        ),
        "{patch}"
    );
    assert!(send(&mut session, EditCommand::Add).contains(&EditEvent::Committed));
    assert_eq!(errors(&folder), 0);

    // On the stile's side, the call says which face.
    send(
        &mut session,
        EditCommand::Start(Picked::Face(PartFace {
            part: stile,
            face: Face::Right,
        })),
    );
    let knob = session
        .edit()
        .unwrap()
        .menu()
        .iter()
        .position(|c| c.entry.label.as_str() == "knob")
        .unwrap();
    send(&mut session, EditCommand::Choose(knob));
    let patch = session.edit().unwrap().chosen().unwrap().patch().unwrap();
    assert!(
        patch.contains("knob(stile, face: right) {\n    place {\n      center(stile, up)\n      center(stile, deep)"),
        "{patch}"
    );
}

/// D4: between the case's two sides, no joint is listed (the sides never
/// meet); the refused entries come after every entry that fits, each
/// error said once.
#[test]
fn the_menu_leaves_out_joints_that_can_never_apply_and_puts_refusals_last() {
    let folder = copy_of("clothing_cabinet_from_box", "menu");
    let mut session = Session::new(folder.clone());
    let pair = FacingPair::new(
        part(&folder, "case.side[left]"),
        part(&folder, "case.side[right]"),
    )
    .unwrap();
    send(&mut session, EditCommand::Start(Picked::Between(pair)));
    send(&mut session, EditCommand::CheckMenu);
    let edit = session.edit().unwrap();
    let menu = edit.menu();
    assert!(
        menu.iter().all(|c| !matches!(
            &c.entry.group,
            stave_app::edit::MenuGroup::Standard(name) if name.as_str() == "joinery"
        )),
        "{:?}",
        menu.iter()
            .map(|c| c.entry.label.as_str())
            .collect::<Vec<_>>()
    );
    let refused = edit.refused_from();
    assert!(refused > 0 && refused < menu.len());
    for (index, candidate) in menu.iter().enumerate() {
        let greyed = matches!(candidate.applicability, Applicability::Greyed(_));
        assert_eq!(
            greyed,
            index >= refused,
            "{}",
            candidate.entry.label.as_str()
        );
        if let Applicability::Greyed(problems) = &candidate.applicability {
            let said: Vec<(String, String)> = problems
                .iter()
                .map(|p| (p.code.to_string(), p.message.clone()))
                .collect();
            let mut once = said.clone();
            once.dedup();
            once.sort();
            once.dedup();
            assert_eq!(once.len(), said.len(), "{said:?}");
        }
    }
}

const SLATS: &str = r#"stave 1

project "Slat test" {
  units: mm
}

param overall_width  = 400mm
param overall_height = 300mm
param overall_depth  = 300mm

region envelope = box(across: overall_width, up: overall_height, deep: overall_depth)

stock oak_19 { species: white_oak, form: solid, thickness: 19mm, max_width: 200mm, max_length: 2700mm }

part rail_l : oak_19 {
  length: 300mm
  width:  40mm
  place {
    orient(length: back, width: up)
    flush(envelope.bottom)
    flush(envelope.left)
    flush(envelope.front)
  }
}

part rail_r : oak_19 {
  length: 300mm
  width:  40mm
  place {
    orient(length: back, width: up)
    flush(envelope.bottom)
    flush(envelope.right)
    flush(envelope.front)
  }
}

part slat : oak_19 {
  length: 300mm
  width:  40mm
  place {
    orient(length: back, width: right)
    against(rail_l.top)
    flush(rail_l.right)
    flush(envelope.front)
  }
}
"#;

/// D6: a part is repeated, spaced along a span, from the session.
#[test]
fn a_part_is_spaced_along_a_span_from_the_session() {
    let folder = project_of("spaced", SLATS);
    let mut session = Session::new(folder.clone());
    let events = send(
        &mut session,
        EditCommand::RepeatPart {
            part: parse_name("slat").unwrap(),
            count: RepeatCount::new(3).unwrap(),
            axis: Axis::Across,
            from: "rail_l.right".to_owned(),
            to: "rail_r.left".to_owned(),
        },
    );
    assert!(events.contains(&EditEvent::ChangeStarted), "{events:?}");
    assert!(fits(&session), "{:?}", session.change().unwrap().progress());
    assert!(send(&mut session, EditCommand::Add).contains(&EditEvent::Committed));
    let opened = stave_app::open(&folder).unwrap();
    let design = opened.design.as_ref().unwrap();
    for label in ["left", "middle", "right"] {
        assert!(
            design.part_named(&format!("slat[{label}]")).is_some(),
            "slat[{label}]"
        );
    }
    assert_eq!(opened.error_count(), 0);
}

/// D7: Hold offers the distance as it is now.
#[test]
fn hold_offers_the_measured_distance() {
    let folder = copy_of("clothing_cabinet_from_box", "measure");
    let session = Session::new(folder.clone());
    // The divider stands 500mm above the case's base.
    let distance = session
        .measure(
            &Measured::Face {
                item: "case.base".to_owned(),
                face: Face::Top,
            },
            &Measured::Face {
                item: "divider.panel".to_owned(),
                face: Face::Bottom,
            },
        )
        .unwrap();
    assert_eq!(distance, Length::from_mm(500));
    assert!(
        session
            .measure(
                &Measured::Item("divider.panel".to_owned()),
                &Measured::Item("divider.panel".to_owned()),
            )
            .is_err()
    );
}

const NESTING: &str = r#"stave 1

project "Nesting test" {
  units: mm
}

param overall_width  = 400mm
param overall_height = 300mm
param overall_depth  = 300mm

region envelope = box(across: overall_width, up: overall_height, deep: overall_depth)

stock oak_19 { species: white_oak, form: solid, thickness: 19mm, max_width: 400mm, max_length: 2700mm }

/// A tray: a board on the bottom of a space. It may nest inside itself.
component Tray {
  param space       : Region
  param board_stock : Stock

  part board : board_stock {
    length: space.across - 20mm
    width:  space.deep - 20mm
    place {
      flush(space.bottom)
      center(space, across)
      center(space, deep)
    }
  }

  /// The space above the board.
  region above = opening(
    left:   space.left,
    right:  space.right,
    bottom: board.top,
    top:    space.top,
    front:  space.front,
    back:   space.back,
  )
}

/// A box: a board on the bottom of a space, with room above it.
component Bin {
  param space       : Region
  param board_stock : Stock

  part board : board_stock {
    length: space.across - 20mm
    width:  space.deep - 20mm
    place {
      flush(space.bottom)
      center(space, across)
      center(space, deep)
    }
  }

  /// The space above the board.
  region above = opening(
    left:   space.left,
    right:  space.right,
    bottom: board.top,
    top:    space.top,
    front:  space.front,
    back:   space.back,
  )
}

assembly tray : Tray {
  space:       envelope
  board_stock: oak_19
}

assembly bin : Bin {
  space:       envelope
  board_stock: oak_19
}
"#;

/// D8: a component is not offered in the space its own assembly exposes,
/// unless its doc comment says it may nest inside itself; and in the
/// cabinet, `Case` is not offered inside the case.
#[test]
fn a_component_is_not_offered_inside_itself_unless_it_may_nest() {
    let menu_labels = |folder: &Path, region_name: &str| {
        let mut session = Session::new(folder.to_path_buf());
        send(
            &mut session,
            EditCommand::Start(Picked::Region(region(folder, region_name))),
        );
        session
            .edit()
            .unwrap()
            .menu()
            .iter()
            .map(|c| c.entry.label.as_str().to_owned())
            .collect::<Vec<_>>()
    };
    let folder = project_of("nesting", NESTING);
    let in_bin = menu_labels(&folder, "bin.above");
    assert!(!in_bin.contains(&"Bin".to_owned()), "{in_bin:?}");
    assert!(in_bin.contains(&"Tray".to_owned()), "{in_bin:?}");
    let in_tray = menu_labels(&folder, "tray.above");
    assert!(in_tray.contains(&"Tray".to_owned()), "{in_tray:?}");
    assert!(in_tray.contains(&"Bin".to_owned()), "{in_tray:?}");

    let cabinet = copy_of("clothing_cabinet_from_box", "nesting-case");
    let inside = menu_labels(&cabinet, "case.inside");
    assert!(!inside.contains(&"Case".to_owned()), "{inside:?}");
    assert!(inside.contains(&"Divider".to_owned()), "{inside:?}");
    let envelope = menu_labels(&cabinet, "envelope");
    assert!(envelope.contains(&"Case".to_owned()), "{envelope:?}");
}
