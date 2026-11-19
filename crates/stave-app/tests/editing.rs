//! The edit in progress (`stave_app::editing`, `docs/editing.md`) on a
//! copy of `examples/clothing_cabinet_assemblies` made in a temporary
//! folder: the menu for a space with each entry's applicability, choosing
//! and filling in a shelf, committing it through the session, and a
//! cancel that leaves every file as it was. The example is read, never
//! written: each test works on its own copy.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use stave_app::edit::{Applicability, Candidate, FacingPair, PartFace, Picked, Verdict};
use stave_app::editing::{
    Edit, EditCommand, EditEvent, Progress, Rejection, entry_matches, parse_name, parse_value,
};
use stave_app::session::{Command, Event, Session};
use stave_lang::placement::Face;
use stave_model::nonempty::NonEmpty;
use stave_modules::DiskFiles;

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A fresh temporary folder holding a copy of the assemblies cabinet
/// (its `hardware` module included). Returns the project folder.
fn copy_of_the_cabinet(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("stave-editing-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let project = root.join("cabinet");
    let mut folders = vec![(
        workspace().join("examples/clothing_cabinet_assemblies"),
        project.clone(),
    )];
    while let Some((from, to)) = folders.pop() {
        std::fs::create_dir_all(&to).unwrap();
        for entry in std::fs::read_dir(&from).unwrap() {
            let path = entry.unwrap().path();
            let target = to.join(path.file_name().unwrap());
            if path.is_dir() {
                folders.push((path, target));
            } else {
                std::fs::copy(&path, &target).unwrap();
            }
        }
    }
    project
}

/// Every file under `folder` with its bytes.
fn snapshot(folder: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut folders = vec![folder.to_path_buf()];
    while let Some(folder) = folders.pop() {
        for entry in std::fs::read_dir(&folder).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                folders.push(path);
            } else {
                out.insert(path.clone(), std::fs::read(&path).unwrap());
            }
        }
    }
    out
}

fn files() -> Arc<DiskFiles> {
    Arc::new(DiskFiles)
}

fn region(edit_or_opened: &stave_app::Opened, name: &str) -> stave_model::ids::RegionId {
    let model = edit_or_opened.model.as_ref().unwrap();
    model
        .regions
        .iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("a region called {name}"))
        .id
}

fn part(opened: &stave_app::Opened, name: &str) -> stave_model::ids::PartId {
    let model = opened.model.as_ref().unwrap();
    model
        .parts
        .iter()
        .find(|p| p.name == name)
        .unwrap_or_else(|| panic!("a part called {name}"))
        .id
}

fn entry<'a>(menu: &'a [Candidate], label: &str) -> (usize, &'a Candidate) {
    menu.iter()
        .enumerate()
        .find(|(_, c)| entry_matches(c, label))
        .unwrap_or_else(|| {
            let labels: Vec<&str> = menu.iter().map(|c| c.entry.label.as_str()).collect();
            panic!("{label} is in the menu {labels:?}")
        })
}

fn opened_for(folder: &Path) -> stave_app::Opened {
    stave_app::open(folder).unwrap()
}

#[test]
fn the_space_above_the_divider_lists_the_shelf_with_its_applicability() {
    let folder = copy_of_the_cabinet("menu");
    let base = opened_for(&folder);
    let above = region(&base, "divider.above");
    let mut edit = Edit::start(&folder, files(), Picked::Region(above)).unwrap();
    assert!(
        edit.menu()
            .iter()
            .all(|c| c.applicability == Applicability::NotChecked)
    );
    let never = AtomicBool::new(false);
    assert!(edit.check_menu(&never).unwrap());

    // The shelf's sides are a place param still to point at: it needs a
    // choice, which is not a refusal.
    let (_, shelf) = entry(edit.menu(), "Shelf");
    let Applicability::NeedsChoice(names) = &shelf.applicability else {
        panic!("the shelf needs a choice: {:?}", shelf.applicability);
    };
    assert_eq!(
        names.iter().map(|n| n.as_str()).collect::<Vec<_>>(),
        ["sides"]
    );
    // Others are listed too, each with an answer.
    assert!(edit.menu().len() > 3, "{:#?}", edit.menu().len());
    assert!(
        edit.menu()
            .iter()
            .all(|c| c.applicability != Applicability::NotChecked)
    );
    // Checking left the base as it was.
    assert_eq!(
        edit.base().diagnostics.len(),
        opened_for(&folder).diagnostics.len()
    );
}

