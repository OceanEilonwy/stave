//! The window's session: views, the selection and its history, the scene
//! and the background check, driven the way the window drives them.

use std::path::{Path, PathBuf};
use std::time::Instant;

use stave_app::check::{self, Outcome};
use stave_app::empty::Reason;
use stave_app::focus::Target;
use stave_app::inspect::{self, SwatchShape};
use stave_app::session::{
    Activity, Build, CheckStatus, Command, Editor, Event, Explode, Notice, ProblemIndex, Selection,
    Session, SourceLocation, Tone, Verdict, View,
};
use stave_modules::MemoryFiles;

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn example(name: &str) -> PathBuf {
    workspace().join("examples").join(name)
}

/// A session for `folder` after its first check.
fn checked_session(folder: PathBuf) -> Session {
    let mut session = Session::new(folder.clone());
    let events = session.apply(Command::Check);
    let ticket = started(&events).expect("a check starts");
    session.finish_check(ticket, check::run(&folder));
    session
}

fn started(events: &[Event]) -> Option<stave_app::session::CheckTicket> {
    events.iter().find_map(|event| match event {
        Event::CheckStarted(ticket) => Some(*ticket),
        _ => None,
    })
}

fn part(session: &Session, name: &str) -> Selection {
    let (design, _) = session.drawable().expect("the design solves");
    let found = design
        .parts
        .iter()
        .find(|p| p.name == name)
        .unwrap_or_else(|| panic!("no part '{name}'"));
    Selection::Part(found.id)
}

#[test]
fn a_new_session_has_nothing_checked() {
    let session = Session::new(example("stool"));
    assert_eq!(session.view(), View::Design);
    assert_eq!(session.editor(), Editor::Closed);
    assert_eq!(session.selection(), None);
    assert!(!session.can_go_back());
    assert!(!session.can_go_forward());
    assert_eq!(session.check_status(), &CheckStatus::Idle);
    assert_eq!(session.check_status().pill().tone, Tone::Unknown);
    assert_eq!(session.check_status().line(), "Not checked yet");
}

#[test]
fn back_and_forward_walk_the_selection_history() {
    let mut session = checked_session(example("stool"));
    let seat = part(&session, "seat");
    let leg = part(&session, "leg[front_left]");

    let events = session.apply(Command::Select(Some(seat.clone())));
    assert!(events.contains(&Event::SelectionChanged));
    assert!(events.contains(&Event::HistoryChanged));
    session.apply(Command::Select(Some(leg.clone())));
    assert_eq!(session.selection(), Some(&leg));
    assert!(session.can_go_back());
    assert!(!session.can_go_forward(), "forward waits until you go back");

    session.apply(Command::Back);
    assert_eq!(session.selection(), Some(&seat));
    assert!(session.can_go_forward());
    session.apply(Command::Back);
    assert_eq!(session.selection(), None);
    assert!(!session.can_go_back());

    session.apply(Command::Forward);
    session.apply(Command::Forward);
    assert_eq!(session.selection(), Some(&leg));
    assert!(!session.can_go_forward());

    // A new selection after going back drops what was ahead.
    session.apply(Command::Back);
    session.apply(Command::Select(Some(leg.clone())));
    assert!(!session.can_go_forward());
}

#[test]
fn selecting_the_same_thing_again_changes_nothing() {
    let mut session = checked_session(example("stool"));
    let seat = part(&session, "seat");
    session.apply(Command::Select(Some(seat.clone())));
    let events = session.apply(Command::Select(Some(seat)));
    assert!(events.is_empty(), "{events:?}");
}

#[test]
fn views_switch_and_are_remembered() {
    let mut session = checked_session(example("stool"));
    let events = session.apply(Command::ShowView(View::CutList));
    assert_eq!(session.view(), View::CutList);
    assert!(events.contains(&Event::ViewChanged));
    session.apply(Command::ShowView(View::Design));
    session.apply(Command::Back);
    assert_eq!(session.view(), View::CutList);
}

#[test]
fn a_source_link_opens_the_editor_there() {
    let mut session = checked_session(example("clothing_cabinet"));
    let stile = part(&session, "door_l.stile[hinge]");
    session.apply(Command::Select(Some(stile.clone())));
    let shown = session.shown().expect("a result").clone();
    let inspection = inspect::inspect(&shown, &stile).expect("the stile is described");
    let source = inspection.source.expect("it is written somewhere");
    assert_eq!(source.label, "doors.stave:9");

    let events = session.apply(Command::GoToSource(source.location.clone()));
    assert!(events.contains(&Event::EditorChanged));
    assert_eq!(session.view(), View::Design);
    assert_eq!(session.editor(), Editor::Open);
    assert_eq!(
        session.selection(),
        Some(&Selection::Location(source.location))
    );
    session.apply(Command::Back);
    assert_eq!(session.editor(), Editor::Closed);
    assert_eq!(session.selection(), Some(&stile));
}

