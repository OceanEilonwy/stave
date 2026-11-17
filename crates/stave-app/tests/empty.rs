//! The view's empty states: still checking, errors, a folder that can't
//! be opened, and a library folder that lists what it defines.

use std::path::{Path, PathBuf};

use stave_app::check;
use stave_app::empty::{self, LibraryGroup, Reason};
use stave_app::session::{Command, Event, Session};

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn session_for(folder: PathBuf) -> Session {
    let mut session = Session::new(folder.clone());
    let events = session.apply(Command::Check);
    let ticket = events
        .iter()
        .find_map(|event| match event {
            Event::CheckStarted(ticket) => Some(*ticket),
            _ => None,
        })
        .expect("a check starts");
    session.finish_check(ticket, check::run(&folder));
    session
}

#[test]
fn a_library_folder_says_so_instead_of_a_blank_view() {
    let session = session_for(workspace().join("examples/japanese_joinery"));
    let state = empty::of(&session).expect("an empty state");
    assert_eq!(state.reason, Reason::Library);
    assert_eq!(state.title, "Nothing to show");
    assert!(state.description.contains("library"));
    assert!(state.offers_open);
}

#[test]
fn a_library_folder_lists_what_it_defines_and_says_how_to_use_it() {
    let session = session_for(workspace().join("examples/japanese_joinery"));
    let state = empty::of(&session).expect("an empty state");
    let library = state.library.as_ref().expect("the library's contents");
    assert_eq!(library.title, "Japanese joinery samples");
    assert_eq!(library.import_name, "japanese_joinery");
    let groups: Vec<(LibraryGroup, Vec<&str>)> = library
        .groups
        .iter()
        .map(|(group, items)| (*group, items.iter().map(|i| i.name.as_str()).collect()))
        .collect();
    assert_eq!(
        groups,
        vec![
            (
                LibraryGroup::Joints,
                vec!["ari_tsugi", "chidori", "kanawa_tsugi"]
            ),
            (LibraryGroup::Tools, vec!["sumitsubo"]),
            (LibraryGroup::Recipes, vec!["outline_with_sumitsubo"]),
        ]
    );
    let sumitsubo = &library.groups[1].1[0];
    assert_eq!(
        sumitsubo.summary.as_deref(),
        Some("An ink line: snaps a long, straight line along a beam.")
    );
    let location = sumitsubo.location.as_ref().expect("where it is written");
    assert!(
        location.path.ends_with("library.stave"),
        "{}",
        location.path
    );
    assert_eq!(location.line.get(), 18);
    // The sidebar and the inspector say what is true for a library, not
    // "Nothing selected".
    assert_eq!(state.sidebar(), "A library has no parts. It defines:");
    let note = state.inspector();
    assert_eq!(note.title, "A library folder");
    assert!(note.description.contains("no parts of its own"));
    assert_eq!(
        note.code.as_deref(),
        Some("import \"../japanese_joinery\" as japanese_joinery")
    );
    let after = note.after.unwrap_or_default();
    assert!(after.contains("japanese_joinery.ari_tsugi"), "{after}");
    // A project has no library.
    let broken = session_for(workspace().join("tests/broken/unknown_name"));
    let state = empty::of(&broken).expect("an empty state");
    assert_eq!(state.library, None);
    assert_eq!(state.inspector().title, "Nothing to inspect yet");
}

#[test]
fn the_empty_state_follows_the_session() {
    let unchecked = Session::new(workspace().join("examples/stool"));
    assert_eq!(
        empty::of(&unchecked).map(|s| s.reason),
        Some(Reason::Checking)
    );
    assert_eq!(
        empty::of(&session_for(workspace().join("examples/stool"))),
        None
    );
    let broken = session_for(workspace().join("tests/broken/unknown_name"));
    assert_eq!(empty::of(&broken).map(|s| s.reason), Some(Reason::Errors));
    let missing = session_for(workspace().join("tests/no_such_folder"));
    assert_eq!(
        empty::of(&missing).map(|s| s.reason),
        Some(Reason::CantOpen)
    );
}
