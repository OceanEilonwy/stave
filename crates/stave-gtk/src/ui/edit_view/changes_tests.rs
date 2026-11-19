//! The changes of the design on the real session, as the window draws
//! them: what the change's card says for a removal that something still
//! uses (and once that goes too) and for a rename, and which item a
//! selection changes. The door fixture (`fixtures/door_hits_wall`) is
//! copied to a folder of its own.

use std::path::{Path, PathBuf};

use stave_app::editing::{EditCommand, parse_name};
use stave_app::session::{Command, Event, Selection, Session};

use super::changes::{Commit, change_words};
use super::item_tools::{piece_count, selected_item};

fn fixture(name: &str) -> PathBuf {
    let folder =
        std::env::temp_dir().join(format!("stave-gtk-changes-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    let source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/door_hits_wall/cabinet.stave");
    std::fs::copy(source, folder.join("cabinet.stave")).unwrap();
    folder.canonicalize().unwrap()
}

fn checked(folder: &Path) -> Session {
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

#[test]
fn a_removal_names_what_goes_with_it_before_anything_goes() {
    let folder = fixture("remove");
    let mut session = checked(&folder);
    session.apply(Command::Edit(EditCommand::Remove(
        parse_name("side_l").unwrap(),
    )));
    let words = change_words(session.change().expect("a removal"), false);
    assert_eq!(words.eyebrow, "Change · remove");
    assert!(words.waiting_on_dependants, "{words:?}");
    assert!(words.dependants.contains(&"side_r".to_owned()), "{words:?}");
    // Apply keeps a destructive look for a removal.
    assert_eq!(words.commit, Commit::Remove);
    assert_eq!(words.item.as_deref(), Some("side_l"));
    assert!(!words.can_add, "nothing is removed before the user says");
    assert!(words.verdict.is_none());

    session.apply(Command::Edit(EditCommand::IncludeDependants));
    let words = change_words(session.change().expect("a removal"), false);
    assert!(!words.waiting_on_dependants);
    assert!(
        words
            .lines
            .iter()
            .any(|line| line.starts_with("Also removes: ") && line.contains("side_r")),
        "{words:?}"
    );
    // Cancel leaves the text as it was.
    let before = std::fs::read_to_string(folder.join("cabinet.stave")).unwrap();
    session.apply(Command::Edit(EditCommand::Cancel));
    assert!(session.change().is_none());
    assert_eq!(
        std::fs::read_to_string(folder.join("cabinet.stave")).unwrap(),
        before
    );
}

#[test]
fn a_rename_says_what_it_rewrites_and_fits() {
    let folder = fixture("rename");
    let mut session = checked(&folder);
    session.apply(Command::Edit(EditCommand::RenameItem {
        item: parse_name("wall").unwrap(),
        to: parse_name("back_wall").unwrap(),
    }));
    let words = change_words(session.change().expect("a rename"), false);
    assert_eq!(words.eyebrow, "Change · rename");
    assert_eq!(words.commit, Commit::Write);
    assert!(
        words
            .lines
            .iter()
            .any(|line| line == "wall becomes back_wall everywhere it is used"),
        "{words:?}"
    );
    assert!(words.can_add, "{words:?}");
    // While a command waits to try it again, it is checking, and can't be
    // added.
    assert!(!change_words(session.change().unwrap(), true).can_add);
}

#[test]
fn a_selection_changes_the_item_it_belongs_to() {
    let folder = fixture("item");
    let session = checked(&folder);
    let (design, _) = session.drawable().expect("the fixture places");
    let side_r = design.part_named("side_r").expect("side_r").id;
    assert_eq!(
        selected_item(design, &Selection::Part(side_r)).as_deref(),
        Some("side_r")
    );
    assert_eq!(piece_count(design, "side_r"), 1);
}