#[test]
fn a_candidate_that_cannot_fit_is_refused_with_the_checker_s_message() {
    let folder = copy_of_the_cabinet("refused");
    let base = opened_for(&folder);
    // drawer[top] already fills this bay: another drawer can't go in.
    let bay = region(&base, "drawer_bay[top]");
    let mut edit = Edit::start(&folder, files(), Picked::Region(bay)).unwrap();
    edit.check_menu(&AtomicBool::new(false)).unwrap();
    let (_, drawer) = entry(edit.menu(), "Drawer");
    let Applicability::Greyed(problems) = &drawer.applicability else {
        panic!("the drawer is refused: {:?}", drawer.applicability);
    };
    assert!(
        problems
            .iter()
            .all(|d| d.is_error() && !d.message.is_empty()),
        "{problems:#?}"
    );
    // Greyed entries come after the rest of their group.
    let group: Vec<&Candidate> = edit
        .menu()
        .iter()
        .filter(|c| c.entry.group == drawer.entry.group)
        .collect();
    let first_greyed = group
        .iter()
        .position(|c| matches!(c.applicability, Applicability::Greyed(_)))
        .unwrap();
    assert!(
        group[first_greyed..]
            .iter()
            .all(|c| matches!(c.applicability, Applicability::Greyed(_)))
    );
}

#[test]
fn checking_the_menu_can_stop_part_way() {
    let folder = copy_of_the_cabinet("stop");
    let base = opened_for(&folder);
    let above = region(&base, "divider.above");
    let mut edit = Edit::start(&folder, files(), Picked::Region(above)).unwrap();
    let stop = AtomicBool::new(true);
    assert!(edit.check_menu(&stop).is_err());
    // Entries that need a choice are answered without a check; the rest
    // wait.
    assert!(
        edit.menu()
            .iter()
            .any(|c| c.applicability == Applicability::NotChecked)
    );
}

#[test]
fn a_door_stile_s_face_offers_a_knob() {
    let folder = copy_of_the_cabinet("knob");
    let base = opened_for(&folder);
    let stile = part(&base, "door_l.stile[hinge]");
    let edit = Edit::start(
        &folder,
        files(),
        Picked::Face(PartFace {
            part: stile,
            face: Face::Front,
        }),
    )
    .unwrap();
    let (_, knob) = entry(edit.menu(), "hw.brass_knob_26");
    assert_eq!(knob.place[0].name.as_str(), "piece");
    let (_, std_knob) = entry(edit.menu(), "knob");
    assert!(matches!(
        std_knob.declaration,
        stave_app::edit::Declaration::Kind(_)
    ));
}