#[test]
fn focus_mode_is_a_place_in_the_history() {
    let mut session = checked_session(example("clothing_cabinet"));
    let (design, _) = session.drawable().unwrap();
    let hinge = design
        .hardware
        .iter()
        .find(|h| h.name == "door_l.hinges[upper]")
        .unwrap()
        .id;
    let events = session.apply(Command::Focus(Target::Hardware(hinge)));
    assert!(events.contains(&Event::FocusChanged));
    assert_eq!(session.focus(), Some(Target::Hardware(hinge)));
    assert_eq!(session.selection(), Some(&Selection::Hardware(hinge)));
    session.apply(Command::LeaveFocus);
    assert_eq!(session.focus(), None);
    assert_eq!(session.selection(), Some(&Selection::Hardware(hinge)));
    session.apply(Command::Back);
    assert_eq!(session.focus(), Some(Target::Hardware(hinge)));
}

#[test]
fn a_check_runs_then_reports_its_counts() {
    let folder = example("stool");
    let mut session = Session::new(folder.clone());
    let events = session.apply(Command::Check);
    let ticket = started(&events).expect("a check starts");
    assert!(events.contains(&Event::CheckStatusChanged));
    assert_eq!(
        session.check_status(),
        &CheckStatus::Checking { last: None }
    );
    let pill = session.check_status().pill();
    assert_eq!(pill.activity, Activity::Checking);
    assert_eq!(pill.label, "Checking…");

    let events = session.finish_check(ticket, check::run(&folder));
    assert!(events.contains(&Event::ResultChanged));
    assert!(events.contains(&Event::CheckStatusChanged));
    let Some(Verdict::Checked(summary)) = session.check_status().verdict() else {
        panic!("{:?}", session.check_status());
    };
    assert_eq!(summary.errors, 0);
    assert_eq!(summary.files, 1);
    let pill = session.check_status().pill();
    assert_eq!(
        (pill.tone, pill.label.as_str(), pill.activity),
        (Tone::Clean, "No problems", Activity::Idle)
    );
    assert!(
        session
            .check_status()
            .line()
            .starts_with("Checked 1 file · 0 errors · 0 warnings · "),
        "{}",
        session.check_status().line()
    );
}

#[test]
fn while_checking_again_the_pill_keeps_its_last_colour_and_words() {
    let folder = workspace().join("tests/broken/door_hits_side");
    let mut session = checked_session(folder);
    let before = session.check_status().pill();
    assert_eq!(before.tone, Tone::Errors);
    assert_eq!(before.label, "1 problem");

    session.apply(Command::Check);
    let during = session.check_status().pill();
    assert_eq!(during.tone, before.tone);
    assert_eq!(during.label, before.label);
    assert_eq!(during.activity, Activity::Checking);
    assert_eq!(
        session.check_status().line(),
        "Checking again… last result: 1 error"
    );
}

#[test]
fn a_stale_check_is_dropped_and_a_check_asked_for_meanwhile_runs_next() {
    let folder = example("stool");
    let mut session = Session::new(folder.clone());
    let first = started(&session.apply(Command::Check)).unwrap();
    // Asked again while running: queued, not started.
    assert_eq!(started(&session.apply(Command::Check)), None);
    let events = session.finish_check(first, check::run(&folder));
    let second = started(&events).expect("the queued check starts");
    assert!(session.check_status().is_checking());
    // The first ticket's outcome, again, is stale now.
    assert!(session.finish_check(first, check::run(&folder)).is_empty());
    session.finish_check(second, check::run(&folder));
    assert!(!session.check_status().is_checking());
}

#[test]
fn a_folder_that_cant_be_read_says_so() {
    let folder = workspace().join("tests/no_such_folder");
    let mut session = Session::new(folder.clone());
    let ticket = started(&session.apply(Command::Check)).unwrap();
    let outcome = check::run(&folder);
    assert!(matches!(outcome, Outcome::CantOpen { .. }));
    let events = session.finish_check(ticket, outcome);
    assert_eq!(session.check_status().pill().label, "Can't open");
    assert!(session.drawable().is_none());
    // The view leaves "Checking…" for why: the window redraws on a
    // changed result, so one is said.
    assert!(events.contains(&Event::ResultChanged));
}

