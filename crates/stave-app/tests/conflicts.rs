//! Conflicts as the window and the AI tools see them: each conflict's ways
//! to settle it, each side checked on its own, and a resolution tried
//! before it is written.

use std::path::{Path, PathBuf};

use stave_app::conflicts::{self, SideChange};
use stave_lang::conflict::Resolution;
use stave_modules::MemoryFiles;

/// `examples/box` in memory, with `box.stave`'s `box_length` line changed
/// into a conflict whose sides set `yours` and `theirs`.
fn box_with_conflict(yours: &str, theirs: &str) -> (PathBuf, MemoryFiles, String) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/box");
    let folder = PathBuf::from("/project/box");
    let original = std::fs::read_to_string(root.join("box.stave")).unwrap();
    let line = original
        .lines()
        .find(|line| line.starts_with("param box_length"))
        .unwrap();
    let conflicted = original.replacen(
        line,
        &format!(
            "<<<<<<< conflict 1 of 1\n\
             +++++++ yours\n\
             param box_length = {yours} within 200mm..450mm\n\
             ------- before\n\
             {line}\n\
             +++++++ theirs\n\
             param box_length = {theirs} within 200mm..450mm\n\
             >>>>>>> conflict 1 of 1 ends"
        ),
        1,
    );
    let lid = std::fs::read_to_string(root.join("lid.stave")).unwrap();
    let files = MemoryFiles::new()
        .with(folder.join("box.stave"), conflicted.clone())
        .with(folder.join("lid.stave"), lid);
    (folder, files, conflicted)
}

#[test]
fn conflict_fixes_keep_each_side() {
    let (_, _, text) = box_with_conflict("320mm", "350mm");
    let found = conflicts::conflicts(&text);
    assert_eq!(found.len(), 1);
    let conflict = &found[0];
    let labels: Vec<&str> = conflict
        .actions
        .iter()
        .map(|action| action.label.as_str())
        .collect();
    // Both sides declare `box_length`, so keeping both is not offered.
    assert_eq!(labels, ["Keep yours", "Keep theirs"]);
    for (side, value) in [(0, "320mm"), (1, "350mm")] {
        let action = &conflict.actions[side];
        assert_eq!(action.resolution, Resolution::Keep(side));
        let kept =
            stave_lang::patch::apply_text_edits(&text, std::slice::from_ref(&action.edit)).unwrap();
        assert!(kept.contains(&format!("param box_length = {value} within")));
        assert!(stave_lang::conflict::find_conflicts(&kept).is_empty());
    }
    // The conflict's lines, counted from 1, marker lines included.
    let first = text
        .lines()
        .position(|line| line.starts_with("<<<<<<<"))
        .unwrap()
        + 1;
    assert_eq!(conflict.lines, first..=first + 7);
}

#[test]
fn keep_both_is_offered_for_whole_items_with_different_names() {
    let text = "stave 1\n<<<<<<< conflict 1 of 1\n+++++++ yours\nlet a = 1\n------- before\n+++++++ theirs\nlet b = 2\n>>>>>>> conflict 1 of 1 ends\n";
    let found = conflicts::conflicts(text);
    let labels: Vec<&str> = found[0]
        .actions
        .iter()
        .map(|action| action.label.as_str())
        .collect();
    assert_eq!(labels, ["Keep yours", "Keep theirs", "Keep both"]);
}

#[test]
fn each_keep_names_its_side() {
    let labels = |text: &str| -> Vec<String> {
        conflicts::conflicts(text)[0]
            .actions
            .iter()
            .map(|action| action.label.clone())
            .collect()
    };
    // No labels Stave knows: two sides by their tint, more by number.
    let two = "stave 1\n<<<<<<<\nlet a = 1\n=======\nlet a = 2\n>>>>>>>\n";
    assert_eq!(labels(two), ["Keep the green side", "Keep the blue side"]);
    let three = "stave 1\n<<<<<<< conflict 1 of 1\n+++++++ one\nlet a = 1\n------- before\n+++++++ two\nlet a = 2\n------- before\n+++++++ three\nlet a = 3\n>>>>>>> conflict 1 of 1 ends\n";
    assert_eq!(labels(three), ["Keep side 1", "Keep side 2", "Keep side 3"]);
    // A suggestion moved onto main: jj's labels.
    let rebased = "stave 1\n<<<<<<< conflict 1 of 1\n+++++++ abc \"Longer\" (rebased revision)\nlet a = 1\n------- before\n+++++++ def \"Saved\" (rebase destination)\nlet a = 2\n>>>>>>> conflict 1 of 1 ends\n";
    assert_eq!(
        labels(rebased),
        ["Keep the suggestion's", "Keep the project's"]
    );
}

#[test]
fn each_side_is_checked_on_its_own() {
    // 500mm is outside `within 200mm..450mm`, so only the other side errs.
    let (folder, files, _) = box_with_conflict("320mm", "500mm");
    let sides = conflicts::check_sides(&folder, &files).unwrap();
    assert_eq!(sides.len(), 2);
    assert_eq!(sides[0].error_count(), 0, "{:#?}", sides[0].diagnostics);
    assert!(sides[1].error_count() > 0);
}

#[test]
fn a_side_as_a_change_against_its_base() {
    let (_, _, text) = box_with_conflict("320mm", "350mm");
    let found = conflicts::conflicts(&text);
    let change = conflicts::side_change(&text, &found[0].conflict, 1);
    let SideChange::Against { lines } = change else {
        panic!("the conflict has a base");
    };
    assert_eq!(
        lines,
        "- param box_length = 300mm within 200mm..450mm        // outside, left to right\n\
         + param box_length = 350mm within 200mm..450mm\n"
    );
}

#[test]
fn trying_a_resolution_checks_the_result_and_writes_nothing() {
    let (folder, files, text) = box_with_conflict("320mm", "350mm");
    let path = folder.join("box.stave");
    let tried = conflicts::try_resolution(&folder, &files, &path, 0, &Resolution::Keep(1)).unwrap();
    assert!(tried.text.contains("param box_length = 350mm"));
    assert_eq!(tried.opened.error_count(), 0);
    // A resolution that leaves text Stave can't read is reported.
    let broken = conflicts::try_resolution(
        &folder,
        &files,
        &path,
        0,
        &Resolution::Text("param box_length = ".to_owned()),
    )
    .unwrap();
    assert!(broken.opened.error_count() > 0);
    // There is one conflict, so a second can't be resolved.
    assert!(conflicts::try_resolution(&folder, &files, &path, 1, &Resolution::KeepBoth).is_err());
    assert_eq!(
        stave_modules::FileSource::read(&files, &path).unwrap(),
        text
    );
}