#[test]
fn choosing_the_shelf_and_adding_it_gives_a_design_with_one_more_shelf() {
    let folder = copy_of_the_cabinet("commit");
    let base = opened_for(&folder);
    let shelves_before = base
        .model
        .as_ref()
        .unwrap()
        .assemblies
        .iter()
        .filter(|a| a.name.starts_with("shelf"))
        .count();
    let above = region(&base, "divider.above");
    let (side_l, side_r) = (part(&base, "side_l"), part(&base, "side_r"));

    let mut session = Session::new(folder.clone());
    let events = session.apply(Command::Edit(EditCommand::Start(Picked::Region(above))));
    assert!(
        events.contains(&Event::Edit(EditEvent::Started)),
        "{events:?}"
    );
    let (index, _) = entry(session.edit().unwrap().menu(), "Shelf");
    let events = session.apply(Command::Edit(EditCommand::Choose(index)));
    assert!(events.contains(&Event::Edit(EditEvent::Chosen)));
    let chosen = session.edit().unwrap().chosen().unwrap();
    assert!(matches!(chosen.progress(), Progress::NeedsChoice(_)));

    // The card asks for the sides; pointing at the two sides binds them.
    let events = session.apply(Command::Edit(EditCommand::Bind(Picked::Group(
        NonEmpty::new(vec![side_l, side_r]).unwrap(),
    ))));
    assert!(
        events.contains(&Event::Edit(EditEvent::PlaceBound)),
        "{events:?}"
    );
    assert!(events.contains(&Event::Edit(EditEvent::VerdictChanged)));
    let chosen = session.edit().unwrap().chosen().unwrap();
    let Progress::Tried(Verdict::Fits(_)) = chosen.progress() else {
        panic!(
            "the shelf fits: {:#?}\n{}",
            chosen.progress(),
            chosen.patch().unwrap_or_default()
        );
    };
    assert_eq!(chosen.item_name().unwrap().as_str(), "shelf_2");

    // A setting and a name typed in the card.
    let events = session.apply(Command::Edit(EditCommand::Set {
        name: parse_name("headroom").unwrap(),
        value: parse_value("400mm").unwrap(),
    }));
    assert!(events.contains(&Event::Edit(EditEvent::SettingChanged)));
    let events = session.apply(Command::Edit(EditCommand::Rename(
        parse_name("shelf").unwrap(),
    )));
    assert!(
        events.contains(&Event::Edit(EditEvent::Rejected(
            Rejection::NameUnavailable("shelf".to_owned())
        ))),
        "{events:?}"
    );
    let events = session.apply(Command::Edit(EditCommand::Rename(
        parse_name("sweater_shelf").unwrap(),
    )));
    assert!(events.contains(&Event::Edit(EditEvent::Renamed)));
    let chosen = session.edit().unwrap().chosen().unwrap();
    assert!(
        matches!(chosen.progress(), Progress::Tried(Verdict::Fits(_))),
        "{:#?}",
        chosen.progress()
    );
    let patch = chosen.patch().unwrap().to_owned();
    assert!(
        patch.contains("add assembly sweater_shelf : casework.Shelf {"),
        "{patch}"
    );
    assert!(patch.contains("headroom:    400mm"), "{patch}");

    let events = session.apply(Command::Edit(EditCommand::Add));
    assert!(
        events.contains(&Event::Edit(EditEvent::Committed)),
        "{events:?}"
    );
    assert!(events.contains(&Event::ResultChanged));
    assert!(session.edit().is_none());
    // The session shows the committed design.
    let shown = session.shown().unwrap();
    assert!(
        shown
            .design()
            .unwrap()
            .parts()
            .iter()
            .any(|p| p.name == "sweater_shelf.board")
    );

    // Opened again from disk, it checks clean with one more shelf.
    let reopened = opened_for(&folder);
    let errors: Vec<_> = reopened
        .diagnostics
        .iter()
        .filter(|d| d.is_error())
        .collect();
    assert!(errors.is_empty(), "{errors:#?}");
    let shelves_after = reopened
        .model
        .as_ref()
        .unwrap()
        .assemblies
        .iter()
        .filter(|a| a.name.starts_with("shelf") || a.name == "sweater_shelf")
        .count();
    assert_eq!(shelves_after, shelves_before + 1);
    let carcass = std::fs::read_to_string(folder.join("carcass.stave")).unwrap();
    assert!(
        carcass.contains("assembly sweater_shelf : casework.Shelf {"),
        "{carcass}"
    );
}

#[test]
fn cancel_leaves_every_file_byte_for_byte() {
    let folder = copy_of_the_cabinet("cancel");
    let before = snapshot(&folder);
    let base = opened_for(&folder);
    let above = region(&base, "divider.above");
    let pair = FacingPair::new(part(&base, "side_l"), part(&base, "side_r")).unwrap();

    let mut session = Session::new(folder.clone());
    session.apply(Command::Edit(EditCommand::Start(Picked::Region(above))));
    session.apply(Command::Edit(EditCommand::CheckMenu));
    let (index, _) = entry(session.edit().unwrap().menu(), "Shelf");
    session.apply(Command::Edit(EditCommand::Choose(index)));
    session.apply(Command::Edit(EditCommand::Bind(Picked::Between(pair))));
    let events = session.apply(Command::Edit(EditCommand::Cancel));
    assert!(events.contains(&Event::Edit(EditEvent::Cancelled)));
    assert!(session.edit().is_none());
    assert_eq!(snapshot(&folder), before);
    // Nothing to add once cancelled.
    let events = session.apply(Command::Edit(EditCommand::Add));
    assert!(events.contains(&Event::Edit(EditEvent::Rejected(Rejection::NoEdit))));
    assert_eq!(snapshot(&folder), before);
}

