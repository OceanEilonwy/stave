//! Conflict markers (grammar, "Conflicts"): finding conflicts, choosing a
//! side, and resolving one.

use stave_lang::conflict::{Resolution, ResolveError, find_conflicts, resolve, side_text};

const SNAPSHOT: &str = "\
stave 1

<<<<<<< conflict 1 of 1
+++++++ your saved work
let shelf_count = 3
------- base
let shelf_count = 2
+++++++ Claude Desktop
let shelf_count = 4
>>>>>>> conflict 1 of 1 ends
let after = 1
";

const GIT: &str = "\
stave 1

<<<<<<< HEAD
let shelf_count = 3
||||||| base
let shelf_count = 2
=======
let shelf_count = 4
>>>>>>> suggestion
let after = 1
";

fn slice(text: &str, range: std::ops::Range<usize>) -> &str {
    &text[range]
}

#[test]
fn finds_snapshot_style() {
    let conflicts = find_conflicts(SNAPSHOT);
    let [conflict] = conflicts.as_slice() else {
        panic!("one conflict: {conflicts:?}");
    };
    assert!(slice(SNAPSHOT, conflict.range.clone()).starts_with("<<<<<<< conflict 1 of 1\n"));
    assert!(slice(SNAPSHOT, conflict.range.clone()).ends_with(">>>>>>> conflict 1 of 1 ends\n"));
    let labels: Vec<&str> = conflict
        .sides
        .iter()
        .map(|side| side.label.as_str())
        .collect();
    assert_eq!(labels, ["your saved work", "Claude Desktop"]);
    let texts: Vec<&str> = conflict
        .sides
        .iter()
        .map(|side| slice(SNAPSHOT, side.range.clone()))
        .collect();
    assert_eq!(texts, ["let shelf_count = 3\n", "let shelf_count = 4\n"]);
    let base = conflict.base.as_ref().expect("a base");
    assert_eq!(base.label, "base");
    assert_eq!(slice(SNAPSHOT, base.range.clone()), "let shelf_count = 2\n");
}

#[test]
fn finds_git_style_with_base() {
    let conflicts = find_conflicts(GIT);
    let [conflict] = conflicts.as_slice() else {
        panic!("one conflict: {conflicts:?}");
    };
    let labels: Vec<&str> = conflict
        .sides
        .iter()
        .map(|side| side.label.as_str())
        .collect();
    assert_eq!(labels, ["HEAD", "suggestion"]);
    let texts: Vec<&str> = conflict
        .sides
        .iter()
        .map(|side| slice(GIT, side.range.clone()))
        .collect();
    assert_eq!(texts, ["let shelf_count = 3\n", "let shelf_count = 4\n"]);
    let base = conflict.base.as_ref().expect("a base");
    assert_eq!(base.label, "base");
    assert_eq!(slice(GIT, base.range.clone()), "let shelf_count = 2\n");
    assert_eq!(slice(GIT, conflict.range.clone()).lines().count(), 7);
}

#[test]
fn finds_git_style_without_base_and_with_empty_side() {
    let text = "<<<<<<< ours\nlet a = 1\n=======\n>>>>>>> theirs\n";
    let conflicts = find_conflicts(text);
    let [conflict] = conflicts.as_slice() else {
        panic!("one conflict: {conflicts:?}");
    };
    assert!(conflict.base.is_none());
    assert_eq!(conflict.range, 0..text.len());
    assert_eq!(slice(text, conflict.sides[0].range.clone()), "let a = 1\n");
    assert_eq!(slice(text, conflict.sides[1].range.clone()), "");
}

#[test]
fn accepts_longer_markers() {
    // jj lengthens every marker of a conflict when the file holds a run of
    // seven; a run of seven inside it is then text.
    let text = "\
<<<<<<<<<<< conflict 1 of 1
+++++++++++ side #1
let a = 1
======= not a marker here
----------- base
let a = 0
+++++++++++ side #2
let a = 2
>>>>>>>>>>> conflict 1 of 1 ends
";
    let conflicts = find_conflicts(text);
    let [conflict] = conflicts.as_slice() else {
        panic!("one conflict: {conflicts:?}");
    };
    assert_eq!(
        slice(text, conflict.sides[0].range.clone()),
        "let a = 1\n======= not a marker here\n"
    );
    assert_eq!(slice(text, conflict.sides[1].range.clone()), "let a = 2\n");
}

#[test]
fn six_chars_is_not_a_marker() {
    assert!(
        find_conflicts("<<<<<< ours\nlet a = 1\n======\nlet a = 2\n>>>>>> theirs\n").is_empty()
    );
    // A run must be followed by a space or the end of the line.
    assert!(find_conflicts("<<<<<<<x\nlet a = 1\n=======\nlet a = 2\n>>>>>>>\n").is_empty());
    // Only at the start of a line.
    assert!(find_conflicts(" <<<<<<<\nlet a = 1\n=======\nlet a = 2\n>>>>>>>\n").is_empty());
}

