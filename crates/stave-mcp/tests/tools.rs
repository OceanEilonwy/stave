//! The tools, called as an AI app calls them, on copies of `examples/`.

mod common;

use std::path::Path;

use common::{connect, copy_of, example_copy, is_error, options, structured, text};
use serde_json::json;
use stave_lang::{Severity, render_diagnostics};
use stave_mcp::{McpError, OpenProject, project_path};

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

const THROUGH_DIVIDER: &str = r#"patch "Add a shelf through the divider" {
  add part shelf_3 : oak_19 {
    length: fit
    width:  cabinet_depth - back.thickness
    place {
      against(base.top, gap: drawer_zone - 10mm)
      center(base, across)
      flush(side_l.front)
    }
  } in "carcass.stave"
  add joint shelf_3_dados : dado(shelf_3, into: [side_l, side_r], depth: 9mm)
}
"#;

fn path_text(path: &Path) -> String {
    path.display().to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn open_project_summarises() {
    let folder = example_copy("clothing_cabinet", "open_project_summarises");
    let opened = stave_app::open(&folder).unwrap();
    let parts = opened.design.as_ref().unwrap().parts().len();
    let connection = connect(options("open_project_summarises")).await;
    let result = connection
        .call("open_project", json!({ "path": path_text(&folder) }))
        .await;
    assert!(!is_error(&result), "{}", text(&result));
    let summary = structured(&result);
    assert_eq!(summary["parts"], json!(parts));
    assert_eq!(summary["errors"], json!(opened.error_count()));
    assert_eq!(summary["folder"], json!(path_text(&folder)));
    let words = text(&result);
    assert!(words.contains(&format!("{parts} parts")), "{words}");
    assert!(words.contains("no errors"), "{words}");
}

#[tokio::test(flavor = "multi_thread")]
async fn open_project_refuses_missing_folder() {
    let folder = common::scratch_folder("open_project_refuses_missing_folder").join("not_there");
    let connection = connect(options("open_project_refuses_missing_folder")).await;
    let result = connection
        .call("open_project", json!({ "path": path_text(&folder) }))
        .await;
    assert!(is_error(&result));
    assert!(
        text(&result).contains("There is no folder"),
        "{}",
        text(&result)
    );
    // Nothing is open afterwards.
    let check = connection.call("check", json!({})).await;
    assert!(is_error(&check));
    assert!(
        text(&check).contains("No project is open"),
        "{}",
        text(&check)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn open_project_refuses_a_relative_path() {
    let connection = connect(options("open_project_refuses_a_relative_path")).await;
    let result = connection
        .call("open_project", json!({ "path": "examples/box" }))
        .await;
    assert!(is_error(&result));
    assert!(
        text(&result).contains("not a full path"),
        "{}",
        text(&result)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn project_folders_with_parent_steps_or_links_are_refused() {
    let root = common::scratch_folder("project_folders_with_parent_steps_or_links_are_refused")
        .canonicalize()
        .unwrap();
    let elsewhere = root.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let link = root.join("link");
    std::os::unix::fs::symlink(&elsewhere, &link).unwrap();
    let connection = connect(options(
        "project_folders_with_parent_steps_or_links_are_refused",
    ))
    .await;
    let climbing = format!("{}/../elsewhere", root.join("sub").display());
    let open = connection
        .call("open_project", json!({ "path": climbing }))
        .await;
    assert!(is_error(&open));
    assert!(text(&open).contains("'..'"), "{}", text(&open));
    let steps = "new \"Box\" 300mm 200mm 200mm\ncreate\n";
    let made = connection
        .call("new_project", json!({ "folder": climbing, "steps": steps }))
        .await;
    assert!(is_error(&made));
    assert!(text(&made).contains("'..'"), "{}", text(&made));
    let linked = connection
        .call(
            "new_project",
            json!({ "folder": path_text(&link), "steps": steps }),
        )
        .await;
    assert!(is_error(&linked));
    assert!(text(&linked).contains("is a link"), "{}", text(&linked));
    assert_eq!(std::fs::read_dir(&elsewhere).unwrap().count(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn check_matches_stave_check_output() {
    let folder = copy_of(
        "tests/broken/double_doors_no_gap",
        "check_matches_stave_check_output",
    );
    let opened = stave_app::open(&folder).unwrap();
    let count = |severity: Severity| {
        opened
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == severity)
            .count()
    };
    let (errors, warnings) = (count(Severity::Error), count(Severity::Warning));
    assert!(errors > 0, "the broken project has errors");
    let plural =
        |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    let expected = format!(
        "{}checked {}: {}, {}\n",
        render_diagnostics(&opened.sources, &opened.diagnostics),
        plural(opened.root_files, "file", "files"),
        plural(errors, "error", "errors"),
        plural(warnings, "warning", "warnings"),
    );
    let connection = connect(options("check_matches_stave_check_output")).await;
    connection
        .call("open_project", json!({ "path": path_text(&folder) }))
        .await;
    let result = connection.call("check", json!({})).await;
    assert!(!is_error(&result));
    assert_eq!(text(&result), expected);
    let problems = structured(&result)["problems"].as_array().unwrap();
    assert_eq!(problems.len(), opened.diagnostics.len());
    assert_eq!(problems[0]["number"], json!(1));
    assert_eq!(structured(&result)["errors"], json!(errors));
}

#[tokio::test(flavor = "multi_thread")]
async fn fixes_for_gives_patches() {
    let folder = copy_of(
        "tests/broken/double_doors_no_gap",
        "fixes_for_gives_patches",
    );
    let opened = stave_app::open(&folder).unwrap();
    let (number, diagnostic) = opened
        .diagnostics
        .iter()
        .enumerate()
        .find(|(_, diagnostic)| !diagnostic.fixes.is_empty())
        .expect("a problem with a fix");
    let connection = connect(options("fixes_for_gives_patches")).await;
    connection
        .call("open_project", json!({ "path": path_text(&folder) }))
        .await;
    let result = connection
        .call("fixes_for", json!({ "problem": number + 1 }))
        .await;
    assert!(!is_error(&result), "{}", text(&result));
    let fixes = structured(&result)["fixes"].as_array().unwrap();
    assert_eq!(fixes.len(), diagnostic.fixes.len());
    let first = diagnostic.fixes[0].to_string();
    assert_eq!(fixes[0]["patch"], json!(first));
    assert!(text(&result).contains(&first), "{}", text(&result));
    // A number past the end is refused.
    let past = connection
        .call(
            "fixes_for",
            json!({ "problem": opened.diagnostics.len() + 1 }),
        )
        .await;
    assert!(is_error(&past));
    assert!(
        text(&past).contains("There is no problem"),
        "{}",
        text(&past)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn try_patch_fits_and_writes_nothing() {
    let folder = example_copy("clothing_cabinet", "try_patch_fits_and_writes_nothing");
    let before = common::snapshot(&folder);
    let connection = connect(options("try_patch_fits_and_writes_nothing")).await;
    connection
        .call("open_project", json!({ "path": path_text(&folder) }))
        .await;
    let result = connection
        .call("try_patch", json!({ "patch": SHOE_SHELF }))
        .await;
    assert!(!is_error(&result), "{}", text(&result));
    let words = text(&result);
    assert!(
        words.contains("patch \"Add a shoe shelf\": fits"),
        "{words}"
    );
    assert!(words.contains("would change carcass.stave"), "{words}");
    let tried = &structured(&result)["patches"][0];
    assert_eq!(tried["verdict"], json!("fits"));
    assert_eq!(tried["changed"], json!(["carcass.stave"]));
    assert_eq!(structured(&result)["fits"], json!(true));
    // A patch file with its `stave 1` line reads the same.
    let with_header = connection
        .call(
            "try_patch",
            json!({ "patch": format!("stave 1\n\n{SHOE_SHELF}") }),
        )
        .await;
    assert_eq!(structured(&with_header)["fits"], json!(true));
    assert_eq!(common::snapshot(&folder), before, "nothing was written");
}

#[tokio::test(flavor = "multi_thread")]
async fn try_patch_refused_lists_diagnostics() {
    let folder = example_copy("clothing_cabinet", "try_patch_refused_lists_diagnostics");
    let before = common::snapshot(&folder);
    let connection = connect(options("try_patch_refused_lists_diagnostics")).await;
    connection
        .call("open_project", json!({ "path": path_text(&folder) }))
        .await;
    let patches = format!("{SHOE_SHELF}{THROUGH_DIVIDER}");
    let result = connection
        .call("try_patch", json!({ "patch": patches }))
        .await;
    assert!(!is_error(&result), "{}", text(&result));
    let words = text(&result);
    assert!(
        words.contains("patch \"Add a shoe shelf\": fits"),
        "{words}"
    );
    assert!(
        words.contains("patch \"Add a shelf through the divider\": refused"),
        "{words}"
    );
    assert!(words.contains("error["), "{words}");
    let tried = structured(&result)["patches"].as_array().unwrap();
    assert_eq!(tried.len(), 2);
    assert_eq!(tried[1]["verdict"], json!("refused"));
    let diagnostics = tried[1]["problems"].as_array().unwrap();
    assert!(!diagnostics.is_empty());
    assert!(diagnostics.iter().any(|d| d["severity"] == json!("error")));
    assert_eq!(structured(&result)["fits"], json!(false));
    assert_eq!(common::snapshot(&folder), before, "nothing was written");
}

#[tokio::test(flavor = "multi_thread")]
async fn new_project_refuses_non_empty_folder() {
    let folder = common::scratch_folder("new_project_refuses_non_empty_folder")
        .canonicalize()
        .unwrap();
    std::fs::write(folder.join("notes.txt"), "keep me").unwrap();
    let connection = connect(options("new_project_refuses_non_empty_folder")).await;
    let result = connection
        .call(
            "new_project",
            json!({ "folder": path_text(&folder), "steps": "new \"Box\" 300mm 200mm 200mm\ncreate\n" }),
        )
        .await;
    assert!(is_error(&result));
    assert!(text(&result).contains("isn't empty"), "{}", text(&result));
    assert_eq!(
        std::fs::read_dir(&folder).unwrap().count(),
        1,
        "nothing was written"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn new_project_from_steps_checks() {
    let folder = common::scratch_folder("new_project_from_steps_checks")
        .canonicalize()
        .unwrap()
        .join("side_table");
    let steps = "\
new \"Side table\" 500mm 600mm 500mm
import std:frames
stock oak_19 white_oak solid 19mm max_width 200mm max_length 2700mm
create
select region envelope
choose frames.Top
set top_stock = oak_19
add
";
    let connection = connect(options("new_project_from_steps_checks")).await;
    let result = connection
        .call(
            "new_project",
            json!({ "folder": path_text(&folder), "steps": steps }),
        )
        .await;
    assert!(!is_error(&result), "{}", text(&result));
    let opened = stave_app::open(&folder).unwrap();
    assert_eq!(opened.error_count(), 0);
    assert!(!opened.design.as_ref().unwrap().parts().is_empty());
    assert_eq!(structured(&result)["errors"], json!(0));
    assert!(text(&result).contains("> create"), "{}", text(&result));
    // The new project is the open one.
    let check = connection.call("check", json!({})).await;
    assert!(!is_error(&check), "{}", text(&check));
}

#[tokio::test(flavor = "multi_thread")]
async fn new_project_reports_a_failed_step() {
    let folder = common::scratch_folder("new_project_reports_a_failed_step")
        .canonicalize()
        .unwrap()
        .join("chest");
    let connection = connect(options("new_project_reports_a_failed_step")).await;
    let result = connection
        .call(
            "new_project",
            json!({ "folder": path_text(&folder), "steps": "create\n" }),
        )
        .await;
    assert!(is_error(&result));
    assert!(
        text(&result).contains("start a new project first"),
        "{}",
        text(&result)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn new_project_without_create_leaves_nothing() {
    let root = common::scratch_folder("new_project_without_create_leaves_nothing")
        .canonicalize()
        .unwrap();
    // Two folders that don't exist yet: both are made, and both go.
    let folder = root.join("projects").join("box");
    let connection = connect(options("new_project_without_create_leaves_nothing")).await;
    let result = connection
        .call(
            "new_project",
            json!({ "folder": path_text(&folder), "steps": "new \"Box\" 300mm 200mm 200mm\n" }),
        )
        .await;
    assert!(is_error(&result));
    assert!(
        text(&result).contains("never reached 'create'"),
        "{}",
        text(&result)
    );
    assert_eq!(
        std::fs::read_dir(&root).unwrap().count(),
        0,
        "no folder is left behind"
    );
    // An empty folder that was already there stays, empty.
    let existing = root.join("existing");
    std::fs::create_dir(&existing).unwrap();
    let again = connection
        .call(
            "new_project",
            json!({ "folder": path_text(&existing), "steps": "new \"Box\" 300mm 200mm 200mm\n" }),
        )
        .await;
    assert!(is_error(&again));
    assert_eq!(std::fs::read_dir(&existing).unwrap().count(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn new_project_failing_removes_the_folders_it_made() {
    let root = common::scratch_folder("new_project_failing_removes_the_folders_it_made")
        .canonicalize()
        .unwrap();
    let folder = root.join("a").join("b").join("chest");
    let connection = connect(options("new_project_failing_removes_the_folders_it_made")).await;
    let result = connection
        .call(
            "new_project",
            json!({ "folder": path_text(&folder), "steps": "create\n" }),
        )
        .await;
    assert!(is_error(&result));
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn try_patch_refuses_paths_outside_the_project() {
    let folder = example_copy(
        "clothing_cabinet",
        "try_patch_refuses_paths_outside_the_project",
    );
    // The copy's own scratch folder: `..` from the project lands here.
    let root = folder.parent().unwrap().to_path_buf();
    let outside = std::env::temp_dir().join(format!(
        "stave-mcp-test-{}-outside.stave",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&outside);
    let before = common::snapshot(&root);
    let connection = connect(options("try_patch_refuses_paths_outside_the_project")).await;
    connection
        .call("open_project", json!({ "path": path_text(&folder) }))
        .await;
    for target in ["../outside.stave".to_owned(), path_text(&outside)] {
        let patch = SHOE_SHELF.replace("in \"carcass.stave\"", &format!("in \"{target}\""));
        let result = connection
            .call("try_patch", json!({ "patch": patch }))
            .await;
        assert!(!is_error(&result), "{}", text(&result));
        let tried = &structured(&result)["patches"][0];
        assert_eq!(tried["verdict"], json!("refused"), "{}", text(&result));
        assert_eq!(structured(&result)["fits"], json!(false));
        let words = text(&result);
        assert!(
            words.contains(&format!(
                "'{target}' is not a file name in the project's folder"
            )),
            "{words}"
        );
    }
    assert_eq!(common::snapshot(&root), before, "nothing was written");
    assert!(!root.join("outside.stave").exists());
    assert!(!outside.exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn grammar_returns_section() {
    let connection = connect(options("grammar_returns_section")).await;
    let result = connection
        .call("grammar", json!({ "topic": "Patches" }))
        .await;
    assert!(!is_error(&result), "{}", text(&result));
    assert!(text(&result).starts_with("## Patches"), "{}", text(&result));
    assert!(text(&result).contains("patch_op"), "{}", text(&result));
    // Without a topic: the sections.
    let contents = connection.call("grammar", json!({})).await;
    assert!(text(&contents).contains("Patches"), "{}", text(&contents));
    let missing = connection
        .call("grammar", json!({ "topic": "Teapots" }))
        .await;
    assert!(is_error(&missing));
    assert!(
        text(&missing).contains("Patches"),
        "the refusal lists the sections"
    );
}

/// The name of the first declaration starting with `word` (`kind`,
/// `profile`) in `std/<module>`, read at test time so the test names none
/// itself.
fn first_std_declaration(module: &str, word: &str) -> String {
    let folder = common::workspace().join("std").join(module);
    let mut files: Vec<_> = std::fs::read_dir(folder)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    files.sort();
    let prefix = format!("{word} ");
    for file in files {
        let text = std::fs::read_to_string(file).unwrap();
        for line in text.lines() {
            let line = line.strip_prefix("pub ").unwrap_or(line);
            if let Some(rest) = line.strip_prefix(&prefix) {
                return rest.split([' ', ':', '{']).next().unwrap().to_owned();
            }
        }
    }
    panic!("std/{module} declares no {word}");
}

#[tokio::test(flavor = "multi_thread")]
async fn std_lookup_finds_kind() {
    let kind = first_std_declaration("joinery", "kind");
    let connection = connect(options("std_lookup_finds_kind")).await;
    let result = connection.call("std_lookup", json!({ "kind": kind })).await;
    assert!(!is_error(&result), "{}", text(&result));
    let words = text(&result);
    assert!(words.contains(&format!("kind {kind} :")), "{words}");
    assert!(words.contains("std:joinery"), "{words}");
    // joinery is in the prelude: no import is suggested.
    assert!(words.contains("without an import"), "{words}");
    assert!(!words.contains("import \""), "{words}");
    assert!(
        words.trim_end().ends_with('}'),
        "the whole declaration: {words}"
    );
    assert_eq!(structured(&result)["module"], json!("joinery"));
    let missing = connection
        .call("std_lookup", json!({ "kind": "no_such_kind_at_all" }))
        .await;
    assert!(is_error(&missing));
}

#[tokio::test(flavor = "multi_thread")]
async fn std_lookup_tells_the_import_for_a_module_outside_the_prelude() {
    let profile = first_std_declaration("casework", "profile");
    let connection = connect(options(
        "std_lookup_tells_the_import_for_a_module_outside_the_prelude",
    ))
    .await;
    let result = connection
        .call("std_lookup", json!({ "kind": profile }))
        .await;
    assert!(!is_error(&result), "{}", text(&result));
    let words = text(&result);
    // The import as the grammar writes it: `import STRING`.
    assert!(words.contains("`import \"std:casework\"`"), "{words}");
    assert_eq!(structured(&result)["module"], json!("casework"));
}

#[tokio::test(flavor = "multi_thread")]
async fn describe_tells_the_tree_and_one_part() {
    let folder = example_copy("box", "describe_tells_the_tree_and_one_part");
    let opened = stave_app::open(&folder).unwrap();
    let part = opened.design.as_ref().unwrap().parts()[0].name.clone();
    let connection = connect(options("describe_tells_the_tree_and_one_part")).await;
    connection
        .call("open_project", json!({ "path": path_text(&folder) }))
        .await;
    let tree = connection.call("describe", json!({})).await;
    assert!(!is_error(&tree), "{}", text(&tree));
    assert!(text(&tree).contains(&part), "{}", text(&tree));
    let one = connection.call("describe", json!({ "path": part })).await;
    assert!(!is_error(&one), "{}", text(&one));
    assert!(text(&one).contains("Cut size"), "{}", text(&one));
    let missing = connection
        .call("describe", json!({ "path": "no_such_part" }))
        .await;
    assert!(is_error(&missing));
}

#[tokio::test(flavor = "multi_thread")]
async fn list_projects_reads_the_window_list() {
    let options = options("list_projects_reads_the_window_list");
    let project = example_copy("box", "list_projects_reads_the_window_list");
    let gone = project.with_file_name("gone");
    let other_app = project.with_file_name("other");
    std::fs::create_dir_all(&other_app).unwrap();
    let bookmark = |path: &Path, app: &str, modified: &str| {
        format!(
            r#"<bookmark href="file://{}" added="{modified}" modified="{modified}" visited="{modified}">
  <info><metadata owner="http://freedesktop.org">
    <mime:mime-type type="inode/directory"/>
    <bookmark:applications>
      <bookmark:application name="{app}" exec="'stave-gtk %u'" modified="{modified}" count="1"/>
    </bookmark:applications>
  </metadata></info>
</bookmark>"#,
            path.display()
        )
    };
    let xbel = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<xbel version="1.0" xmlns:bookmark="http://www.freedesktop.org/standards/desktop-bookmarks" xmlns:mime="http://www.freedesktop.org/standards/shared-mime-info">
{}
{}
{}
</xbel>
"#,
        bookmark(&project, "Stave", "2026-11-19T10:00:00.000000Z"),
        bookmark(&gone, "Stave", "2026-11-20T10:00:00.000000Z"),
        bookmark(&other_app, "Files", "2026-11-20T11:00:00.000000Z"),
    );
    std::fs::create_dir_all(options.recent_projects.parent().unwrap()).unwrap();
    std::fs::write(&options.recent_projects, xbel).unwrap();
    let connection = connect(options).await;
    let result = connection.call("list_projects", json!({})).await;
    assert!(!is_error(&result), "{}", text(&result));
    assert_eq!(
        structured(&result)["projects"],
        json!([{ "folder": path_text(&project), "name": "box" }])
    );
    assert!(
        text(&result).contains(&path_text(&project)),
        "{}",
        text(&result)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn list_projects_with_no_list_says_so() {
    let connection = connect(options("list_projects_with_no_list_says_so")).await;
    let result = connection.call("list_projects", json!({})).await;
    assert!(!is_error(&result), "{}", text(&result));
    assert_eq!(structured(&result)["projects"], json!([]));
}

#[test]
fn project_path_rejects_escape() {
    let root = common::scratch_folder("project_path_rejects_escape")
        .canonicalize()
        .unwrap();
    let folder = root.join("project");
    let outside = root.join("outside");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, folder.join("link")).unwrap();
    let open = OpenProject {
        folder: folder.clone(),
    };
    for escape in [
        "../outside/x.stave",
        "sub/../../outside",
        "/etc/passwd",
        "link/x.stave",
        "link",
    ] {
        assert!(
            matches!(
                project_path(&open, escape),
                Err(McpError::OutsideProject(_))
            ),
            "{escape} is refused"
        );
    }
    assert_eq!(
        project_path(&open, "carcass.stave").unwrap(),
        folder.join("carcass.stave")
    );
    assert_eq!(
        project_path(&open, "./hardware/hinges.stave").unwrap(),
        folder.join("hardware/hinges.stave")
    );
}