#[test]
fn the_build_button_checks_then_says_the_plan_isnt_built_yet() {
    let folder = example("stool");
    let mut session = checked_session(folder.clone());
    let events = session.apply(Command::Build);
    assert_eq!(session.build(), Build::Building);
    assert!(events.contains(&Event::BuildChanged));
    let ticket = started(&events).expect("building checks first");
    // A second press while building does nothing.
    assert!(session.apply(Command::Build).is_empty());
    let events = session.finish_check(ticket, check::run(&folder));
    assert_eq!(session.build(), Build::Idle);
    assert!(events.contains(&Event::Notice(Notice::BuildPlanNotBuilt)));
    assert_eq!(
        Notice::BuildPlanNotBuilt.text(),
        "The build plan (PDF) isn't built yet"
    );
}

#[test]
fn scenes_bring_their_own_explode_amount() {
    let mut session = checked_session(example("clothing_cabinet"));
    let (_, graph) = session.drawable().unwrap();
    let names: Vec<String> = graph.scenes.iter().map(|s| s.name.clone()).collect();
    assert!(names.len() > 1, "{names:?}");
    let events = session.apply(Command::ShowScene(names[1].clone()));
    assert!(events.contains(&Event::SceneChanged));
    assert_eq!(session.scene_index(), 1);
    session.apply(Command::SetExplode(Explode::new(0.5).unwrap()));
    assert_eq!(session.explode().get(), 0.5);
    // A scene that doesn't exist is ignored.
    assert!(
        session
            .apply(Command::ShowScene("no_such_scene".to_owned()))
            .is_empty()
    );
}

#[test]
fn the_editor_opens_and_closes_without_a_history_entry() {
    let mut session = checked_session(example("stool"));
    let events = session.apply(Command::ShowEditor(Editor::Open));
    assert_eq!(events, vec![Event::EditorChanged]);
    assert_eq!(session.editor(), Editor::Open);
    assert!(!session.can_go_back());
    assert!(session.apply(Command::ShowEditor(Editor::Open)).is_empty());
    // It stays open while the selection changes.
    let seat = part(&session, "seat");
    session.apply(Command::Select(Some(seat)));
    assert_eq!(session.editor(), Editor::Open);
    session.apply(Command::ShowEditor(Editor::Closed));
    assert_eq!(session.editor(), Editor::Closed);
}

#[test]
fn f8_steps_through_the_problems_and_wraps_round() {
    let mut session = checked_session(workspace().join("tests/broken/door_hits_side"));
    let count = session.problems().len();
    assert!(count >= 1);
    let last = ProblemIndex(count - 1);
    let events = session.apply(Command::NextProblem);
    assert!(events.contains(&Event::SelectionChanged));
    assert_eq!(
        session.selection(),
        Some(&Selection::Problem(ProblemIndex(0)))
    );
    assert_eq!(
        session.selected_problem().map(|p| p.code.as_str()),
        Some("motion")
    );
    // Before the first comes the last; after the last, the first again.
    session.apply(Command::PreviousProblem);
    assert_eq!(session.selection(), Some(&Selection::Problem(last)));
    session.apply(Command::NextProblem);
    assert_eq!(
        session.selection(),
        Some(&Selection::Problem(ProblemIndex(0)))
    );
    // From anything else, Previous goes to the last.
    session.apply(Command::Select(None));
    session.apply(Command::PreviousProblem);
    assert_eq!(session.selection(), Some(&Selection::Problem(last)));
}

#[test]
fn stepping_does_nothing_without_problems() {
    let mut session = checked_session(example("stool"));
    assert!(session.problems().is_empty());
    assert!(session.apply(Command::NextProblem).is_empty());
    assert!(session.apply(Command::PreviousProblem).is_empty());
}

const PROJECT: &str = "stave 1

project \"Test\" {
  units: mm
}

stock pine_19 { species: pine, form: solid, thickness: 19mm }

part base : pine_19 {
  length: 600mm
  width:  300mm
  place {
    ground
  }
}
";

const SHELF: &str = "
part shelf : pine_19 {
  length: 600mm
  width:  200mm
  place {
    against(base.top)
    flush(base.left)
    flush(base.back)
  }
}
";

fn memory_check(text: &str) -> Outcome {
    let mut memory = MemoryFiles::new();
    memory.insert(Path::new("/project/project.stave"), text);
    let opened = stave_app::open_with(Path::new("/project"), &memory).expect("it loads");
    Outcome::Checked(Box::new(check::checked(
        Path::new("/project"),
        opened,
        Instant::now(),
    )))
}

