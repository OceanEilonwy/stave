//! `stave edit` driving the whole edit loop with no window: a selection
//! named on the command line, its menu with each entry's applicability,
//! a chosen entry filled in and committed, and a script of such steps. On
//! a copy of `examples/clothing_cabinet_assemblies` made in a temporary
//! folder (the example is read, never written).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A fresh temporary folder holding a copy of the assemblies cabinet as
/// `cabinet/` (its `hardware` module included).
fn copy_of_the_cabinet(name: &str) -> PathBuf {
    let folder =
        std::env::temp_dir().join(format!("stave-edit-loop-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    let mut folders = vec![(
        workspace().join("examples/clothing_cabinet_assemblies"),
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

/// Every file under `folder` with its bytes.
fn snapshot(folder: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut folders = vec![folder.to_path_buf()];
    while let Some(folder) = folders.pop() {
        for entry in std::fs::read_dir(&folder).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                folders.push(path);
            } else {
                out.insert(path.clone(), std::fs::read(&path).unwrap());
            }
        }
    }
    out
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

#[test]
fn the_menu_for_the_space_above_the_divider_lists_the_shelf_and_why_a_drawer_cant_go() {
    let folder = copy_of_the_cabinet("menu");
    let before = snapshot(&folder);
    let output = stave(&folder, &["edit", "cabinet", "--region", "divider.above"]);
    assert_eq!(output.status.code(), Some(0), "{}", stdout(&output));
    let printed = stdout(&output);
    assert!(
        printed.starts_with("In this space: divider.above\n5 menu entries\n"),
        "{printed}"
    );
    assert!(
        printed.contains("\n  3. casework.Shelf             needs a choice: sides\n"),
        "{printed}"
    );
    // A divider is not offered inside a divider's own space. A drawer
    // would run into the shelf and the doors: refused, with the checker's
    // messages, after the entries that fit.
    assert!(
        printed.contains(
            "\n  5. casework.Drawer            refused\n       error[overlap]: shelf and drawer_2.side[left] overlap\n"
        ),
        "{printed}"
    );
    // The same every run, and nothing written.
    let again = stave(&folder, &["edit", "cabinet", "--region", "divider.above"]);
    assert_eq!(stdout(&again), printed);
    assert_eq!(snapshot(&folder), before);
}

#[test]
fn a_door_stile_s_face_offers_the_knobs() {
    let folder = copy_of_the_cabinet("face");
    let output = stave(
        &folder,
        &["edit", "cabinet", "--face", "door_l.stile[hinge].front"],
    );
    assert_eq!(output.status.code(), Some(0), "{}", stdout(&output));
    let printed = stdout(&output);
    assert!(
        printed.starts_with("On this face: door_l.stile[hinge].front\n"),
        "{printed}"
    );
    assert!(printed.contains(". hw.brass_knob_26 "), "{printed}");
    assert!(printed.contains(". hardware.knob "), "{printed}");
}

#[test]
fn choosing_the_shelf_and_committing_gives_a_design_that_checks_clean() {
    let folder = copy_of_the_cabinet("commit");
    let output = stave(
        &folder,
        &[
            "edit",
            "cabinet",
            "--region",
            "divider.above",
            "--choose",
            "Shelf",
            "--bind",
            "parts side_l side_r",
            "--set",
            "headroom=400mm",
            "--commit",
        ],
    );
    assert_eq!(output.status.code(), Some(0), "{}", stdout(&output));
    assert_eq!(
        stdout(&output),
        "In this space: divider.above
5 menu entries
casework.Shelf as shelf_2 (In this space)
  space        divider.above
  sides        [side_l, side_r]
  panel_stock  oak_19 (copied from shelf)
  headroom     400mm
  joinery      dado (default)
  depth        9mm (default)
verdict: fits
patch:
  stave 1

  patch \"Add a shelf\" {
    add assembly shelf_2 : casework.Shelf {
      space:       divider.above
      sides:       [side_l, side_r]
      panel_stock: oak_19
      headroom:    400mm
    } in \"carcass.stave\"
  }
wrote cabinet/carcass.stave
committed: the design checks with 0 errors
"
    );
    let check = stave(&folder, &["check", "cabinet"]);
    let stderr = String::from_utf8_lossy(&check.stderr);
    assert!(check.status.success(), "{stderr}");
    assert!(stderr.ends_with(": 0 errors, 0 warnings\n"), "{stderr}");
    let carcass = std::fs::read_to_string(folder.join("cabinet/carcass.stave")).unwrap();
    assert!(
        carcass.contains("assembly shelf_2 : casework.Shelf {\n"),
        "{carcass}"
    );
    // Re-opened, the design has one more shelf.
    let dump = stave(&folder, &["dump", "cabinet"]);
    assert!(
        stdout(&dump).contains("\nshelf_2.board "),
        "{}",
        stdout(&dump)
    );
}

#[test]
fn a_candidate_still_to_point_at_is_not_committed() {
    let folder = copy_of_the_cabinet("incomplete");
    let before = snapshot(&folder);
    let output = stave(
        &folder,
        &[
            "edit",
            "cabinet",
            "--region",
            "divider.above",
            "--choose",
            "Shelf",
            "--commit",
        ],
    );
    assert_eq!(output.status.code(), Some(1), "{}", stdout(&output));
    let printed = stdout(&output);
    assert!(
        printed.contains("\n  sides        point at it\n"),
        "{printed}"
    );
    assert!(
        printed.contains("\nverdict: needs a choice: sides\n"),
        "{printed}"
    );
    assert!(
        printed.ends_with("not committed: the edit doesn't fit, so nothing was written\n"),
        "{printed}"
    );
    assert_eq!(snapshot(&folder), before);
}

#[test]
fn a_script_runs_the_loop_and_a_cancel_leaves_the_files_as_they_were() {
    let folder = copy_of_the_cabinet("script");
    std::fs::write(
        folder.join("steps.txt"),
        "# a shelf too low for its space, then given up
select region divider.above
choose casework.Shelf
bind parts side_l side_r
set headroom = 1200mm   # lower than the space is tall
cancel
",
    )
    .unwrap();
    let before = snapshot(&folder);
    let output = stave(&folder, &["edit", "cabinet", "--script", "steps.txt"]);
    assert_eq!(output.status.code(), Some(0), "{}", stdout(&output));
    assert_eq!(
        stdout(&output),
        "> select region divider.above
In this space: divider.above
5 menu entries
> choose casework.Shelf
needs a choice: sides
> bind parts side_l side_r
fits
> set headroom = 1200mm
refused: 1 error
> cancel
cancelled: nothing was written
"
    );
    assert_eq!(snapshot(&folder), before);
}

#[test]
fn a_script_stops_at_the_first_step_that_fails() {
    let folder = copy_of_the_cabinet("fails");
    std::fs::write(
        folder.join("steps.txt"),
        "select region divider.above\nchoose Shelf\nset headroom = 1200mm\nbind parts side_l side_r\nshow\ncommit\nlist\n",
    )
    .unwrap();
    let before = snapshot(&folder);
    let output = stave(&folder, &["edit", "cabinet", "--script", "steps.txt"]);
    assert_eq!(output.status.code(), Some(1), "{}", stdout(&output));
    let printed = stdout(&output);
    assert!(
        printed.contains(
            "\nverdict: refused: 1 error\nerror[require]: 'shelf_2.shelf_inside' fails: the shelf is lower than the bottom of its space; make the headroom smaller\n"
        ),
        "{printed}"
    );
    assert!(
        printed
            .ends_with("> commit\nnot committed: the edit doesn't fit, so nothing was written\n"),
        "{printed}"
    );
    assert_eq!(snapshot(&folder), before);

    // A name that isn't in the design fails the step, in words.
    std::fs::write(folder.join("bad.txt"), "select region nowhere\nlist\n").unwrap();
    let output = stave(&folder, &["edit", "cabinet", "--script", "bad.txt"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        stdout(&output),
        "> select region nowhere\nerror: the design has no region called 'nowhere'\n"
    );
    // A line that isn't a step is an error before anything runs.
    std::fs::write(
        folder.join("worse.txt"),
        "select region divider.above\npaint it\n",
    )
    .unwrap();
    let output = stave(&folder, &["edit", "cabinet", "--script", "worse.txt"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("line 2: 'paint it' is not a step"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
