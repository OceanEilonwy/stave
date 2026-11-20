//! Suggestions, applying them, undo and the merge policy, called as an AI
//! app calls them, on copies of `examples/`.

mod common;

use std::path::{Path, PathBuf};

use common::{connect, example_copy, is_error, options, structured, text};
use serde_json::json;
use stave_history::{Actor, ActorKind, History, ProjectFiles, RelativePath, RevisionId};

const SHOE_SHELF: &str = r#"patch "Add a shoe shelf" {
  add part shoe_shelf : oak_19 {
    length: fit
    width:  cabinet_depth - back.thickness
    place {
      against(divider.top, gap: 400mm)
      center(base, across)
      flush(side_l.front)
    }
  } in "carcass.stave"
  add joint shoe_shelf_dados : dado(shoe_shelf, into: [side_l, side_r], depth: 9mm)
}
"#;

fn path_text(path: &Path) -> String {
    path.display().to_string()
}

fn person() -> Actor {
    Actor {
        kind: ActorKind::User,
        name: "Ann".to_owned(),
        id: "window".to_owned(),
    }
}

fn relative(path: &str) -> RelativePath {
    RelativePath::new(Path::new(path)).unwrap()
}

fn file_text(files: &ProjectFiles, path: &str) -> String {
    String::from_utf8(files.get(&relative(path)).unwrap().to_vec()).unwrap()
}

/// The person saves `path` from the window with `from` replaced by `to`.
fn person_saves(folder: &Path, path: &str, from: &str, to: &str) -> RevisionId {
    let mut history = History::open(folder).unwrap();
    let mut files = history.files_at(&history.main().unwrap()).unwrap();
    let changed = file_text(&files, path);
    assert!(changed.contains(from), "{path} holds {from}");
    files.insert(relative(path), changed.replace(from, to).into_bytes());
    history
        .save(&files, &person(), "The person's change")
        .unwrap()
}

/// The project's files on disk, without its history.
fn project_files_on_disk(folder: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let history = folder.join(".stave");
    common::snapshot(folder)
        .into_iter()
        .filter(|(path, _)| !path.starts_with(&history))
        .collect()
}

/// A copy of the clothing cabinet, opened with versions turned on.
async fn opened_with_versions(test: &str) -> (PathBuf, common::Connection) {
    opened_with_versions_and(test, options(test)).await
}

async fn opened_with_versions_and(
    test: &str,
    options: stave_mcp::ServerOptions,
) -> (PathBuf, common::Connection) {
    let folder = example_copy("clothing_cabinet", test);
    let connection = connect(options).await;
    let result = connection
        .call(
            "open_project",
            json!({ "path": path_text(&folder), "start_versions": true }),
        )
        .await;
    assert!(!is_error(&result), "{}", text(&result));
    (folder, connection)
}

async fn suggest_shoe_shelf(connection: &common::Connection) -> String {
    let result = connection
        .call(
            "suggest",
            json!({ "patch": SHOE_SHELF, "title": "Add a shoe shelf" }),
        )
        .await;
    assert!(!is_error(&result), "{}", text(&result));
    structured(&result)["id"].as_str().unwrap().to_owned()
}

