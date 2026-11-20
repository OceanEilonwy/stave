//! Steps that run while someone else moves main: catching up decides and
//! writes under the lock, a stale suggestion or resolve is refused, a save
//! with nothing new records nothing, a clash never reaches the git branch,
//! and two starts at once make one repository.

mod common;

use std::collections::BTreeSet;

use common::{
    DRESSER, agent, dresser_folder, git_dresser_folder, git_output, other_agent, path, text, user,
    with,
};
use stave_history::{CatchUp, History, HistoryError, UndoScope, Written, merge_text};

const WIDER: &str = "width = 1000mm\nheight = 1200mm\ndepth = 450mm\n";
const NARROWER: &str = "width = 800mm\nheight = 1200mm\ndepth = 450mm\n";

/// A started project with its first save, and the agent's change applied
/// to main behind the window's back; returns the window and its files.
fn behind_main() -> (tempfile::TempDir, History, stave_history::ProjectFiles) {
    let folder = dresser_folder();
    let mut window = History::start(folder.path()).expect("start");
    let files = window.read_folder().expect("read folder");
    window.save(&files, &user(), "First cut").expect("save");
    let mut agent_history = History::open(folder.path()).expect("agent opens");
    let base = agent_history.main().expect("main");
    let wider = with(&files, "dresser.stave", WIDER);
    let id =
        common::suggest(&mut agent_history, &agent(), None, &wider, "p", "Widen").expect("suggest");
    agent_history
        .apply(&id, &agent(), &base, &wider)
        .expect("apply");
    (folder, window, files)
}

#[test]
fn catching_up_waits_for_an_unsaved_file_main_changed() {
    let (_folder, mut window, files) = behind_main();
    let unsaved: BTreeSet<_> = [path("dresser.stave")].into();
    let mut asked_to_write = false;
    let caught_up = window
        .catch_up(&files, &unsaved, |_| {
            asked_to_write = true;
            Ok::<_, HistoryError>(Written::All)
        })
        .expect("catch up");
    assert_eq!(
        caught_up,
        CatchUp::Waiting {
            files: vec![path("dresser.stave")]
        }
    );
    assert!(!asked_to_write);
    // Still behind: with nothing unsaved it writes main's change.
    let CatchUp::Write { files: written, .. } =
        common::catch_up(&mut window, &files).expect("catch up")
    else {
        panic!("main moved");
    };
    assert_eq!(text(&written, "dresser.stave"), WIDER);
}

#[test]
fn a_failed_or_held_back_write_leaves_the_window_behind() {
    let (_folder, mut window, files) = behind_main();
    let error = window
        .catch_up(&files, &BTreeSet::new(), |_| {
            Err::<Written, _>(HistoryError::NothingToUndo)
        })
        .expect_err("the write failed");
    assert!(matches!(error, HistoryError::NothingToUndo), "{error:?}");
    let held = window
        .catch_up(&files, &BTreeSet::new(), |_| {
            Ok::<_, HistoryError>(Written::HeldBack {
                files: vec![path("dresser.stave")],
            })
        })
        .expect("catch up");
    assert_eq!(
        held,
        CatchUp::Waiting {
            files: vec![path("dresser.stave")]
        }
    );
    // The window is still behind, so the next catch-up writes again.
    assert!(matches!(
        common::catch_up(&mut window, &files).expect("catch up"),
        CatchUp::Write { .. }
    ));
}

#[test]
fn saving_nothing_new_while_behind_records_nothing() {
    let (_folder, mut window, files) = behind_main();
    let main = window.main().expect("main");
    let saved = window.save(&files, &user(), "Saved").expect("save");
    assert_eq!(saved, main);
    assert_eq!(window.versions(10).expect("versions").len(), 2);
}

#[test]
fn adding_to_a_suggestion_that_moved_on_is_refused() {
    let folder = dresser_folder();
    let mut history = History::start(folder.path()).expect("start");
    let files = history.read_folder().expect("read folder");
    history.save(&files, &user(), "First cut").expect("save");
    let wider = with(&files, "dresser.stave", WIDER);
    let id = common::suggest(&mut history, &agent(), None, &wider, "p", "Widen").expect("suggest");
    let tip = common::tip(&history, &id).expect("tip");
    // Someone adds to it first.
    common::suggest(&mut history, &agent(), Some(&id), &wider, "q", "Again").expect("add");
    let narrower = with(&files, "dresser.stave", NARROWER);
    let error = history
        .suggest(&agent(), Some(&id), &tip, &narrower, "r", "Narrow")
        .expect_err("moved on");
    assert!(
        matches!(error, HistoryError::SuggestionMoved { .. }),
        "{error:?}"
    );
    let error = history
        .write_suggestion_files(&id, &tip, &narrower, &agent())
        .expect_err("moved on");
    assert!(
        matches!(error, HistoryError::SuggestionMoved { .. }),
        "{error:?}"
    );
}

