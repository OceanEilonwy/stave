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
use crate::ui::gtk_test::{on_gtk_thread, settle};
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

/// The menu and the card as the window launches them (`EditView::new`),
/// with what they tell it recorded instead of sent to a window.
struct Launched {
    menu: Controller<EditMenuComponent>,
    card: Controller<EditCardComponent>,
    slots: CardSlots,
    chosen: Rc<RefCell<Vec<Declaration>>>,
    actions: Rc<RefCell<Vec<CardAction>>>,
    /// Each [`EditCardOutput::OptionsOpen`], in order.
    open: Rc<RefCell<Vec<Option<String>>>>,
}

impl Launched {
    fn new() -> Launched {
        let chosen: Rc<RefCell<Vec<Declaration>>> = Rc::default();
        let menu = {
            let chosen = chosen.clone();
            EditMenuComponent::builder().launch(()).connect_receiver(
                move |_, output| match output {
                    EditMenuOutput::Chosen(declaration) => chosen.borrow_mut().push(declaration),
                    EditMenuOutput::Drawn => {}
                },
            )
        };
        let actions: Rc<RefCell<Vec<CardAction>>> = Rc::default();
        let open: Rc<RefCell<Vec<Option<String>>>> = Rc::default();
        let slots = CardSlots::new();
        let card = {
            let actions = actions.clone();
            let open = open.clone();
            EditCardComponent::builder()
                .launch(slots.clone())
                .connect_receiver(move |_, output| match output {
                    EditCardOutput::Action(action) => actions.borrow_mut().push(action),
                    EditCardOutput::OptionsOpen(setting) => open.borrow_mut().push(setting),
                    EditCardOutput::Link(_) | EditCardOutput::Hover(_) => {}
                })
        };
        Launched {
            menu,
            card,
            slots,
            chosen,
            actions,
            open,
        }
    }

    /// Draws the edit as the window does, then runs the main loop so the
    /// menu and the card draw what they were sent.
    fn draw(&self, session: &Session, checking: bool) -> Drawn {
        let drawn = draw_edit(&self.menu, &self.card, session, checking);
        settle();
        drawn
    }

    /// What the card reported since this was last asked.
    fn take_actions(&self) -> Vec<CardAction> {
        self.actions.borrow_mut().drain(..).collect()
    }

    fn menu_shows(&self) -> bool {
        self.menu.widget().is_visible()
    }

    fn card_shows(&self) -> bool {
        self.card.widget().is_visible()
    }
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
    let editing = Launched::new();

    // Nothing in progress: neither shows.
    assert_eq!(editing.draw(&session, false), Drawn::Nothing);
    assert!(!editing.menu_shows() && !editing.card_shows());

    // A click in the bay: the menu, every entry not checked yet, then
    // checked (a step at a time in the window).
    assert!(
        send(&mut session, EditCommand::Start(Picked::Region(bay))).contains(&EditEvent::Started)
    );
    assert_eq!(editing.draw(&session, false), Drawn::Menu);
    assert!(editing.menu_shows() && !editing.card_shows());
    let edit = session.edit().unwrap();
    assert_eq!(unchecked(edit), edit.menu().len());
    let stop = AtomicBool::new(false);
    session.check_edit_menu(&stop);
    assert_eq!(unchecked(session.edit().unwrap()), 0);
    editing.draw(&session, false);
    let rows = editing.menu.model().rows().to_vec();
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
    assert!(
        editing.chosen.borrow().is_empty(),
        "the press is only queued"
    );
    settle();
    let declaration = editing
        .chosen
        .borrow_mut()
        .pop()
        .expect("the Door entry reports");
    let index = entry_index(session.edit().unwrap(), declaration).unwrap();
    assert!(send(&mut session, EditCommand::Choose(index)).contains(&EditEvent::Chosen));