#[test]
fn markers_end_at_a_carriage_return_and_at_the_end_of_the_file() {
    let text = "<<<<<<<\r\nlet a = 1\r\n=======\r\nlet a = 2\r\n>>>>>>>";
    let conflicts = find_conflicts(text);
    let [conflict] = conflicts.as_slice() else {
        panic!("one conflict: {conflicts:?}");
    };
    assert_eq!(conflict.range, 0..text.len());
    assert_eq!(conflict.sides[0].label, "");
    assert_eq!(
        slice(text, conflict.sides[1].range.clone()),
        "let a = 2\r\n"
    );
}

#[test]
fn shapes_stave_cannot_read_are_not_conflicts() {
    for text in [
        // never closed
        "<<<<<<< ours\nlet a = 1\n=======\nlet a = 2\n",
        // a diff section
        "<<<<<<< conflict\n%%%%%%% diff\n-let a = 1\n+let a = 2\n+++++++ side #2\nlet a = 3\n>>>>>>> end\n",
        // snapshot with one side
        "<<<<<<< conflict\n+++++++ side #1\nlet a = 1\n>>>>>>> end\n",
        // git without a separator
        "<<<<<<< ours\nlet a = 1\n>>>>>>> theirs\n",
        // a stray separator
        "let a = 1\n=======\nlet a = 2\n",
    ] {
        assert!(find_conflicts(text).is_empty(), "{text:?}");
    }
}

#[test]
fn finds_several_conflicts_in_order() {
    let text = format!("{GIT}{SNAPSHOT}");
    let conflicts = find_conflicts(&text);
    assert_eq!(conflicts.len(), 2);
    assert!(conflicts[0].range.end <= conflicts[1].range.start);
}

#[test]
fn side_text_picks_each_side() {
    assert_eq!(
        side_text(SNAPSHOT, 0),
        "stave 1\n\nlet shelf_count = 3\nlet after = 1\n"
    );
    assert_eq!(
        side_text(SNAPSHOT, 1),
        "stave 1\n\nlet shelf_count = 4\nlet after = 1\n"
    );
    assert_eq!(side_text(GIT, 0), side_text(SNAPSHOT, 0));
    assert_eq!(side_text(GIT, 1), side_text(SNAPSHOT, 1));
    // A file without conflicts is its own every side.
    assert_eq!(side_text("stave 1\n", 3), "stave 1\n");
}

#[test]
fn side_text_chooses_the_side_in_every_conflict() {
    let text = format!("{GIT}{}", SNAPSHOT.trim_start_matches("stave 1\n"));
    assert_eq!(
        side_text(&text, 1),
        "stave 1\n\nlet shelf_count = 4\nlet after = 1\n\nlet shelf_count = 4\nlet after = 1\n"
    );
}

#[test]
fn resolve_keep_one_side() {
    assert_eq!(
        resolve(GIT, 0, &Resolution::Keep(1)),
        Ok("stave 1\n\nlet shelf_count = 4\nlet after = 1\n".to_owned())
    );
}

#[test]
fn resolve_keep_both_joins_sides() {
    let text = "\
stave 1

<<<<<<< conflict 1 of 1
+++++++ your saved work
let shelf_count = 3
------- base
+++++++ Claude Desktop
let drawer_count = 2
>>>>>>> conflict 1 of 1 ends
";
    assert_eq!(
        resolve(text, 0, &Resolution::KeepBoth),
        Ok("stave 1\n\nlet shelf_count = 3\nlet drawer_count = 2\n".to_owned())
    );
}

#[test]
fn resolve_keep_both_ends_every_side_with_a_newline() {
    let text = "<<<<<<< a\nlet a = 1\n=======\nlet b = 2\n>>>>>>> b";
    assert_eq!(
        resolve(text, 0, &Resolution::KeepBoth),
        Ok("let a = 1\nlet b = 2\n".to_owned())
    );
}

#[test]
fn resolve_with_new_text_keeps_the_next_line_apart() {
    assert_eq!(
        resolve(GIT, 0, &Resolution::Text("let shelf_count = 5".to_owned())),
        Ok("stave 1\n\nlet shelf_count = 5\nlet after = 1\n".to_owned())
    );
    assert_eq!(
        resolve(GIT, 0, &Resolution::Text(String::new())),
        Ok("stave 1\n\nlet after = 1\n".to_owned())
    );
}

#[test]
fn resolve_refuses_what_is_not_there() {
    assert_eq!(
        resolve(GIT, 1, &Resolution::KeepBoth),
        Err(ResolveError::NoSuchConflict {
            conflict: 1,
            count: 1
        })
    );
    assert_eq!(
        resolve(GIT, 0, &Resolution::Keep(2)),
        Err(ResolveError::NoSuchSide { side: 2, count: 2 })
    );
}
