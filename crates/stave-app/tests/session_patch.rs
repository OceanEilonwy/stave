//! A patch given whole, as an AI app's suggestion is shown in the window
//! (`docs/editing.md`, "Changes of the design"): tried on a scratch copy
//! like any change, its ghost the design it would make, written only when
//! the user adds it.

use std::path::{Path, PathBuf};

use stave_app::change::ChangeRequest;
use stave_app::edit::Verdict;
use stave_app::editing::{EditCommand, EditEvent, Progress, Rejection};
use stave_app::session::{Command, Event, Session};

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A copy of an example in a fresh temporary folder: the project folder.
fn copy_of(example: &str, name: &str) -> PathBuf {
    let root =
        std::env::temp_dir().join(format!("stave-session-patch-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let project = root.join("project");
    std::fs::create_dir_all(&project).unwrap();
    for entry in std::fs::read_dir(workspace().join("examples").join(example)).unwrap() {
        let path = entry.unwrap().path();
        if path.is_file() {
            std::fs::copy(&path, project.join(path.file_name().unwrap())).unwrap();
        }
    }
    project
}

#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "a test picks out the variant it checks, and any other is not what it counts"
)]
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

const WIDER: &str = "stave 1\n\npatch \"Make the chest wider\" {\n  set overall_width = 900mm\n}\n";

#[test]
fn a_patch_shows_as_a_change_with_a_ghost_and_writes_nothing() {
    let folder = copy_of("chest_of_drawers", "ghost");
    let chest = folder.join("chest.stave");
    let before = std::fs::read_to_string(&chest).unwrap();
    let mut session = Session::new(folder.clone());
    let events = send(&mut session, EditCommand::ShowPatch(WIDER.to_owned()));
    assert!(events.contains(&EditEvent::ChangeStarted), "{events:?}");
    let change = session.change().expect("a change in progress");
    assert!(matches!(change.request(), ChangeRequest::Patch { .. }));
    assert_eq!(change.summary(), "Make the chest wider");
    assert!(
        matches!(change.progress(), Progress::Tried(Verdict::Fits(_))),
        "{:#?}",
        change.progress()
    );
    assert!(change.ghost().is_some());
    assert_eq!(std::fs::read_to_string(&chest).unwrap(), before);
    // Cancelled, it leaves the text as it was.
    send(&mut session, EditCommand::Cancel);
    assert!(session.change().is_none());
    assert_eq!(std::fs::read_to_string(&chest).unwrap(), before);
}

#[test]
fn a_patch_added_is_written() {
    let folder = copy_of("chest_of_drawers", "add");
    let chest = folder.join("chest.stave");
    let mut session = Session::new(folder.clone());
    send(&mut session, EditCommand::ShowPatch(WIDER.to_owned()));
    let events = send(&mut session, EditCommand::Add);
    assert!(events.contains(&EditEvent::Committed), "{events:?}");
    let after = std::fs::read_to_string(&chest).unwrap();
    assert!(after.contains("overall_width  = 900mm") || after.contains("overall_width = 900mm"));
}

#[test]
fn a_patch_that_does_not_read_cannot_start() {
    let folder = copy_of("chest_of_drawers", "unreadable");
    let mut session = Session::new(folder);
    let events = send(
        &mut session,
        EditCommand::ShowPatch("stave 1\n\npatch \"Oops\" { set = }\n".to_owned()),
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, EditEvent::Rejected(Rejection::CantStart(_)))),
        "{events:?}"
    );
    assert!(session.change().is_none());
}

#[test]
fn a_patch_that_does_not_fit_is_refused_and_writes_nothing() {
    let folder = copy_of("chest_of_drawers", "refused");
    let chest = folder.join("chest.stave");
    let before = std::fs::read_to_string(&chest).unwrap();
    let mut session = Session::new(folder);
    let patch = "stave 1\n\npatch \"Too narrow\" {\n  set overall_width = 10mm\n}\n";
    send(&mut session, EditCommand::ShowPatch(patch.to_owned()));
    let change = session.change().expect("a change in progress");
    assert!(
        matches!(change.progress(), Progress::Tried(Verdict::Refused(_))),
        "{:#?}",
        change.progress()
    );
    let events = send(&mut session, EditCommand::Add);
    assert!(
        events.contains(&EditEvent::Rejected(Rejection::DoesNotFit)),
        "{events:?}"
    );
    assert_eq!(std::fs::read_to_string(&chest).unwrap(), before);
}