#[test]
fn the_selection_and_history_follow_their_names_into_the_next_result() {
    let mut session = Session::new(PathBuf::from("/project"));
    let ticket = started(&session.apply(Command::Check)).unwrap();
    session.finish_check(ticket, memory_check(&format!("{PROJECT}{SHELF}")));
    let shelf = part(&session, "shelf");
    let base = part(&session, "base");
    session.apply(Command::Select(Some(base)));
    session.apply(Command::Select(Some(shelf)));

    // A part declared first renumbers the others.
    let edited = PROJECT.replace(
        "part base",
        "part cleat : pine_19 {\n  length: 100mm\n  width: 50mm\n  place {\n    against(base.bottom)\n    flush(base.left)\n    flush(base.back)\n  }\n}\n\npart base",
    );
    let ticket = started(&session.apply(Command::Check)).unwrap();
    session.finish_check(ticket, memory_check(&format!("{edited}{SHELF}")));
    assert_eq!(session.selection(), Some(&part(&session, "shelf")));
    session.apply(Command::Back);
    assert_eq!(session.selection(), Some(&part(&session, "base")));

    // A design that no longer solves keeps the last one on show.
    let ticket = started(&session.apply(Command::Check)).unwrap();
    session.finish_check(
        ticket,
        memory_check(&format!("{PROJECT}\npart broken : pine_19 {{\n}}\n")),
    );
    assert!(session.showing_older());
    assert!(session.drawable().is_some());
    assert_eq!(session.check_status().pill().tone, Tone::Errors);
}

#[test]
fn a_selected_location_survives_a_new_result() {
    let mut session = checked_session(example("stool"));
    let location = SourceLocation {
        path: example("stool").join("stool.stave").display().to_string(),
        line: stave_app::session::LineNumber::new(3).unwrap(),
    };
    session.apply(Command::GoToSource(location.clone()));
    let ticket = started(&session.apply(Command::Check)).unwrap();
    session.finish_check(ticket, check::run(&example("stool")));
    assert_eq!(session.selection(), Some(&Selection::Location(location)));
}

#[test]
fn the_parts_tree_lists_the_carcass_then_the_assemblies() {
    let session = checked_session(example("clothing_cabinet"));
    let (design, _) = session.drawable().unwrap();
    let tree = inspect::parts_tree(design);
    let carcass: Vec<&str> = tree.carcass.iter().map(|e| e.label.as_str()).collect();
    assert!(carcass.contains(&"side_l, side_r"), "{carcass:?}");
    let assemblies: Vec<(&str, Option<&str>)> = tree
        .assemblies
        .iter()
        .map(|a| (a.label.as_str(), a.detail.as_deref()))
        .collect();
    assert!(
        assemblies.contains(&("door_l", Some("turns 110°"))),
        "{assemblies:?}"
    );
    assert!(
        assemblies.contains(&("door_r", Some("mirror of door_l"))),
        "{assemblies:?}"
    );
    let door = tree
        .assemblies
        .iter()
        .find(|a| a.label == "door_l")
        .unwrap();
    let rows: Vec<&str> = door.entries.iter().map(|e| e.label.as_str()).collect();
    assert!(
        rows.contains(&"stile[hinge, latch]") || rows.contains(&"stile[hinge]"),
        "{rows:?}"
    );
    assert!(rows.contains(&"hinges[lower, upper]"), "{rows:?}");
    let hinges = door
        .entries
        .iter()
        .find(|e| e.label == "hinges[lower, upper]")
        .unwrap();
    assert_eq!(hinges.swatch.shape, SwatchShape::Round);
    assert_eq!(hinges.items.len(), 2);

    let found = tree.filtered("handle");
    assert!(found.carcass.is_empty());
    assert!(!found.assemblies.is_empty());
    assert!(tree.filtered("no such thing").is_empty());
}