#[tokio::test(flavor = "multi_thread")]
async fn open_project_asks_before_turning_versions_on() {
    let folder = example_copy("clothing_cabinet", "open_project_asks");
    let connection = connect(options("open_project_asks")).await;
    let first = connection
        .call("open_project", json!({ "path": path_text(&folder) }))
        .await;
    assert!(!is_error(&first), "{}", text(&first));
    assert!(
        text(&first).contains(
            "Versions are off for this project. Call open_project again with start_versions: \
             true to turn them on."
        ),
        "{}",
        text(&first)
    );
    assert_eq!(structured(&first)["versions"], json!("off"));
    assert!(!folder.join(".stave").exists(), "nothing was written");
    // The tools that need versions say how to turn them on.
    let suggest = connection
        .call("suggest", json!({ "patch": SHOE_SHELF, "title": "Shoes" }))
        .await;
    assert!(is_error(&suggest));
    assert!(
        text(&suggest).contains("start_versions"),
        "{}",
        text(&suggest)
    );

    let second = connection
        .call(
            "open_project",
            json!({ "path": path_text(&folder), "start_versions": true }),
        )
        .await;
    assert!(!is_error(&second), "{}", text(&second));
    assert_eq!(structured(&second)["versions"], json!("on"));
    let history = History::open(&folder).unwrap();
    let versions = history.versions(10).unwrap();
    assert_eq!(
        versions.len(),
        1,
        "the folder as it was is the first version"
    );
    assert_eq!(versions[0].actor.kind, ActorKind::User);
    assert_eq!(
        history.files_at(&history.main().unwrap()).unwrap(),
        history.read_folder().unwrap()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn open_project_nested_folder_refused() {
    let root = common::scratch_folder("open_project_nested_folder_refused")
        .canonicalize()
        .unwrap();
    std::fs::create_dir(root.join(".git")).unwrap();
    let folder = root.join("cabinet");
    std::fs::create_dir(&folder).unwrap();
    let source = common::workspace().join("examples/box");
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_file() {
            std::fs::copy(entry.path(), folder.join(entry.file_name())).unwrap();
        }
    }
    let connection = connect(options("open_project_nested_folder_refused")).await;
    for start_versions in [false, true] {
        let result = connection
            .call(
                "open_project",
                json!({ "path": path_text(&folder), "start_versions": start_versions }),
            )
            .await;
        assert!(is_error(&result));
        let words = text(&result);
        assert!(words.contains(&path_text(&root)), "{words}");
        assert!(words.contains("folder of its own"), "{words}");
    }
    assert!(!folder.join(".stave").exists(), "no history was made");
    let check = connection.call("check", json!({})).await;
    assert!(is_error(&check), "nothing is open");
}

#[tokio::test(flavor = "multi_thread")]
async fn new_project_respects_policy_and_records_revision() {
    let steps = "new \"Box\" 300mm 200mm 200mm\ncreate\n";
    let test = "new_project_respects_policy_and_records_revision";

    // Under `off`, agents make nothing.
    let refusing = options(&format!("{test}-off"));
    std::fs::create_dir_all(&refusing.config_dir).unwrap();
    std::fs::write(
        refusing.config_dir.join("agents.toml"),
        "merge_policy = \"off\"\n",
    )
    .unwrap();
    let root = common::scratch_folder(test).canonicalize().unwrap();
    let folder = root.join("box");
    let connection = connect(refusing).await;
    let result = connection
        .call(
            "new_project",
            json!({ "folder": path_text(&folder), "steps": steps }),
        )
        .await;
    assert!(is_error(&result));
    assert!(
        text(&result).contains("Only the person using Stave"),
        "{}",
        text(&result)
    );
    assert!(!folder.exists(), "nothing was written");

    // Under the default, the project is made with versions on, its first
    // revision recorded as the agent.
    let connection = connect(options(test)).await;
    let result = connection
        .call(
            "new_project",
            json!({ "folder": path_text(&folder), "steps": steps }),
        )
        .await;
    assert!(!is_error(&result), "{}", text(&result));
    let history = History::open(&folder).unwrap();
    let versions = history.versions(10).unwrap();
    assert_eq!(versions.len(), 1);
    assert_eq!(versions[0].actor.kind, ActorKind::Agent);
    assert_eq!(versions[0].actor.name, "Test Host");
    assert_eq!(
        history.files_at(&versions[0].id).unwrap(),
        history.read_folder().unwrap()
    );
    let listed = connection.call("list_versions", json!({})).await;
    assert!(!is_error(&listed), "{}", text(&listed));
    assert_eq!(structured(&listed)["versions"].as_array().unwrap().len(), 1);
    assert!(text(&listed).contains("Test Host"), "{}", text(&listed));
}

#[tokio::test(flavor = "multi_thread")]
async fn suggest_then_apply_moves_main() {
    let (folder, connection) = opened_with_versions("suggest_then_apply_moves_main").await;
    let before = History::open(&folder).unwrap().main().unwrap();
    let on_disk = project_files_on_disk(&folder);

    // A refused patch is not recorded.
    let refused = connection
        .call(
            "suggest",
            json!({ "patch": "patch \"Bad\" { set no_such_thing = 3mm }", "title": "Bad" }),
        )
        .await;
    assert!(is_error(&refused), "{}", text(&refused));

    let id = suggest_shoe_shelf(&connection).await;
    let history = History::open(&folder).unwrap();
    assert_eq!(history.main().unwrap(), before, "suggesting leaves main");
    let suggestions = history.suggestions().unwrap();
    assert_eq!(suggestions.len(), 1);
    assert_eq!(suggestions[0].id.as_str(), id);
    assert_eq!(suggestions[0].agent.name, "Test Host");
    assert!(
        file_text(
            &history.suggestion_files(&suggestions[0].id).unwrap(),
            "carcass.stave"
        )
        .contains("shoe_shelf")
    );

    let listed = connection.call("list_suggestions", json!({})).await;
    assert!(!is_error(&listed), "{}", text(&listed));
    assert!(
        text(&listed).contains("Add a shoe shelf"),
        "{}",
        text(&listed)
    );
    assert_eq!(structured(&listed)["suggestions"][0]["id"], json!(id));

    // try_patch on the suggestion's line sees the shelf already there.
    let again = connection
        .call(
            "try_patch",
            json!({ "patch": SHOE_SHELF, "suggestion": id }),
        )
        .await;
    assert_eq!(structured(&again)["fits"], json!(false), "{}", text(&again));

    let applied = connection
        .call("apply_suggestion", json!({ "id": id }))
        .await;
    assert!(!is_error(&applied), "{}", text(&applied));
    let history = History::open(&folder).unwrap();
    let main = history.main().unwrap();
    assert_ne!(main, before);
    assert_eq!(structured(&applied)["main"], json!(main.as_str()));
    assert!(file_text(&history.files_at(&main).unwrap(), "carcass.stave").contains("shoe_shelf"));
    assert!(history.suggestions().unwrap().is_empty());
    let versions = history.versions(1).unwrap();
    assert_eq!(versions[0].actor.kind, ActorKind::Agent);
    assert_eq!(versions[0].message, "Add a shoe shelf");
    // Only main moved: the window brings the folder up to date.
    assert_eq!(project_files_on_disk(&folder), on_disk);
}

#[tokio::test(flavor = "multi_thread")]
async fn apply_refused_under_off_policy() {
    let (folder, connection) = opened_with_versions("apply_refused_under_off_policy").await;
    std::fs::write(
        folder.join(".stave/agents.toml"),
        "merge_policy = \"off\"\n",
    )
    .unwrap();
    let id = suggest_shoe_shelf(&connection).await;
    let before = History::open(&folder).unwrap().main().unwrap();
    let result = connection
        .call("apply_suggestion", json!({ "id": id }))
        .await;
    assert!(is_error(&result));
    assert_eq!(
        text(&result),
        "Only the person using Stave can apply suggestions in this project."
    );
    let history = History::open(&folder).unwrap();
    assert_eq!(history.main().unwrap(), before);
    assert_eq!(
        history.suggestions().unwrap().len(),
        1,
        "the suggestion stays"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn apply_refused_when_not_clean() {
    let (folder, connection) = opened_with_versions("apply_refused_when_not_clean").await;
    let id = suggest_shoe_shelf(&connection).await;
    // The person saves a design that doesn't check: the require fails.
    let broken = person_saves(
        &folder,
        "cabinet.stave",
        "cabinet_depth - back.thickness >= 540mm",
        "cabinet_depth - back.thickness >= 5400mm",
    );
    let result = connection
        .call("apply_suggestion", json!({ "id": id }))
        .await;
    assert!(is_error(&result), "{}", text(&result));
    assert!(
        text(&result).contains("doesn't check clean"),
        "{}",
        text(&result)
    );
    let history = History::open(&folder).unwrap();
    assert_eq!(history.main().unwrap(), broken, "main didn't move");
    assert_eq!(
        history.suggestions().unwrap().len(),
        1,
        "the suggestion stays"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn apply_replays_after_main_moved() {
    let (folder, connection) = opened_with_versions("apply_replays_after_main_moved").await;
    let id = suggest_shoe_shelf(&connection).await;
    let saved = person_saves(
        &folder,
        "cabinet.stave",
        "param cabinet_width  = 900mm",
        "param cabinet_width  = 950mm",
    );
    let result = connection
        .call("apply_suggestion", json!({ "id": id }))
        .await;
    assert!(!is_error(&result), "{}", text(&result));
    let history = History::open(&folder).unwrap();
    let main = history.main().unwrap();
    assert_ne!(main, saved);
    let files = history.files_at(&main).unwrap();
    assert!(file_text(&files, "carcass.stave").contains("shoe_shelf"));
    assert!(file_text(&files, "cabinet.stave").contains("param cabinet_width  = 950mm"));
    let versions = history.versions(3).unwrap();
    assert_eq!(versions[1].id, saved, "main stays one line");
}

#[tokio::test(flavor = "multi_thread")]
async fn apply_reports_clashes_when_the_patch_no_longer_fits() {
    let (folder, connection) = opened_with_versions("apply_reports_clashes").await;
    let id = suggest_shoe_shelf(&connection).await;
    // The person adds a shelf of the same name at the same place, so the
    // patch no longer fits and the text clashes.
    let saved = {
        let mut history = History::open(&folder).unwrap();
        let mut files = history.files_at(&history.main().unwrap()).unwrap();
        let original = file_text(&files, "carcass.stave");
        let suggested = file_text(
            &history
                .suggestion_files(&stave_history::SuggestionId::new(id.clone()))
                .unwrap(),
            "carcass.stave",
        );
        let same = original
            .bytes()
            .zip(suggested.bytes())
            .take_while(|(a, b)| a == b)
            .count();
        let at = original[..same]
            .rfind('\n')
            .map_or(0, |newline| newline + 1);
        let changed = format!(
            "{}part shoe_shelf : oak_19 {{\n  length: 300mm\n}}\n\n{}",
            &original[..at],
            &original[at..]
        );
        files.insert(relative("carcass.stave"), changed.into_bytes());
        history
            .save(&files, &person(), "The person's shelf")
            .unwrap()
    };
    let result = connection
        .call("apply_suggestion", json!({ "id": id }))
        .await;
    assert!(is_error(&result), "{}", text(&result));
    let words = text(&result);
    assert!(words.contains("clashes in carcass.stave"), "{words}");
    let history = History::open(&folder).unwrap();
    assert_eq!(history.main().unwrap(), saved, "main didn't move");
    let suggestions = history.suggestions().unwrap();
    assert_eq!(suggestions.len(), 1, "the suggestion stays");
    assert_eq!(suggestions[0].base, saved, "moved onto main");
    assert_eq!(suggestions[0].conflicted, [relative("carcass.stave")]);
    let listed = connection.call("list_suggestions", json!({})).await;
    assert!(text(&listed).contains("clashes"), "{}", text(&listed));
}

#[tokio::test(flavor = "multi_thread")]
async fn undo_reverts_own_apply() {
    let (folder, connection) = opened_with_versions("undo_reverts_own_apply").await;
    let before = History::open(&folder).unwrap().main().unwrap();
    let id = suggest_shoe_shelf(&connection).await;
    let applied = connection
        .call("apply_suggestion", json!({ "id": id }))
        .await;
    assert!(!is_error(&applied), "{}", text(&applied));
    let undone = connection.call("undo", json!({})).await;
    assert!(!is_error(&undone), "{}", text(&undone));
    assert!(
        text(&undone).contains("Add a shoe shelf"),
        "{}",
        text(&undone)
    );
    let history = History::open(&folder).unwrap();
    assert_eq!(history.main().unwrap(), before);
    // The applied suggestion is waiting again.
    let waiting = history.suggestions().unwrap();
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0].id.as_str(), id);

    // Discarding a suggestion leaves main as it is.
    let discarded = connection
        .call("discard_suggestion", json!({ "id": id }))
        .await;
    assert!(!is_error(&discarded), "{}", text(&discarded));
    let history = History::open(&folder).unwrap();
    assert!(history.suggestions().unwrap().is_empty());
    assert_eq!(history.main().unwrap(), before);
}

#[tokio::test(flavor = "multi_thread")]
async fn undo_refused_after_the_persons_later_save() {
    let (folder, connection) = opened_with_versions("undo_refused_after_later_save").await;
    let id = suggest_shoe_shelf(&connection).await;
    let applied = connection
        .call("apply_suggestion", json!({ "id": id }))
        .await;
    assert!(!is_error(&applied), "{}", text(&applied));
    let saved = person_saves(
        &folder,
        "cabinet.stave",
        "param cabinet_width  = 900mm",
        "param cabinet_width  = 950mm",
    );
    let undone = connection.call("undo", json!({})).await;
    assert!(is_error(&undone), "{}", text(&undone));
    assert!(text(&undone).contains("Ann"), "{}", text(&undone));
    assert_eq!(History::open(&folder).unwrap().main().unwrap(), saved);
}