#[test]
fn add_never_overwrites_a_change_saved_since_the_edit_began() {
    let folder = copy_of_the_cabinet("stale");
    let base = opened_for(&folder);
    let above = region(&base, "divider.above");
    let (side_l, side_r) = (part(&base, "side_l"), part(&base, "side_r"));
    let carcass = folder.join("carcass.stave");

    let mut session = Session::new(folder.clone());
    session.apply(Command::Edit(EditCommand::Start(Picked::Region(above))));
    let (index, _) = entry(session.edit().unwrap().menu(), "Shelf");
    session.apply(Command::Edit(EditCommand::Choose(index)));
    session.apply(Command::Edit(EditCommand::Bind(Picked::Group(
        NonEmpty::new(vec![side_l, side_r]).unwrap(),
    ))));
    assert!(matches!(
        session.edit().unwrap().chosen().unwrap().progress(),
        Progress::Tried(Verdict::Fits(_))
    ));

    // Someone saves a change to the file the patch edits.
    let saved = format!(
        "{}\n// a note saved in the editor\n",
        std::fs::read_to_string(&carcass).unwrap()
    );
    std::fs::write(&carcass, &saved).unwrap();
    let before = snapshot(&folder);

    let events = session.apply(Command::Edit(EditCommand::Add));
    assert!(
        events.iter().any(|e| matches!(
            e,
            Event::Edit(EditEvent::Rejected(Rejection::CantWrite(message)))
                if message.contains("changed since the edit began")
        )),
        "{events:?}"
    );
    assert!(!events.contains(&Event::Edit(EditEvent::Committed)));
    // The saved change is kept and nothing else was written.
    assert_eq!(std::fs::read_to_string(&carcass).unwrap(), saved);
    assert_eq!(snapshot(&folder), before);
    // The edit is still open, so it can be cancelled or started again.
    assert!(session.edit().is_some());
}

#[test]
fn add_writes_through_open_buffers() {
    let folder = copy_of_the_cabinet("buffers");
    let base = opened_for(&folder);
    let above = region(&base, "divider.above");
    let (side_l, side_r) = (part(&base, "side_l"), part(&base, "side_r"));
    let carcass = folder.join("carcass.stave");
    let on_disk = std::fs::read_to_string(&carcass).unwrap();

    let buffers = std::rc::Rc::new(std::cell::RefCell::new(stave_ide::Buffers::new()));
    let mut session = Session::new(folder.clone());
    session.set_edit_writer(Box::new(buffers.clone()));
    session.apply(Command::Edit(EditCommand::Start(Picked::Region(above))));
    let (index, _) = entry(session.edit().unwrap().menu(), "Shelf");
    session.apply(Command::Edit(EditCommand::Choose(index)));
    session.apply(Command::Edit(EditCommand::Bind(Picked::Group(
        NonEmpty::new(vec![side_l, side_r]).unwrap(),
    ))));
    let events = session.apply(Command::Edit(EditCommand::Add));
    assert!(
        events.contains(&Event::Edit(EditEvent::Committed)),
        "{events:?}"
    );

    // The buffer holds the change, unsaved; the disk is as it was.
    let document = stave_ide::Document::File(carcass.clone());
    let buffers = buffers.borrow();
    assert!(buffers.is_dirty(&document));
    assert!(
        buffers
            .text(&document)
            .unwrap()
            .contains("assembly shelf_2 : casework.Shelf {")
    );
    assert_eq!(std::fs::read_to_string(&carcass).unwrap(), on_disk);
}

#[test]
fn a_design_that_does_not_solve_cannot_be_edited_by_pointing() {
    let folder = copy_of_the_cabinet("unsolved");
    let base = opened_for(&folder);
    let above = region(&base, "divider.above");
    // A requirement that fails stops the solver writing a design.
    let cabinet = folder.join("cabinet.stave");
    let text = std::fs::read_to_string(&cabinet)
        .unwrap()
        .replace(">= 540mm \"room", ">= 940mm \"room");
    std::fs::write(&cabinet, text).unwrap();
    let mut session = Session::new(folder.clone());
    let events = session.apply(Command::Edit(EditCommand::Start(Picked::Region(above))));
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "a test picks out the variant it checks, and any other is a failure or not what it counts"
    )]
    let rejected = events.iter().find_map(|event| match event {
        Event::Edit(EditEvent::Rejected(Rejection::CantStart(message))) => Some(message.clone()),
        _ => None,
    });
    let message = rejected.unwrap_or_else(|| panic!("refused: {events:?}"));
    assert!(message.contains("until it places"), "{message}");
}