#[test]
fn a_new_suggestion_starts_from_the_base_it_was_made_on() {
    let folder = dresser_folder();
    let mut history = History::start(folder.path()).expect("start");
    let files = history.read_folder().expect("read folder");
    history.save(&files, &user(), "First cut").expect("save");
    let base = history.main().expect("main");
    // The window saves after the agent read main.
    let deeper = with(
        &files,
        "dresser.stave",
        "width = 900mm\nheight = 1200mm\ndepth = 500mm\n",
    );
    history.save(&deeper, &user(), "Deeper").expect("save");
    let wider = with(&files, "dresser.stave", WIDER);
    let id = history
        .suggest(&agent(), None, &base, &wider, "p", "Widen")
        .expect("suggest");
    let suggestion = history
        .suggestions()
        .expect("suggestions")
        .into_iter()
        .find(|suggestion| suggestion.id == id)
        .expect("listed");
    // It doesn't undo the save: it starts before it.
    assert_eq!(suggestion.base, base);
}

#[test]
fn an_agent_cannot_undo_a_suggestion_someone_else_moved_on() {
    let folder = dresser_folder();
    let mut history = History::start(folder.path()).expect("start");
    let files = history.read_folder().expect("read folder");
    history.save(&files, &user(), "First cut").expect("save");
    let wider = with(&files, "dresser.stave", WIDER);
    let id = common::suggest(&mut history, &agent(), None, &wider, "p", "Widen").expect("suggest");
    let tip = common::tip(&history, &id).expect("tip");
    history
        .write_suggestion_files(
            &id,
            &tip,
            &with(&files, "dresser.stave", NARROWER),
            &other_agent(),
        )
        .expect("resolve");
    let error = history
        .undo(&agent(), UndoScope::Own)
        .expect_err("someone else built on it");
    assert!(
        matches!(error, HistoryError::UndoBlocked { .. }),
        "{error:?}"
    );
}

#[test]
fn a_clash_on_main_never_moves_the_git_branch() {
    let folder = git_dresser_folder();
    let mut window = History::start(folder.path()).expect("start");
    let files = window.read_folder().expect("read folder");
    let mut agent_history = History::open(folder.path()).expect("agent opens");
    let base = agent_history.main().expect("main");
    let wider = with(&files, "dresser.stave", WIDER);
    let id =
        common::suggest(&mut agent_history, &agent(), None, &wider, "p", "Widen").expect("suggest");
    agent_history
        .apply(&id, &agent(), &base, &wider)
        .expect("apply");
    let branch_before = git_output(folder.path(), &["rev-parse", "main"], "");
    // The window, behind, changes the same line: main records a clash.
    window
        .save(&with(&files, "dresser.stave", NARROWER), &user(), "Narrow")
        .expect("save");
    assert_eq!(
        git_output(folder.path(), &["rev-parse", "main"], ""),
        branch_before
    );
    assert_eq!(
        git_output(folder.path(), &["status", "--porcelain"], ""),
        "M dresser.stave"
    );
}

#[test]
fn two_starts_at_once_make_one_repository() {
    let folder = dresser_folder();
    let path = folder.path().to_path_buf();
    let starts: Vec<_> = (0..4)
        .map(|_| {
            let path = path.clone();
            std::thread::spawn(move || History::start(&path).map(|_history| ()))
        })
        .collect();
    for start in starts {
        start.join().expect("thread").expect("start");
    }
    let mut history = History::open(folder.path()).expect("open");
    let files = history.read_folder().expect("read folder");
    assert_eq!(text(&files, "dresser.stave"), DRESSER);
    history.save(&files, &user(), "First cut").expect("save");
}

#[test]
fn text_merges_line_by_line_and_marks_a_clash() {
    let base = DRESSER.as_bytes();
    let deeper = b"width = 900mm\nheight = 1200mm\ndepth = 500mm\n";
    let clean = merge_text(base, WIDER.as_bytes(), deeper);
    assert!(!clean.conflicted);
    assert_eq!(
        clean.contents,
        b"width = 1000mm\nheight = 1200mm\ndepth = 500mm\n"
    );
    let clash = merge_text(base, WIDER.as_bytes(), NARROWER.as_bytes());
    assert!(clash.conflicted);
    let shown = String::from_utf8(clash.contents).expect("text");
    assert!(shown.contains("width = 1000mm") && shown.contains("width = 800mm"));
}
