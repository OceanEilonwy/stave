//! The conflict tools, called as an AI app calls them, on copies of
//! `examples/box` holding a clash.

mod common;

use std::path::Path;

use common::{connect, example_copy, is_error, options, structured, text};
use serde_json::json;
use stave_history::{Actor, ActorKind, History, ProjectFiles, RelativePath, SuggestionId};

/// The `box_length` line of `examples/box/box.stave`.
fn length_line(text: &str) -> String {
    text.lines()
        .find(|line| line.starts_with("param box_length"))
        .unwrap()
        .to_owned()
}

/// `box.stave` with its `box_length` line set to `value`.
fn with_length(text: &str, value: &str) -> String {
    text.replacen(
        &length_line(text),
        &format!("param box_length = {value} within 200mm..450mm"),
        1,
    )
}

/// Writes a conflict over the `box_length` line of the copy's `box.stave`:
/// "yours" sets `yours`, "theirs" sets `theirs`. Returns the file's text.
fn write_length_conflict(folder: &Path, yours: &str, theirs: &str) -> String {
    let path = folder.join("box.stave");
    let original = std::fs::read_to_string(&path).unwrap();
    let line = length_line(&original);
    let conflicted = original.replacen(
        &line,
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
    std::fs::write(&path, &conflicted).unwrap();
    conflicted
}

async fn open(connection: &common::Connection, folder: &Path) {
    let result = connection
        .call(
            "open_project",
            json!({ "path": folder.display().to_string() }),
        )
        .await;
    assert!(!is_error(&result), "{}", text(&result));
}

#[tokio::test(flavor = "multi_thread")]
async fn list_conflicts_describes_sides_as_patches() {
    let folder = example_copy("box", "list_conflicts_describes_sides_as_patches");
    write_length_conflict(&folder, "320mm", "500mm");
    let connection = connect(options("list_conflicts_describes_sides_as_patches")).await;
    open(&connection, &folder).await;
    let result = connection.call("list_conflicts", json!({})).await;
    assert!(!is_error(&result), "{}", text(&result));
    let listed = structured(&result);
    let conflicts = listed["conflicts"].as_array().unwrap();
    assert_eq!(conflicts.len(), 1, "{listed:#}");
    let conflict = &conflicts[0];
    assert_eq!(conflict["file"], json!("box.stave"));
    assert_eq!(conflict["number"], json!(1));
    assert!(
        conflict["base"]
            .as_str()
            .unwrap()
            .contains("box_length = 300mm")
    );
    let sides = conflict["sides"].as_array().unwrap();
    assert_eq!(sides.len(), 2);
    assert_eq!(sides[0]["label"], json!("yours"));
    assert!(
        sides[0]["change"]
            .as_str()
            .unwrap()
            .contains("+ param box_length = 320mm"),
        "{:#}",
        sides[0]
    );
    assert!(
        sides[1]["change"]
            .as_str()
            .unwrap()
            .contains("- param box_length = 300mm"),
        "{:#}",
        sides[1]
    );
    assert!(
        sides[1]["change"]
            .as_str()
            .unwrap()
            .contains("+ param box_length = 500mm")
    );
    // 500mm is outside its range: only that side has errors.
    assert_eq!(sides[0]["errors"], json!(0), "{:#}", sides[0]);
    assert!(sides[1]["errors"].as_u64().unwrap() > 0);
    // "yours" is the window's side: the person using Stave.
    assert!(
        sides[0]["origin"]
            .as_str()
            .unwrap()
            .contains("person using Stave"),
        "{:#}",
        sides[0]
    );
    let words = text(&result);
    assert!(words.contains("box.stave"), "{words}");
    assert!(words.contains("+ param box_length = 500mm"), "{words}");
}

#[tokio::test(flavor = "multi_thread")]
async fn try_resolution_reports_problems() {
    let folder = example_copy("box", "try_resolution_reports_problems");
    let conflicted = write_length_conflict(&folder, "320mm", "350mm");
    let connection = connect(options("try_resolution_reports_problems")).await;
    open(&connection, &folder).await;

    let clean = connection
        .call(
            "try_resolution",
            json!({ "file": "box.stave", "conflict": 1, "resolution": { "keep": 2 } }),
        )
        .await;
    assert!(!is_error(&clean), "{}", text(&clean));
    assert_eq!(structured(&clean)["errors"], json!(0));

    let broken = connection
        .call(
            "try_resolution",
            json!({
                "file": "box.stave",
                "conflict": 1,
                "resolution": { "text": "param box_length = 500mm within 200mm..450mm" },
            }),
        )
        .await;
    assert!(!is_error(&broken), "{}", text(&broken));
    let tried = structured(&broken);
    assert!(tried["errors"].as_u64().unwrap() > 0, "{tried:#}");
    assert!(!tried["problems"].as_array().unwrap().is_empty());
    assert!(text(&broken).contains("error"), "{}", text(&broken));

    // A conflict that isn't there, and a file outside the project.
    let missing = connection
        .call(
            "try_resolution",
            json!({ "file": "box.stave", "conflict": 2, "resolution": "keep_both" }),
        )
        .await;
    assert!(is_error(&missing));
    let outside = connection
        .call(
            "try_resolution",
            json!({ "file": "../box.stave", "conflict": 1, "resolution": { "keep": 1 } }),
        )
        .await;
    assert!(is_error(&outside));
    assert!(text(&outside).contains("outside the project"));

    // Trying writes nothing.
    assert_eq!(
        std::fs::read_to_string(folder.join("box.stave")).unwrap(),
        conflicted
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn resolve_keep_both_writes_and_records() {
    let folder = example_copy("box", "resolve_keep_both_writes_and_records");
    History::start(&folder).unwrap();
    let path = folder.join("box.stave");
    let original = std::fs::read_to_string(&path).unwrap();
    let line = length_line(&original);
    let conflicted = original.replacen(
        &line,
        &format!(
            "{line}\n\
             <<<<<<< conflict 1 of 1\n\
             +++++++ yours\n\
             let lid_gap = 1mm\n\
             ------- before\n\
             +++++++ theirs\n\
             let lid_overhang = 2mm\n\
             >>>>>>> conflict 1 of 1 ends"
        ),
        1,
    );
    std::fs::write(&path, &conflicted).unwrap();
    let connection = connect(options("resolve_keep_both_writes_and_records")).await;
    open(&connection, &folder).await;

    let result = connection
        .call(
            "resolve_conflict",
            json!({ "file": "box.stave", "conflict": 1, "resolution": "keep_both" }),
        )
        .await;
    assert!(!is_error(&result), "{}", text(&result));
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.contains("let lid_gap = 1mm\nlet lid_overhang = 2mm\n"));
    assert!(stave_lang::conflict::find_conflicts(&written).is_empty());

    let history = History::open(&folder).unwrap();
    let latest = &history.versions(1).unwrap()[0];
    assert_eq!(latest.actor.kind, ActorKind::Agent);
    assert_eq!(latest.actor.name, "Test Host");
    let main_files = history.files_at(&history.main().unwrap()).unwrap();
    let box_stave = RelativePath::new(Path::new("box.stave")).unwrap();
    assert_eq!(main_files.get(&box_stave), Some(written.as_bytes()));
}

#[tokio::test(flavor = "multi_thread")]
async fn resolve_refuses_a_result_that_does_not_check_clean() {
    let folder = example_copy("box", "resolve_refuses_unclean");
    let conflicted = write_length_conflict(&folder, "320mm", "500mm");
    let connection = connect(options("resolve_refuses_unclean")).await;
    open(&connection, &folder).await;
    let result = connection
        .call(
            "resolve_conflict",
            json!({ "file": "box.stave", "conflict": 1, "resolution": { "keep": 2 } }),
        )
        .await;
    assert!(is_error(&result), "{}", text(&result));
    assert_eq!(
        std::fs::read_to_string(folder.join("box.stave")).unwrap(),
        conflicted
    );
}

/// A suggestion by another agent that sets `box_length` to 350mm, made
/// before the person saved 320mm, then moved onto main so it clashes.
fn clashing_suggestion(folder: &Path) -> SuggestionId {
    let mut history = History::start(folder).unwrap();
    let box_stave = RelativePath::new(Path::new("box.stave")).unwrap();
    let original = std::fs::read_to_string(folder.join("box.stave")).unwrap();
    let agent = Actor {
        kind: ActorKind::Agent,
        name: "Other Host".to_owned(),
        id: "0001".to_owned(),
    };
    let person = Actor {
        kind: ActorKind::User,
        name: "Person".to_owned(),
        id: String::new(),
    };
    // The window records the project as it is first.
    history
        .save(&history.read_folder().unwrap(), &person, "Start")
        .unwrap();
    let mut suggested: ProjectFiles = history.read_folder().unwrap();
    suggested.insert(
        box_stave.clone(),
        with_length(&original, "350mm").into_bytes(),
    );
    let id = history
        .suggest(
            &agent,
            None,
            &history.main().unwrap(),
            &suggested,
            "patch \"Longer\" { set box_length = 350mm }",
            "Make the box longer",
        )
        .unwrap();
    let mut saved = history.read_folder().unwrap();
    saved.insert(box_stave, with_length(&original, "320mm").into_bytes());
    history.save(&saved, &person, "A little longer").unwrap();
    let conflicted = history.rebase_suggestion(&id, &agent).unwrap();
    assert_eq!(conflicted.len(), 1);
    id
}

#[tokio::test(flavor = "multi_thread")]
async fn resolve_in_a_suggestion_writes_the_suggestion() {
    let folder = example_copy("box", "resolve_in_a_suggestion");
    let id = clashing_suggestion(&folder);
    let before = std::fs::read_to_string(folder.join("box.stave")).unwrap();
    let connection = connect(options("resolve_in_a_suggestion")).await;
    open(&connection, &folder).await;

    // The project's own files have no clash; the suggestion has one.
    let project = connection.call("list_conflicts", json!({})).await;
    assert!(!is_error(&project), "{}", text(&project));
    assert!(
        structured(&project)["conflicts"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(text(&project).contains(id.as_str()), "{}", text(&project));

    let listed = connection
        .call("list_conflicts", json!({ "suggestion": id.as_str() }))
        .await;
    assert!(!is_error(&listed), "{}", text(&listed));
    let conflicts = structured(&listed)["conflicts"].as_array().unwrap().clone();
    assert_eq!(conflicts.len(), 1, "{}", text(&listed));
    // The words name where each side came from, not jj's revision labels.
    assert!(
        !text(&listed).contains("rebased revision"),
        "{}",
        text(&listed)
    );
    let origins: Vec<String> = conflicts[0]["sides"]
        .as_array()
        .unwrap()
        .iter()
        .map(|side| side["origin"].as_str().unwrap().to_owned())
        .collect();
    assert!(
        origins.iter().any(|origin| origin.contains("Other Host")),
        "{origins:?}"
    );
    let suggestion_side = conflicts[0]["sides"]
        .as_array()
        .unwrap()
        .iter()
        .position(|side| side["origin"].as_str().unwrap().contains("Other Host"))
        .unwrap();

    let result = connection
        .call(
            "resolve_conflict",
            json!({
                "file": "box.stave",
                "conflict": 1,
                "resolution": { "keep": suggestion_side + 1 },
                "suggestion": id.as_str(),
            }),
        )
        .await;
    assert!(!is_error(&result), "{}", text(&result));
    let history = History::open(&folder).unwrap();
    let suggestion = history
        .suggestions()
        .unwrap()
        .into_iter()
        .find(|suggestion| suggestion.id == id)
        .unwrap();
    assert!(suggestion.conflicted.is_empty());
    let files = history.suggestion_files(&id).unwrap();
    let box_stave = RelativePath::new(Path::new("box.stave")).unwrap();
    let resolved = String::from_utf8(files.get(&box_stave).unwrap().to_vec()).unwrap();
    assert!(resolved.contains("param box_length = 350mm"));
    // The project folder is not touched.
    assert_eq!(
        std::fs::read_to_string(folder.join("box.stave")).unwrap(),
        before
    );
}

/// Turns the line of `text` starting `param {name}` into a clash whose
/// sides set `yours` and `theirs`, keeping its `within` range.
fn clash_over(text: &str, name: &str, yours: &str, theirs: &str) -> String {
    let line = text
        .lines()
        .find(|line| line.starts_with(&format!("param {name} ")))
        .unwrap()
        .to_owned();
    let within = line[line.find(" within").unwrap()..].to_owned();
    text.replacen(
        &line,
        &format!(
            "<<<<<<< conflict\n\
             +++++++ yours\n\
             param {name} = {yours}{within}\n\
             ------- before\n\
             {line}\n\
             +++++++ theirs\n\
             param {name} = {theirs}{within}\n\
             >>>>>>> conflict ends"
        ),
        1,
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn resolve_one_of_two_clashes_writes_and_leaves_the_other() {
    let folder = example_copy("box", "resolve_one_of_two_clashes");
    let path = folder.join("box.stave");
    let original = std::fs::read_to_string(&path).unwrap();
    let conflicted = clash_over(
        &clash_over(&original, "box_length", "320mm", "350mm"),
        "box_depth",
        "210mm",
        "220mm",
    );
    std::fs::write(&path, &conflicted).unwrap();
    let connection = connect(options("resolve_one_of_two_clashes")).await;
    open(&connection, &folder).await;

    // A resolution that breaks every way of settling the other clash is
    // refused, and nothing is written.
    let broken = connection
        .call(
            "resolve_conflict",
            json!({
                "file": "box.stave",
                "conflict": 1,
                "resolution": { "text": "param box_length = 500mm within 200mm..450mm\n" },
            }),
        )
        .await;
    assert!(is_error(&broken), "{}", text(&broken));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), conflicted);

    // Settling the first clash is written, though the second is left.
    let result = connection
        .call(
            "resolve_conflict",
            json!({ "file": "box.stave", "conflict": 1, "resolution": { "keep": 1 } }),
        )
        .await;
    assert!(!is_error(&result), "{}", text(&result));
    assert_eq!(structured(&result)["conflicts_left_in_project"], json!(1));
    assert!(
        text(&result).contains("still have 1 clash"),
        "{}",
        text(&result)
    );
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.contains("param box_length = 320mm within"));
    assert_eq!(stave_lang::conflict::find_conflicts(&written).len(), 1);

    let listed = connection.call("list_conflicts", json!({})).await;
    let conflicts = structured(&listed)["conflicts"].as_array().unwrap().clone();
    assert_eq!(conflicts.len(), 1, "{}", text(&listed));
    assert!(
        conflicts[0]["base"]
            .as_str()
            .unwrap()
            .contains("param box_depth")
    );

    // And the second can then be settled too.
    let last = connection
        .call(
            "resolve_conflict",
            json!({ "file": "box.stave", "conflict": 1, "resolution": { "keep": 2 } }),
        )
        .await;
    assert!(!is_error(&last), "{}", text(&last));
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.contains("param box_depth = 220mm within"));
    assert!(stave_lang::conflict::find_conflicts(&written).is_empty());
}