#[test]
fn the_inspector_describes_a_door_part() {
    let mut session = checked_session(example("clothing_cabinet"));
    let stile = part(&session, "door_l.stile[hinge]");
    session.apply(Command::Select(Some(stile.clone())));
    let shown = session.shown().unwrap().clone();
    let inspection = inspect::inspect(&shown, &stile).unwrap();
    assert_eq!(inspection.eyebrow, "PART · IN door_l");
    assert_eq!(inspection.title, "stile[hinge]");
    let rows: Vec<String> = inspection
        .rows
        .iter()
        .map(|r| format!("{}: {}", r.label, r.value))
        .collect();
    assert!(
        rows.iter().any(|r| r.starts_with("Stock: oak_19 · ")),
        "{rows:?}"
    );
    assert!(
        rows.contains(&"Grain: along the length".to_owned()),
        "{rows:?}"
    );
    assert!(
        rows.contains(&"Moves: with door_l, turns 110°".to_owned()),
        "{rows:?}"
    );
    let leads: Vec<&str> = inspection
        .connections
        .iter()
        .map(|c| c.lead.as_str())
        .collect();
    assert!(leads.iter().any(|l| l.contains("bore")), "{leads:?}");
    assert!(inspection.connections.iter().all(|c| c.target.is_some()));
    let source = inspection.source.unwrap();
    let first_line: String = source.lines[0].iter().map(|t| t.text.as_str()).collect();
    assert_eq!(first_line, "assembly door_l : casework.FramePanelDoor {");
}

#[test]
fn the_title_is_what_the_folder_declares_and_its_name_only_until_then() {
    let folder = example("japanese_joinery");
    let session = Session::new(folder.clone());
    assert_eq!(session.title(), "japanese_joinery");
    let session = checked_session(folder);
    assert_eq!(session.title(), "Japanese joinery samples");
    assert_eq!(checked_session(example("stool")).title(), "Stool");
    assert_eq!(
        checked_session(example("clothing_cabinet")).title(),
        "Clothing cabinet"
    );
}

#[test]
fn a_design_with_errors_keeps_its_declared_title() {
    let mut session = Session::new(PathBuf::from("/project"));
    let ticket = started(&session.apply(Command::Check)).unwrap();
    session.finish_check(
        ticket,
        memory_check(&format!("{PROJECT}\npart broken : pine_19 {{\n}}\n")),
    );
    assert_eq!(session.title(), "Test");
}

#[test]
fn a_folder_that_declares_no_title_is_named_after_itself() {
    let mut session = Session::new(PathBuf::from("/project"));
    let ticket = started(&session.apply(Command::Check)).unwrap();
    session.finish_check(
        ticket,
        memory_check("stave 1\n\nproject \"  \" {\n  units: mm\n}\n"),
    );
    assert_eq!(session.title(), "project");
}

#[test]
fn only_a_project_with_parts_and_no_errors_can_be_built() {
    let session = checked_session(example("stool"));
    assert_eq!(session.buildable(), Ok(()));
    assert_eq!(session.build_tooltip(), "Build plan (Ctrl+B)");

    let mut session = checked_session(example("japanese_joinery"));
    assert_eq!(session.buildable(), Err(Reason::Library));
    assert_eq!(
        session.build_tooltip(),
        "Nothing to build: a library has no parts"
    );
    // The button is off, and the command does nothing either.
    assert!(session.apply(Command::Build).is_empty());
    assert_eq!(session.build(), Build::Idle);

    let session = Session::new(example("stool"));
    assert_eq!(session.buildable(), Err(Reason::Checking));

    let folder = workspace().join("tests/no_such_folder");
    let mut session = Session::new(folder.clone());
    let ticket = started(&session.apply(Command::Check)).unwrap();
    session.finish_check(ticket, check::run(&folder));
    assert_eq!(session.buildable(), Err(Reason::CantOpen));
    assert_eq!(
        session.build_tooltip(),
        "Nothing to build: the folder couldn't be opened"
    );

    let mut session = Session::new(PathBuf::from("/project"));
    let ticket = started(&session.apply(Command::Check)).unwrap();
    session.finish_check(ticket, memory_check(&format!("{PROJECT}{SHELF}")));
    assert_eq!(session.buildable(), Ok(()));
    // Errors turn it off, even while the last good design is on show.
    let ticket = started(&session.apply(Command::Check)).unwrap();
    session.finish_check(
        ticket,
        memory_check(&format!("{PROJECT}\npart broken : pine_19 {{\n}}\n")),
    );
    assert!(session.showing_older());
    assert_eq!(session.buildable(), Err(Reason::Errors));
    assert_eq!(
        session.build_tooltip(),
        "Nothing to build yet: fix the problems first"
    );

    let mut session = Session::new(PathBuf::from("/project"));
    let ticket = started(&session.apply(Command::Check)).unwrap();
    session.finish_check(
        ticket,
        memory_check("stave 1\n\nproject \"Empty\" {\n  units: mm\n}\n"),
    );
    // A project without parts has an error (nothing stands on the ground).
    assert_eq!(session.buildable(), Err(Reason::Errors));
    assert_eq!(
        Reason::NoParts.nothing_to_build(),
        "Nothing to build: the project has no parts yet"
    );
}