    // The card: the hinge side still to point at, so no verdict yet.
    assert!(matches!(editing.draw(&session, true), Drawn::Card(_)));
    assert!(editing.card_shows() && !editing.menu_shows());
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
    let Drawn::Card(offers) = editing.draw(&session, false) else {
        panic!("the card shows");
    };
    // The mount is chosen from its list of options, or typed.
    assert!(offers.has_short_list("mount"));
    assert!(!offers.put_back(), "nothing is blamed yet");
    let lists = editing.card.model().option_lists();
    assert!(
        lists.iter().any(|(param, _, _)| param.as_str() == "mount"),
        "{:?}",
        lists
            .iter()
            .map(|(param, _, _)| param.as_str())
            .collect::<Vec<_>>()
    );
    let (_, mount) = editing
        .card
        .model()
        .entries()
        .into_iter()
        .find(|(param, _)| param.as_str() == "mount")
        .unwrap();
    mount.set_text("overlay(left: 17.8mm, right: 17.8mm)");
    mount.emit_activate();
    assert!(editing.actions.borrow().is_empty(), "only queued");
    settle();
    for action in editing.take_actions() {
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
    editing.draw(&session, false);
    let (add, cancel) = editing.card.widgets().buttons();
    assert!(!add.is_sensitive());
    let fixes = editing.card.model().fix_buttons();
    let ((problem, fix), button) = fixes.first().expect("a fix is offered");
    assert_eq!((*problem, *fix), (0, 0));
    button.emit_clicked();
    settle();
    for action in editing.take_actions() {
        send(&mut session, command_for(action).unwrap());
    }
    // The fix makes it fit: Add is enabled.
    editing.draw(&session, false);
    let chosen_door = session.edit().unwrap().chosen().unwrap();
    assert_eq!(
        ghost_verdict(chosen_door.progress()),
        Some(GhostVerdict::Fits)
    );
    assert!(add.is_sensitive());
    assert!(cancel.is_sensitive());

    // Add writes the door into the text, as one edit.
    add.emit_clicked();
    settle();
    let events: Vec<EditEvent> = editing
        .take_actions()
        .into_iter()
        .flat_map(|action| send(&mut session, command_for(action).unwrap()))
        .collect();
    assert!(events.contains(&EditEvent::Committed), "{events:?}");
    assert_eq!(editing.draw(&session, false), Drawn::Nothing);
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
    editing.draw(&session, false);
    let (_, cancel) = editing.card.widgets().buttons();
    cancel.emit_clicked();
    settle();
    let actions = editing.take_actions();
    assert_eq!(actions, [CardAction::Cancel]);
    for action in actions {
        assert!(send(&mut session, command_for(action).unwrap()).contains(&EditEvent::Cancelled));
    }
    assert_eq!(editing.draw(&session, false), Drawn::Nothing);
    assert_eq!(
        std::fs::read_to_string(folder.join("cabinet.stave")).unwrap(),
        original
    );

    // The New Project dialog reads its fields as a project that writes.
    let (nobody, _) = relm4::channel::<crate::ui::new_project::NewProjectOutput>();
    let dialog = NewProjectDialog::new(&nobody, |output| output);
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

/// The owner's case: a new project, 940 x 1780 x 580mm with one oak
/// board and the standard casework, the Case chosen for its envelope with
/// its panel stock, and its joinery's options checked.
fn case_session(name: &str) -> Session {
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
    let folder =
        std::env::temp_dir().join(format!("stave-gtk-options-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    create_project(&folder, &answers.project().unwrap()).unwrap();
    // The dialog doesn't ask how wide the boards come; without it the
    // 580mm deep case would be warned about (grammar, "stock"), and the
    // card would show the warning. Wide enough that no part is glued up.
    for entry in std::fs::read_dir(&folder).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "stave") {
            let text = std::fs::read_to_string(&path).unwrap();
            let text = text.replace("max_length: 2700mm", "max_width: 600mm, max_length: 2700mm");
            std::fs::write(&path, text).unwrap();
        }
    }
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
    session
}

/// Every label's text under a widget, one a line.
fn words_in(widget: &gtk::Widget) -> String {
    let mut words = String::new();
    let mut stack = vec![widget.clone()];
    while let Some(widget) = stack.pop() {
        if let Some(label) = widget.downcast_ref::<gtk::Label>() {
            words.push_str(&label.label());
            words.push('\n');
        }
        let mut child = widget.last_child();
        while let Some(next) = child {
            child = next.prev_sibling();
            stack.push(next);
        }
    }
    words
}

/// Shows a widget in a window of this size and waits until it is laid
/// out.
fn shown_in_window(child: &gtk::Widget, width: i32, height: i32) -> gtk::Window {
    let window = gtk::Window::builder()
        .default_width(width)
        .default_height(height)
        .decorated(false)
        .child(child)
        .build();
    window.set_size_request(width, height);
    window.present();
    let context = glib::MainContext::default();
    let started = std::time::Instant::now();
    while (!child.is_mapped() || child.height() <= 1)
        && started.elapsed() < std::time::Duration::from_secs(10)
    {
        context.iteration(false);
    }
    for _ in 0..20 {
        context.iteration(false);
    }
    assert!(child.is_mapped(), "the window never came up");
    window
}

#[test]
fn a_settings_short_list_and_its_search_choose_and_a_blamed_setting_can_be_put_back() {
    on_gtk_thread(short_list_search_and_blame_on_the_card);
}

/// The Case's joinery: its short list (the default, then "Search all
/// 20…"), the search dialog finding chigiri by its description (refused:
/// it can't be chosen) and dowel by its name (chosen with Enter), then
/// chigiri typed, blamed, and put back from the card.
fn short_list_search_and_blame_on_the_card() {
    let mut session = case_session("short");
    let editing = Launched::new();
    let Drawn::Card(offers) = editing.draw(&session, false) else {
        panic!("the card shows");
    };
    // No caption line under a setting: its type and where its value
    // comes from are its tooltip and its accessible description.
    let shown = words_in(editing.card.widget().upcast_ref());
    assert!(!shown.contains("your choice"), "{shown}");
    assert!(!shown.contains("JointKind"), "{shown}");
    let (_, joinery) = editing
        .card
        .model()
        .entries()
        .into_iter()
        .find(|(param, _)| param.as_str() == "joinery")
        .unwrap();
    // At its default: empty, the default's value dimmed in its place, in
    // full, without words that would cut it off.
    assert_eq!(joinery.text(), "");
    assert_eq!(
        joinery.placeholder_text().as_deref(),
        Some("dowel(count: 4)")
    );
    assert!(
        joinery
            .tooltip_text()
            .is_some_and(|tip| tip.contains("JointKind · the default, dowel(count: 4)")),
        "{:?}",
        joinery.tooltip_text()
    );

    // The window knows the card lists joinery's options because it drew
    // the card; it opens the list by message, and the card says so.
    assert!(offers.has_short_list("joinery"));
    assert!(!offers.has_short_list("no_such_setting"));
    editing
        .card
        .sender()
        .send(EditCardInput::OpenOptions("joinery".to_owned()))
        .unwrap();
    assert!(editing.open.borrow().is_empty(), "only queued");
    settle();
    assert_eq!(*editing.open.borrow(), [Some("joinery".to_owned())]);
    let (_, toggle, rows) = editing
        .card
        .model()
        .option_lists()
        .into_iter()
        .find(|(param, _, _)| param.as_str() == "joinery")
        .unwrap();
    assert!(toggle.is_active());
    let texts: Vec<&str> = rows.iter().map(|(text, _)| text.as_str()).collect();
    assert_eq!(texts, ["dowel(count: 4)", ""], "the default, then Search…");
    assert!(words_in(rows[0].1.upcast_ref()).contains("default"));
    assert!(words_in(rows[1].1.upcast_ref()).contains("Search all 20…"));
    rows[0].1.emit_clicked();
    rows[1].1.emit_clicked();
    settle();
    // Picking the default closed the list, and the card said so.
    assert_eq!(*editing.open.borrow(), [Some("joinery".to_owned()), None]);
    let joinery_name = parse_name("joinery").unwrap();
    assert_eq!(
        editing.take_actions(),
        [
            CardAction::UseDefault {
                param: joinery_name.clone()
            },
            CardAction::Search {
                param: joinery_name.clone()
            }
        ]
    );
    assert_eq!(
        command_for(CardAction::UseDefault {
            param: joinery_name.clone()
        }),
        Ok(EditCommand::UseDefault(joinery_name.clone()))
    );

    // The search, as the window drives it: given the session's setting,
    // which it searches with stave_app itself.
    let heard: Rc<RefCell<Vec<OptionSearchOutput>>> = Rc::default();
    let search = {
        let heard = heard.clone();
        OptionSearchComponent::builder()
            .launch(())
            .connect_receiver(move |_, output| heard.borrow_mut().push(output))
    };
    let setting = session
        .edit()
        .unwrap()
        .chosen()
        .unwrap()
        .candidate()
        .settings
        .iter()
        .find(|setting| setting.name.as_str() == "joinery")
        .unwrap()
        .clone();
    let ask = |message: OptionSearchInput| {
        search.sender().send(message).unwrap();
        settle();
    };
    let type_query = |query: &str| {
        search.widgets().entry().set_text(query);
        ask(OptionSearchInput::QueryChanged);
    };
    let selected = || search.widgets().selected(&search.model().rows());
    // Over the window's panels, as the window puts it.
    let sheet = gtk::Overlay::builder()
        .child(&gtk::Label::new(Some("the panels")))
        .build();
    sheet.add_overlay(search.widget());
    let window = shown_in_window(sheet.upcast_ref(), 900, 700);
    assert!(!search.widget().is_visible(), "closed until asked for");
    search
        .sender()
        .send(OptionSearchInput::Present {
            name: "joinery".to_owned(),
            setting: Some(Box::new(setting.clone())),
            query: String::new(),
        })
        .unwrap();
    assert!(!search.widget().is_visible(), "only queued");
    settle();
    assert!(search.widget().is_visible());
    let all = search.model().rows();
    assert_eq!(all.len(), 20, "every option, with nothing typed");
    // The arrow keys move through them.
    let first = selected().unwrap();
    ask(OptionSearchInput::Step(1));
    assert_ne!(selected().unwrap(), first);
    ask(OptionSearchInput::Step(-1));
    assert_eq!(selected().unwrap(), first);
    // A copy of another setting doesn't change what it lists.
    let mut other = setting.clone();
    other.name = parse_name("mount").unwrap();
    other.options.clear();
    ask(OptionSearchInput::Update(Box::new(other)));
    assert_eq!(search.model().rows().len(), 20);

    // "key": chigiri, by its description, refused; Enter chooses nothing.
    type_query("key");
    let found = search.model().rows();
    assert_eq!(
        found
            .iter()
            .map(|(_, text)| text.as_str())
            .collect::<Vec<_>>(),
        ["chigiri"]
    );
    let (row, _) = &found[0];
    assert!(row.has_css_class("refused"));
    let words = words_in(row.upcast_ref());
    assert!(words.contains("butterfly key"), "{words}");
    assert!(words.contains("Can't be used here"), "{words}");
    ask(OptionSearchInput::ChooseSelected);
    assert!(heard.borrow().is_empty());
    assert_eq!(search.model().setting(), Some("joinery"), "still open");

    // "dow": dowel by its name; it fits, and Enter chooses it: the search
    // says it closed, then what was chosen.
    type_query("dow");
    assert_eq!(selected().as_deref(), Some("dowel"));
    let words = words_in(search.model().rows()[0].0.upcast_ref());
    assert!(words.contains("Fits"), "{words}");
    ask(OptionSearchInput::ChooseSelected);
    assert_eq!(
        heard.borrow_mut().drain(..).collect::<Vec<_>>(),
        [
            OptionSearchOutput::Closed,
            OptionSearchOutput::Chosen {
                setting: "joinery".to_owned(),
                value: "dowel".to_owned()
            }
        ]
    );
    assert!(!search.widget().is_visible(), "choosing closes it");
    assert_eq!(search.model().setting(), None);

    // Escape in the field closes it, choosing nothing.
    ask(OptionSearchInput::Present {
        name: "joinery".to_owned(),
        setting: Some(Box::new(setting)),
        query: "dow".to_owned(),
    });
    let entry = search.widgets().entry();
    let controllers = entry.observe_controllers();
    let keys = (0..controllers.n_items())
        .filter_map(|index| controllers.item(index))
        .filter_map(|item| item.downcast::<gtk::EventControllerKey>().ok())
        .find(|keys| keys.propagation_phase() == gtk::PropagationPhase::Capture)
        .expect("the field's own keys");
    assert!(keys.emit_by_name::<bool>(
        "key-pressed",
        &[
            &glib::translate::IntoGlib::into_glib(gtk::gdk::Key::Escape),
            &0u32,
            &gtk::gdk::ModifierType::empty(),
        ],
    ));
    assert!(search.widget().is_visible(), "only queued");
    settle();
    assert!(!search.widget().is_visible());
    assert_eq!(
        heard.borrow_mut().drain(..).collect::<Vec<_>>(),
        [OptionSearchOutput::Closed]
    );
    window.destroy();

    // chigiri typed: the card puts the blame first, with the way back.
    send(
        &mut session,
        EditCommand::Set {
            name: parse_name("joinery").unwrap(),
            value: parse_value("chigiri").unwrap(),
        },
    );
    let Drawn::Card(offers) = editing.draw(&session, false) else {
        panic!("the card shows");
    };
    assert!(offers.put_back(), "the window knows the card offers it");
    let put_back = editing
        .card
        .model()
        .put_back_button()
        .expect("the card offers the way back");
    assert_eq!(
        put_back.label().map(|label| label.to_string()),
        Some("Put joinery back to dowel(count: 4), the default".to_owned())
    );
    put_back.emit_clicked();
    settle();
    for action in editing.take_actions() {
        send(&mut session, command_for(action).unwrap());
    }
    let chosen = session.edit().unwrap().chosen().unwrap();
    assert!(matches!(
        chosen.progress(),
        Progress::Tried(Verdict::Fits(_))
    ));
    let Drawn::Card(offers) = editing.draw(&session, false) else {
        panic!("the card shows");
    };
    assert!(!offers.put_back());
    assert!(editing.card.model().put_back_button().is_none());
}

/// The room the Edit panel has in the docked layout's default arrangement
/// at 1440 x 900: its frame, below Parts on the left, is 300 wide and
/// leaves its content 417 high under its tabs (measured from the window).
const PANEL_ROOM: (i32, i32) = (300, 417);

#[test]
fn the_case_card_shows_whole_in_the_edit_panel_at_the_default_size() {
    on_gtk_thread(case_card_fits);
}

fn case_card_fits() {
    let session = case_session("fits");
    let editing = Launched::new();
    let card = editing.card.widget().clone();
    let panel = crate::ui::edit_panel::EditPanel::new(&[card.upcast_ref()]);
    card.set_visible(false);
    assert!(!panel.update());
    assert!(panel.is_empty(), "nothing in progress: how to start");
    // The repeat controls as the window puts them: folded.
    let fold = item_tools::RepeatFoldComponent::builder()
        .launch(item_tools::RepeatFoldInit {
            least: 1,
            apart: Some(item_tools::RepeatFoldSlots {
                button: editing.slots.repeat_button_slot.clone(),
                row: editing.slots.repeat_slot.clone(),
            }),
        })
        .detach();
    let mut shown = item_tools::RepeatFoldShown::new(1);
    {
        let chosen = session.edit().unwrap().chosen().unwrap();
        let _ = fold
            .sender()
            .send(shown.show(stave_app::item_offer::candidate_repeat_fold(
                chosen.candidate(),
                chosen.repeat(),
                shown.asked(),
            )));
    }
    // Drawing settles the main loop, so the fold has shown too.
    editing.draw(&session, false);
    assert!(
        fold.widgets().unfold_button().is_visible(),
        "folded: Repeat… sits beside the buttons"
    );
    assert!(panel.update());
    assert!(!panel.is_empty());
    let window = shown_in_window(panel.root.upcast_ref(), PANEL_ROOM.0, PANEL_ROOM.1);
    // Nothing asks for more width than the panel has.
    let (least, _, _, _) = card.measure(gtk::Orientation::Horizontal, -1);
    assert!(
        least <= PANEL_ROOM.0,
        "the card needs {least} pixels across"
    );
    // Its body needs no scrolling: everything shows at once, Add and
    // Cancel below it.
    let scrolled = editing.card.widgets().scrolled();
    let adjustment = scrolled.vadjustment();
    // What it needs: all of its body, and its foot (Repeat…, Cancel, Add).
    let foot = card.height() - scrolled.height();
    let body = scrolled.child().map_or(0, |body| {
        body.measure(gtk::Orientation::Vertical, PANEL_ROOM.0).1
    });
    eprintln!(
        "the Case card needs {} of {} pixels at {} wide (body {body}, foot {foot})",
        body + foot,
        PANEL_ROOM.1,
        PANEL_ROOM.0,
    );
    assert!(body + foot <= PANEL_ROOM.1);
    assert_eq!(card.height(), PANEL_ROOM.1);
    assert!(
        adjustment.upper() <= adjustment.page_size() + 0.5,
        "the body scrolls: {} of {}",
        adjustment.upper(),
        adjustment.page_size()
    );
    let (add, cancel) = editing.card.widgets().buttons();
    assert!(add.is_mapped() && cancel.is_mapped());
    // "Repeat…" pressed: the count and the direction open above the
    // buttons, still within the panel's width.
    assert!(shown.press());
    let _ = fold
        .sender()
        .send(shown.show(stave_app::item_offer::RepeatFold::Open));
    settle();
    assert!(fold.widgets().row().is_visible());
    let (least, _, _, _) = card.measure(gtk::Orientation::Horizontal, -1);
    assert!(
        least <= PANEL_ROOM.0,
        "open, the card needs {least} pixels across"
    );
    window.destroy();
}

#[test]
fn repeat_is_folded_until_asked_for() {
    on_gtk_thread(repeat_fold_row);
}

fn repeat_fold_row() {
    use stave_app::item_offer::RepeatFold;
    let unfolded = Rc::new(std::cell::Cell::new(0));
    let heard = Rc::clone(&unfolded);
    let fold = item_tools::RepeatFoldComponent::builder()
        .launch(item_tools::RepeatFoldInit {
            least: 1,
            apart: None,
        })
        .connect_receiver(move |_, output| {
            if output == item_tools::RepeatFoldOutput::Unfold {
                heard.set(heard.get() + 1);
            }
        });
    let mut shown = item_tools::RepeatFoldShown::new(1);
    let show = |shown: &mut item_tools::RepeatFoldShown, rule| {
        let _ = fold.sender().send(shown.show(rule));
        settle();
    };
    let unfold = fold.widgets().unfold_button().clone();
    let row = fold.widgets().row().clone();
    show(&mut shown, RepeatFold::Hidden);
    assert!(!fold.widget().is_visible());
    assert!(!shown.press(), "hidden: nothing to press");
    show(&mut shown, RepeatFold::Folded);
    assert!(unfold.is_visible() && !row.is_visible());
    assert!(!shown.asked());
    // People press it: the owner hears it and marks it asked.
    unfold.emit_clicked();
    settle();
    assert_eq!(unfolded.get(), 1);
    shown.unfolded();
    assert!(shown.asked());
    // A script presses it: marked at once (the owner then does what it
    // does for a click).
    shown.forget();
    assert!(shown.press());
    assert!(shown.asked());
    show(&mut shown, RepeatFold::Open);
    assert!(!unfold.is_visible() && row.is_visible());
    assert!(shown.is_open());
    shown.forget();
    assert!(!shown.asked());
}
