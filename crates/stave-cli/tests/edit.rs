//! `stave edit`, the harness for the editor: a patch file tried on a copy
//! of `examples/clothing_cabinet` made in a temporary folder, each patch
//! in order, stopping at the first refusal.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A fresh temporary folder holding a copy of the clothing cabinet as
/// `cabinet/` (its `hardware` module included).
fn copy_of_the_cabinet(name: &str) -> PathBuf {
    let folder = std::env::temp_dir().join(format!("stave-edit-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    let mut folders = vec![(
        workspace().join("examples/clothing_cabinet"),
        folder.join("cabinet"),
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
    folder
}

/// Runs `stave` in `folder`, so paths in its output are relative to it.
fn stave(folder: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_stave"))
        .args(args)
        .current_dir(folder)
        .output()
        .unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn read(folder: &Path, file: &str) -> String {
    std::fs::read_to_string(folder.join(file)).unwrap()
}

const ADD_SHELF: &str = r#"patch "Add a shelf" {
  add part shelf_2 : oak_19 {
    length: fit
    width:  cabinet_depth - back.thickness
    place {
      against(divider.top, gap: 400mm)
      center(base, across)
      flush(side_l.front)
    }
  } in "carcass.stave"
  add joint shelf_2_dados : dado(shelf_2, into: [side_l, side_r], depth: 9mm)
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

const WIDER: &str = "patch \"Wider\" {\n  set cabinet_width = 950mm\n}\n";

/// Writes a patch file of these patches as `name` in `folder`.
fn patch_file(folder: &Path, name: &str, patches: &[&str]) {
    let text = format!("stave 1\n\n{}", patches.join("\n"));
    std::fs::write(folder.join(name), text).unwrap();
}

#[test]
fn a_shelf_fits_and_commits_and_the_project_checks_clean() {
    let folder = copy_of_the_cabinet("fits");
    patch_file(&folder, "shelf.stave", &[ADD_SHELF]);
    let before = read(&folder, "cabinet/carcass.stave");

    let output = stave(&folder, &["edit", "cabinet", "--patch", "shelf.stave"]);
    assert_eq!(output.status.code(), Some(0), "{}", stdout(&output));
    assert_eq!(
        stdout(&output),
        "patch \"Add a shelf\": fits\n  changes cabinet/carcass.stave\nchecked 1 of 1 patch: 1 fits, 0 refused\n"
    );
    assert_eq!(read(&folder, "cabinet/carcass.stave"), before);
    // The same every run.
    let again = stave(&folder, &["edit", "cabinet", "--patch", "shelf.stave"]);
    assert_eq!(stdout(&again), stdout(&output));

    let output = stave(
        &folder,
        &["edit", "cabinet", "--patch", "shelf.stave", "--commit"],
    );
    assert_eq!(output.status.code(), Some(0), "{}", stdout(&output));
    assert_eq!(
        stdout(&output),
        "patch \"Add a shelf\": fits\n  wrote cabinet/carcass.stave\nchecked 1 of 1 patch: 1 fits, 0 refused, wrote 1 file\n"
    );
    let after = read(&folder, "cabinet/carcass.stave");
    assert!(after.contains("part shelf_2 : oak_19 {\n"), "{after}");
    assert!(
        after.contains("joint shelf_2_dados : dado(shelf_2"),
        "{after}"
    );
    // Everything else is as it was, comments included.
    assert!(after.contains("    against(base.top)            // stands on the base\n"));

    let check = stave(&folder, &["check", "cabinet"]);
    let stderr = String::from_utf8_lossy(&check.stderr);
    assert!(check.status.success(), "{stderr}");
    assert!(stderr.ends_with(": 0 errors, 0 warnings\n"), "{stderr}");
}

#[test]
fn a_part_through_another_is_refused_and_nothing_is_written() {
    let folder = copy_of_the_cabinet("refused");
    patch_file(&folder, "through.stave", &[THROUGH_DIVIDER]);
    let before = read(&folder, "cabinet/carcass.stave");

    let output = stave(
        &folder,
        &["edit", "cabinet", "--patch", "through.stave", "--commit"],
    );
    assert_eq!(output.status.code(), Some(1));
    let printed = stdout(&output);
    assert!(
        printed.starts_with(
            "patch \"Add a shelf through the divider\": refused\nerror[overlap]: divider and shelf_3 overlap\n  --> cabinet/carcass.stave:89:6\n"
        ),
        "{printed}"
    );
    assert!(printed.contains("\n   = help: "), "{printed}");
    assert!(
        printed.ends_with("\nchecked 1 of 1 patch: 0 fit, 1 refused, wrote 0 files\n"),
        "{printed}"
    );
    assert_eq!(read(&folder, "cabinet/carcass.stave"), before);
}

#[test]
fn patches_apply_in_order_and_stop_at_the_first_refusal() {
    let folder = copy_of_the_cabinet("order");
    patch_file(&folder, "three.stave", &[ADD_SHELF, THROUGH_DIVIDER, WIDER]);
    let output = stave(
        &folder,
        &["edit", "cabinet", "--patch", "three.stave", "--commit"],
    );
    assert_eq!(output.status.code(), Some(1));
    let printed = stdout(&output);
    assert!(
        printed.starts_with(
            "patch \"Add a shelf\": fits\n  wrote cabinet/carcass.stave\npatch \"Add a shelf through the divider\": refused\nerror[overlap]: "
        ),
        "{printed}"
    );
    // Line numbers are those of the file after the first patch.
    assert!(
        printed.contains("  --> cabinet/carcass.stave:99:6\n"),
        "{printed}"
    );
    assert!(!printed.contains("Wider"), "{printed}");
    assert!(
        printed.ends_with("\nchecked 2 of 3 patches: 1 fits, 1 refused, wrote 1 file\n"),
        "{printed}"
    );
    // The first patch was written; the third was never tried.
    assert!(read(&folder, "cabinet/carcass.stave").contains("part shelf_2 "));
    assert!(!read(&folder, "cabinet/carcass.stave").contains("shelf_3"));
    assert!(read(&folder, "cabinet/cabinet.stave").contains("cabinet_width  = 900mm"));

    // Each patch is tried on top of the ones before it.
    patch_file(&folder, "twice.stave", &[WIDER, WIDER]);
    let output = stave(&folder, &["edit", "cabinet", "--patch", "twice.stave"]);
    assert_eq!(
        stdout(&output),
        "patch \"Wider\": fits\n  changes cabinet/cabinet.stave\npatch \"Wider\": fits\n  changes nothing\nchecked 2 of 2 patches: 2 fit, 0 refused\n"
    );
}

#[test]
fn a_patch_that_cannot_be_applied_says_why() {
    let folder = copy_of_the_cabinet("unapplied");
    patch_file(
        &folder,
        "unknown.stave",
        &["patch \"Taller\" {\n  set cabinet_hieght = 1800mm\n}\n"],
    );
    let output = stave(&folder, &["edit", "cabinet", "--patch", "unknown.stave"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        stdout(&output),
        "patch \"Taller\": refused\nerror[unknown-name]: there is no item named 'cabinet_hieght' in the project\n  --> unknown.stave:4:3\n   |\n 4 |   set cabinet_hieght = 1800mm\n   |   ^^^^^^^^^^^^^^^^^^^^^^^^^^^\n   = help: check the spelling, or add the item in an earlier operation of the patch\n\nchecked 1 of 1 patch: 0 fit, 1 refused\n"
    );

    std::fs::write(
        folder.join("broken.stave"),
        "stave 1\n\npatch \"Broken\" {\n  set = 1\n}\n",
    )
    .unwrap();
    let output = stave(&folder, &["edit", "cabinet", "--patch", "broken.stave"]);
    assert_eq!(output.status.code(), Some(1));
    let printed = stdout(&output);
    assert!(
        printed.starts_with("broken.stave: refused\nerror[syntax]: "),
        "{printed}"
    );
}

#[test]
fn edit_fails_cleanly_on_a_missing_folder_or_patch() {
    let folder = copy_of_the_cabinet("missing");
    patch_file(&folder, "wider.stave", &[WIDER]);
    let output = stave(&folder, &["edit", "no_such", "--patch", "wider.stave"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).starts_with("error: can't open 'no_such'"));
    let output = stave(&folder, &["edit", "cabinet", "--patch", "no_such.stave"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).starts_with("error: can't read 'no_such.stave'")
    );
}
