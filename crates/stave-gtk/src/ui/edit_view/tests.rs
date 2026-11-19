//! The editing widgets on the real session: the menu and the card drawn
//! from what the session holds, what the user does on them turned into
//! the session's commands, and the commit written through a text buffer
//! that one undo takes back. The window itself (its 3D view, the pointer)
//! is shown by `tools/gui-screenshots`; these drive the same functions it
//! calls.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use super::*;
use crate::ui::edit_writer::apply_to_buffer;
use crate::ui::gtk_test::on_gtk_thread;
use crate::ui::new_project::{
    Answers, NewProjectDialog, create_project, importable_libraries, standard_tools,
};
use stave_app::candidates::new_project::BoardForm;
use stave_app::editing::entry_matches;
use stave_app::session::Event;
use stave_lang::placement::Face;
use stave_schema::species::Species;

/// A fresh copy of the door fixture (a cabinet against a wall, with a
/// `Door` component whose door swings into the wall unless its overlay
/// is made smaller) in a temporary folder of its own.
fn door_project(name: &str) -> PathBuf {
    let folder = std::env::temp_dir().join(format!("stave-gtk-edit-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    let source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/door_hits_wall/cabinet.stave");
    std::fs::copy(source, folder.join("cabinet.stave")).unwrap();
    folder.canonicalize().unwrap()
}

/// Sends a command and keeps only the edit events.
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

/// The ids of the fixture's bay and left side.
fn bay_and_side(folder: &Path) -> (RegionId, stave_model::solved::PartId) {
    let opened = stave_app::open(folder).unwrap();
    let model = opened.model.as_ref().unwrap();
    let bay = model.regions.iter().find(|r| r.name == "bay").unwrap().id;
    let side = model.parts.iter().find(|p| p.name == "side_l").unwrap().id;
    (bay, side)
}

#[test]
fn card_actions_become_the_sessions_commands() {
    let name = |text: &str| parse_name(text).unwrap();
    assert_eq!(
        command_for(CardAction::Rename("tall_door".to_owned())),
        Ok(EditCommand::Rename(name("tall_door")))
    );
    assert!(
        command_for(CardAction::Rename("two words".to_owned()))
            .unwrap_err()
            .starts_with("'two words' can't be a name")
    );
    assert_eq!(
        command_for(CardAction::Choose {
            param: name("mount"),
            text: "overlay()".to_owned(),
        }),
        Ok(EditCommand::Set {
            name: name("mount"),
            value: parse_value("overlay()").unwrap(),
        })
    );
    assert!(
        command_for(CardAction::Choose {
            param: name("mount"),
            text: "overlay(".to_owned(),
        })
        .unwrap_err()
        .starts_with("'overlay(' isn't a value Stave can read")
    );
    assert_eq!(
        command_for(CardAction::Fix { problem: 1, fix: 0 }),
        Ok(EditCommand::ApplyFix { problem: 1, fix: 0 })
    );
    assert_eq!(command_for(CardAction::PutBack), Ok(EditCommand::PutBack));
    assert!(tries_again(&EditCommand::PutBack));
    assert_eq!(command_for(CardAction::Add), Ok(EditCommand::Add));
    assert_eq!(command_for(CardAction::Cancel), Ok(EditCommand::Cancel));
    // Choosing, binding, setting, renaming and fixing try it again; the
    // card shows Checking until they have.
    assert!(tries_again(&EditCommand::ApplyFix { problem: 0, fix: 0 }));
    assert!(!tries_again(&EditCommand::Add));
    assert!(!tries_again(&EditCommand::Cancel));
}

#[test]
fn the_card_shows_checking_then_the_tries_verdict() {
    assert_eq!(
        card_verdict(&Progress::Unwritable("no".to_owned()), true),
        Some(Verdict::Checking)
    );
    let Some(Verdict::Refused(refusal)) =
        card_verdict(&Progress::Unwritable("the name is taken".to_owned()), false)
    else {
        panic!("an unwritable candidate shows as refused");
    };
    assert_eq!(
        refusal.problems().first().diagnostic().message,
        "the name is taken"
    );
    assert_eq!(verdict_words(GhostVerdict::Refused), "✕ It doesn't fit");
    assert_eq!(verdict_words(GhostVerdict::Fits), "✓ It fits");
    assert_eq!(ghost_tint(GhostVerdict::Fits)[3], GHOST_TINT);
}

#[test]
fn a_new_project_is_written_from_the_dialogs_answers_and_checks() {
    let tools = standard_tools();
    assert!(
        tools
            .iter()
            .any(|tool| tool.name.as_str() == "tenon_saw" && tool.title == "tenon saw"),
        "{tools:?}"
    );
    assert_eq!(importable_libraries(), ["casework", "frames"]);
    let answers = Answers {
        title: "Hall table".to_owned(),
        size: [900.0, 760.0, 400.0],
        stock_name: "oak_19".to_owned(),
        species: Species::WhiteOak,
        form: BoardForm::Solid,
        thickness: 19.0,
        max_length: 2400.0,
        imports: vec!["std:casework".to_owned()],
        tools: tools.iter().take(2).map(|tool| tool.name.clone()).collect(),
    };
    let project = answers.project().unwrap();
    let folder = std::env::temp_dir().join(format!("stave-gtk-new-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    let path = create_project(&folder, &project).unwrap();
    assert_eq!(path, folder.join("hall_table.stave"));
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("region envelope = box("), "{text}");
    assert!(text.contains("import \"std:casework\""), "{text}");
    let opened = stave_app::open(&folder).unwrap();
    assert_eq!(opened.error_count(), 0, "{:#?}", opened.diagnostics);
    assert!(
        opened
            .model
            .as_ref()
            .unwrap()
            .regions
            .iter()
            .any(|region| region.name == stave_app::naming::ENVELOPE_REGION)
    );
    // Never over a file that is there.
    assert!(create_project(&folder, &project).is_err());
    // What is missing is said in words.
    let unnamed = Answers {
        title: "  ".to_owned(),
        ..answers.clone()
    };
    assert_eq!(unnamed.project().unwrap_err(), "Give the project a name");
    let flat = Answers {
        size: [900.0, 0.0, 400.0],
        ..answers
    };
    assert!(flat.project().unwrap_err().contains("isn't a size"));
}

#[test]
fn the_menu_and_the_card_run_on_the_session() {
    on_gtk_thread(menu_and_card_on_the_session);
}

fn menu_and_card_on_the_session() {
    let folder = door_project("card");
    let original = std::fs::read_to_string(folder.join("cabinet.stave")).unwrap();
    let (bay, side_l) = bay_and_side(&folder);
    let mut session = Session::new(folder.clone());
    let menu = EditMenu::new();
    let card = EditCard::new();
    let chosen: Rc<RefCell<Vec<Declaration>>> = Rc::default();
    {
        let chosen = chosen.clone();
        menu.connect_chosen(move |declaration| chosen.borrow_mut().push(declaration));
    }
    let actions: Rc<RefCell<Vec<CardAction>>> = Rc::default();
    {
        let actions = actions.clone();
        card.connect_action(move |action| actions.borrow_mut().push(action));
    }
    let take = |actions: &Rc<RefCell<Vec<CardAction>>>| -> Vec<CardAction> {
        actions.borrow_mut().drain(..).collect()
    };

    // Nothing in progress: neither shows.
    assert!(!draw_edit(&menu, &card, &session, false));
    assert!(!menu.root.is_visible() && !card.root.is_visible());

    // A click in the bay: the menu, every entry not checked yet, then
    // checked (a step at a time in the window).
    assert!(
        send(&mut session, EditCommand::Start(Picked::Region(bay))).contains(&EditEvent::Started)
    );
    assert!(draw_edit(&menu, &card, &session, false));
    assert!(menu.root.is_visible() && !card.root.is_visible());
    let edit = session.edit().unwrap();
    assert_eq!(unchecked(edit), edit.menu().len());
    let stop = AtomicBool::new(false);
    session.check_edit_menu(&stop);
    assert_eq!(unchecked(session.edit().unwrap()), 0);
    draw_edit(&menu, &card, &session, false);
    let rows = menu.rows();
    let door = session
        .edit()
        .unwrap()
        .menu()
        .iter()
        .find(|candidate| candidate.entry.label.as_str() == "Door")
        .unwrap()
        .declaration;
    let (_, button) = rows
        .iter()
        .find(|(declaration, _)| *declaration == door)
        .unwrap();
    button.emit_clicked();
    let declaration = chosen.borrow_mut().pop().expect("the Door entry reports");
    let index = entry_index(session.edit().unwrap(), declaration).unwrap();
    assert!(send(&mut session, EditCommand::Choose(index)).contains(&EditEvent::Chosen));

    // The card: the hinge side still to point at, so no verdict yet.
    draw_edit(&menu, &card, &session, true);
    assert!(card.root.is_visible() && !menu.root.is_visible());
    let chosen_door = session.edit().unwrap().chosen().unwrap();
    assert_eq!(card_verdict(chosen_door.progress(), false), None);
    // The next click on the model binds it.
    send(
        &mut session,
        EditCommand::Bind(Picked::Face(PartFace {
            part: side_l,
            face: Face::Right,
        })),
    );
    draw_edit(&menu, &card, &session, false);
    // The mount is chosen from its list of options, or typed.
    let lists = card.option_lists();
    assert!(
        lists.iter().any(|(param, _, _)| param.as_str() == "mount"),
        "{:?}",
        lists
            .iter()
            .map(|(param, _, _)| param.as_str())
            .collect::<Vec<_>>()
    );
    let (_, mount) = card
        .entries()
        .into_iter()
        .find(|(param, _)| param.as_str() == "mount")
        .unwrap();
    mount.set_text("overlay(left: 17.8mm, right: 17.8mm)");
    mount.emit_activate();
    for action in take(&actions) {
        send(&mut session, command_for(action).unwrap());
    }
    // Refused: the door swings into the wall, with the checker's fix.
    let chosen_door = session.edit().unwrap().chosen().unwrap();
    assert!(
        matches!(chosen_door.progress(), Progress::Tried(Verdict::Refused(_))),
        "{:#?}",
        chosen_door.progress()
    );
    assert_eq!(
        ghost_verdict(chosen_door.progress()),
        Some(GhostVerdict::Refused)
    );
    assert!(
        chosen_door
            .ghost()
            .and_then(|opened| opened.design.as_ref())
            .is_some()
    );
    draw_edit(&menu, &card, &session, false);
    let (add, cancel) = card.buttons();
    assert!(!add.is_sensitive());
    let fixes = card.fix_buttons();
    let ((problem, fix), button) = fixes.first().expect("a fix is offered");
    assert_eq!((*problem, *fix), (0, 0));
    button.emit_clicked();
    for action in take(&actions) {
        send(&mut session, command_for(action).unwrap());
    }
    // The fix makes it fit: Add is enabled.
    draw_edit(&menu, &card, &session, false);
    let chosen_door = session.edit().unwrap().chosen().unwrap();
    assert_eq!(
        ghost_verdict(chosen_door.progress()),
        Some(GhostVerdict::Fits)
    );
    assert!(add.is_sensitive());
    assert!(cancel.is_sensitive());

    // Add writes the door into the text, as one edit.
    add.emit_clicked();
    let events: Vec<EditEvent> = take(&actions)
        .into_iter()
        .flat_map(|action| send(&mut session, command_for(action).unwrap()))
        .collect();
    assert!(events.contains(&EditEvent::Committed), "{events:?}");
    assert!(!draw_edit(&menu, &card, &session, false));
    let written = std::fs::read_to_string(folder.join("cabinet.stave")).unwrap();
    assert!(written.contains("assembly door : Door {"), "{written}");
    // The window selects the new item by its name.
    let (design, _) = session.drawable().unwrap();
    assert!(matches!(
        stave_app::link::selection_named(design, "door"),
        Some(Selection::Assembly(_))
    ));

    // The same commit through the editor's buffer: one undo takes it back.
    let commit = session.last_commit().first().unwrap().clone();
    assert_eq!(commit.before, original);
    let buffer = gtk::TextBuffer::new(None);
    buffer.set_enable_undo(true);
    buffer.begin_irreversible_action();
    buffer.set_text(&original);
    buffer.end_irreversible_action();
    apply_to_buffer(&buffer, &commit).unwrap();
    let text =
        |buffer: &gtk::TextBuffer| buffer.text(&buffer.start_iter(), &buffer.end_iter(), true);
    assert_eq!(text(&buffer), commit.after);
    buffer.undo();
    assert_eq!(text(&buffer), original);
    // A buffer that no longer holds the text the edit began from is
    // refused, untouched.
    buffer.set_text("stave 1\n");
    assert!(apply_to_buffer(&buffer, &commit).is_err());
    assert_eq!(text(&buffer), "stave 1\n");

    // Cancel (or Escape) leaves no trace.
    std::fs::write(folder.join("cabinet.stave"), &original).unwrap();
    let mut session = Session::new(folder.clone());
    send(&mut session, EditCommand::Start(Picked::Region(bay)));
    let index = session
        .edit()
        .unwrap()
        .menu()
        .iter()
        .position(|candidate| candidate.entry.label.as_str() == "Door")
        .unwrap();
    send(&mut session, EditCommand::Choose(index));
    draw_edit(&menu, &card, &session, false);
    let (_, cancel) = card.buttons();
    cancel.emit_clicked();
    for action in take(&actions) {
        assert!(send(&mut session, command_for(action).unwrap()).contains(&EditEvent::Cancelled));
    }
    assert!(!draw_edit(&menu, &card, &session, false));
    assert_eq!(
        std::fs::read_to_string(folder.join("cabinet.stave")).unwrap(),
        original
    );

    // The New Project dialog reads its fields as a project that writes.
    let dialog = NewProjectDialog::new();
    let project = dialog.answers().project().unwrap();
    assert_eq!(project.title, "Hall table");
    assert_eq!(project.imports, ["std:casework", "std:frames"]);
    assert!(project.tools.is_empty(), "none ticked: every tool");
}

/// A session with the folder checked, as the window has once the first
/// result is in.
fn checked_session(folder: &Path) -> Session {
    let mut session = Session::new(folder.to_owned());
    let ticket = session
        .apply(Command::Check)
        .into_iter()
        .find_map(|event| match event {
            Event::CheckStarted(ticket) => Some(ticket),
            _ => None,
        })
        .expect("a check starts");
    session.finish_check(ticket, stave_app::check::run(folder));
    session
}

/// A project as the New Project dialog writes it, in a folder of its own.
fn new_project(name: &str, imports: &[&str]) -> PathBuf {
    let answers = Answers {
        title: "Hall table".to_owned(),
        size: [900.0, 760.0, 400.0],
        stock_name: "oak_19".to_owned(),
        species: Species::WhiteOak,
        form: BoardForm::Solid,
        thickness: 19.0,
        max_length: 2400.0,
        imports: imports.iter().map(|import| (*import).to_owned()).collect(),
        tools: Vec::new(),
    };
    let folder =
        std::env::temp_dir().join(format!("stave-gtk-opened-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    create_project(&folder, &answers.project().unwrap()).unwrap();
    folder.canonicalize().unwrap()
}

#[test]
fn a_new_project_opens_showing_its_envelope_to_click_in() {
    let folder = new_project("envelope", &["std:casework"]);
    let mut session = checked_session(&folder);
    // No parts yet, but the envelope is a space: the view draws it rather
    // than saying "No parts yet".
    assert!(matches!(
        stave_app::empty::of(&session).map(|empty| empty.reason),
        Some(stave_app::empty::Reason::NoParts)
    ));
    assert!(empty_state(&session).is_none());
    let (_, graph) = session.drawable().unwrap();
    assert!(pointing::only_spaces(graph));
    let posed = graph.rest();
    let envelope = graph
        .regions()
        .iter()
        .find(|region| region.name == stave_app::naming::ENVELOPE_REGION)
        .unwrap()
        .id;
    // Shown lit while nothing is edited, so there is a box to click in.
    assert_eq!(
        pointing::spaces_waiting(graph, &posed, false),
        vec![graph.region_volume(&posed, envelope).unwrap()]
    );
    assert!(pointing::spaces_waiting(graph, &posed, true).is_empty());
    // The camera frames it (a scene with no nodes has no bounds of its
    // own), and a click in the middle of the view lands in it.
    assert!(graph.bounds(&posed).is_none());
    let bounds = drawn_bounds(graph, &posed).expect("the envelope is framed");
    let orbit = Orbit::framing(bounds, 1.6);
    let ray = orbit.ray(800.0, 500.0, 1600.0, 1000.0);
    let Some(Target::Region(hit)) = graph.pick_target(&posed, ray) else {
        panic!("a click in the middle lands in the envelope");
    };
    assert_eq!(hit.region, envelope);
    // That click opens the menu for the space.
    let events = send(&mut session, EditCommand::Start(Picked::Region(envelope)));
    assert!(events.contains(&EditEvent::Started), "{events:?}");
    assert!(!session.edit().unwrap().menu().is_empty());
}

/// A ray from in front and to the right of the cabinet to the middle of
/// the inside face of its left side: it passes through the bay first.
fn ray_to_inside_of(graph: &SceneGraph, posed: &Posed, part: stave_model::solved::PartId) -> Ray {
    let nodes = graph.nodes_of(stave_scene::Item::Part(part));
    let side = graph.bounds_of(posed, &nodes).unwrap();
    let target = glam::Vec3::new(side.max.x, side.centre().y, side.centre().z);
    let origin = target + glam::Vec3::new(1500.0, 0.0, 2000.0);
    Ray {
        origin,
        direction: (target - origin).normalize(),
    }
}

#[test]
fn a_click_that_binds_a_part_reaches_it_through_the_space() {
    let folder = door_project("bind");
    let (bay, side_l) = bay_and_side(&folder);
    let mut session = checked_session(&folder);
    send(&mut session, EditCommand::Start(Picked::Region(bay)));
    let index = session
        .edit()
        .unwrap()
        .menu()
        .iter()
        .position(|candidate| candidate.entry.label.as_str() == "Door")
        .unwrap();
    assert!(send(&mut session, EditCommand::Choose(index)).contains(&EditEvent::Chosen));
    // The card asks for the hinge side, a part: not a space.
    let wanted = pointing::Wanted::of(session.edit());
    assert!(wanted.face && !wanted.region && !wanted.parts, "{wanted:?}");
    let picked = {
        let (_, graph) = session.drawable().unwrap();
        let posed = graph.rest();
        let ray = ray_to_inside_of(graph, &posed, side_l);
        // Outside binding, the bay wins over its own wall...
        assert!(matches!(
            graph.pick_target(&posed, ray),
            Some(Target::Region(hit)) if hit.region == bay
        ));
        // ...so a space asked for still gets the bay.
        assert_eq!(
            pointing::binding_pick(
                graph,
                &posed,
                ray,
                pointing::Wanted {
                    region: true,
                    ..pointing::Wanted::default()
                }
            ),
            Some(Picked::Region(bay))
        );
        // While a part is wanted, the click is on the side's inside face.
        pointing::binding_pick(graph, &posed, ray, wanted)
    };
    assert_eq!(
        picked,
        Some(Picked::Face(PartFace {
            part: side_l,
            face: Face::Right,
        }))
    );
    let events = send(&mut session, EditCommand::Bind(picked.unwrap()));
    assert!(events.contains(&EditEvent::PlaceBound), "{events:?}");
    assert!(!pointing::Wanted::of(session.edit()).any());
}

#[test]
fn a_menu_entry_is_found_by_its_declaration_when_it_is_chosen() {
    // Two components of the project's own for a space: "Bench" is listed
    // first but greys (its own rule refuses the envelope), "Crate" fits.
    let folder = new_project("menu", &[]);
    let file = folder.join("hall_table.stave");
    let mut text = std::fs::read_to_string(&file).unwrap();
    let component = |name: &str, rule: &str| {
        format!(
            "\n/// A block of oak.\ncomponent {name} {{\n  param space : Region\n{rule}  part block : oak_19 {{\n    length: 100mm\n    width:  50mm\n    place {{\n      flush(space.bottom)\n      flush(space.left)\n      flush(space.front)\n    }}\n  }}\n}}\n"
        )
    };
    text.push_str(&component(
        "Bench",
        "  require tall_enough : space.up > 5000mm \"the space is too low\"\n",
    ));
    text.push_str(&component("Crate", ""));
    std::fs::write(&file, text).unwrap();
    let mut session = checked_session(&folder);
    let (_, graph) = session.drawable().unwrap();
    let envelope = graph
        .regions()
        .iter()
        .find(|region| region.name == stave_app::naming::ENVELOPE_REGION)
        .unwrap()
        .id;
    send(&mut session, EditCommand::Start(Picked::Region(envelope)));
    // Where each entry was when the menu was first drawn (when it could
    // be clicked)...
    let before: Vec<Declaration> = session
        .edit()
        .unwrap()
        .menu()
        .iter()
        .map(|candidate| candidate.declaration)
        .collect();
    // ...and after a step of the check sorted greyed entries last.
    session.check_edit_menu(&AtomicBool::new(false));
    let edit = session.edit().unwrap();
    let states: Vec<_> = edit
        .menu()
        .iter()
        .map(|candidate| (candidate.entry.label.as_str(), &candidate.applicability))
        .collect();
    let moved = before
        .iter()
        .enumerate()
        .find(|(index, declaration)| entry_index(edit, **declaration) != Some(*index))
        .map(|(index, declaration)| (index, *declaration));
    let (stale, declaration) =
        moved.unwrap_or_else(|| panic!("checking moves an entry: {states:#?}"));
    // The index the click saw now names another entry; looked up when the
    // command is sent, the clicked entry is the one chosen.
    assert_ne!(
        edit.menu()
            .get(stale)
            .map(|candidate| candidate.declaration),
        Some(declaration)
    );
    let index = entry_index(edit, declaration).unwrap();
    send(&mut session, EditCommand::Choose(index));
    assert_eq!(
        session
            .edit()
            .unwrap()
            .chosen()
            .unwrap()
            .candidate()
            .declaration,
        declaration
    );
}

#[test]
fn the_cards_options_show_their_fit_and_a_blamed_setting_can_be_put_back() {
    on_gtk_thread(options_and_blame_on_the_card);
}

/// The owner's case: the Case chosen for a new project's envelope, its
/// joinery's options checked and drawn, a refused one that can't be
/// chosen, then chigiri typed, blamed, and put back from the card.
fn options_and_blame_on_the_card() {
    let answers = Answers {
        title: "Case test".to_owned(),
        size: [940.0, 1780.0, 580.0],
        stock_name: "oak_19".to_owned(),
        species: Species::WhiteOak,
        form: BoardForm::Solid,
        thickness: 19.0,
        max_length: 2700.0,
        imports: vec!["std:casework".to_owned()],
        tools: Vec::new(),
    };
    let folder = std::env::temp_dir().join(format!("stave-gtk-options-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    create_project(&folder, &answers.project().unwrap()).unwrap();
    let mut session = Session::new(folder.clone());
    let envelope = {
        let opened = stave_app::open(&folder).unwrap();
        let model = opened.model.as_ref().unwrap();
        model
            .regions
            .iter()
            .find(|r| r.name == "envelope")
            .unwrap()
            .id
    };
    send(&mut session, EditCommand::Start(Picked::Region(envelope)));
    let index = session
        .edit()
        .unwrap()
        .menu()
        .iter()
        .position(|candidate| entry_matches(candidate, "casework.Case"))
        .unwrap();
    send(&mut session, EditCommand::Choose(index));
    send(
        &mut session,
        EditCommand::Set {
            name: parse_name("panel_stock").unwrap(),
            value: parse_value("oak_19").unwrap(),
        },
    );
    session.check_setting_options(Some("joinery"), &AtomicBool::new(false));

    let menu = EditMenu::new();
    let card = EditCard::new();
    let actions: Rc<RefCell<Vec<CardAction>>> = Rc::default();
    {
        let actions = actions.clone();
        card.connect_action(move |action| actions.borrow_mut().push(action));
    }
    draw_edit(&menu, &card, &session, false);
    assert!(card.open_options("joinery"));
    let (_, toggle, list) = card
        .option_lists()
        .into_iter()
        .find(|(param, _, _)| param.as_str() == "joinery")
        .unwrap();
    assert!(toggle.is_active());
    let rows: Vec<gtk::Button> = {
        let mut rows = Vec::new();
        let mut child = list.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            if let Ok(button) = widget.downcast::<gtk::Button>() {
                rows.push(button);
            }
        }
        rows
    };
    assert_eq!(rows.len(), 18);
    let row = |text: &str| -> gtk::Button {
        rows.iter()
            .find(|row| {
                // A row's first label is its option's text.
                let mut words = String::new();
                let mut stack = vec![row.upcast_ref::<gtk::Widget>().clone()];
                while let Some(widget) = stack.pop() {
                    if let Some(label) = widget.downcast_ref::<gtk::Label>() {
                        words.push_str(&label.label());
                        words.push('\n');
                    }
                    let mut child = widget.first_child();
                    while let Some(next) = child {
                        child = next.next_sibling();
                        stack.push(next);
                    }
                }
                words.lines().any(|line| line == text)
            })
            .unwrap_or_else(|| panic!("no row for {text}"))
            .clone()
    };
    // A refused option is greyed, still focusable to read why, and never
    // reports; one that fits reports the value.
    let chigiri = row("chigiri");
    assert!(chigiri.has_css_class("stave-edit-unavailable"));
    assert!(chigiri.is_focusable());
    chigiri.emit_clicked();
    assert!(actions.borrow().is_empty());
    let dado = row("dado");
    assert!(!dado.has_css_class("stave-edit-unavailable"));
    dado.emit_clicked();
    assert_eq!(
        actions.borrow_mut().drain(..).collect::<Vec<_>>(),
        [CardAction::Choose {
            param: parse_name("joinery").unwrap(),
            text: "dado".to_owned(),
        }]
    );

    // chigiri typed: the card puts the blame first, with the way back.
    send(
        &mut session,
        EditCommand::Set {
            name: parse_name("joinery").unwrap(),
            value: parse_value("chigiri").unwrap(),
        },
    );
    draw_edit(&menu, &card, &session, false);
    let put_back = card
        .put_back_button()
        .expect("the card offers the way back");
    assert_eq!(
        put_back.label().map(|label| label.to_string()),
        Some("Put joinery back to dowel(count: 4), the default".to_owned())
    );
    put_back.emit_clicked();
    for action in actions.borrow_mut().drain(..).collect::<Vec<_>>() {
        send(&mut session, command_for(action).unwrap());
    }
    let chosen = session.edit().unwrap().chosen().unwrap();
    assert!(matches!(
        chosen.progress(),
        Progress::Tried(Verdict::Fits(_))
    ));
    draw_edit(&menu, &card, &session, false);
    assert!(card.put_back_button().is_none());
}
