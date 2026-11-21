//! `stave edit --script` building a whole piece of furniture with no
//! window (`docs/editing.md`, "Scripts"): a new project from its sizes,
//! stock and tools, a case filling the envelope, a divider, two drawers
//! by a repeat, then changes of the design (a repeat's count, the shares
//! grip, a held distance, a rename, a removal and its dependants, the
//! mirror image of a door). In temporary folders; examples are read,
//! never written.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A fresh, empty temporary folder named for the test.
fn scratch_folder(name: &str) -> PathBuf {
    let folder =
        std::env::temp_dir().join(format!("stave-edit-script-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    folder
}

/// A copy of an example in `folder/project`.
fn copy_example(example: &str, folder: &Path) {
    let mut folders = vec![(
        workspace().join("examples").join(example),
        folder.join("project"),
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

fn checks_clean(folder: &Path, project: &str) {
    let check = stave(folder, &["check", project]);
    let stderr = String::from_utf8_lossy(&check.stderr);
    assert!(check.status.success(), "{stderr}");
    assert!(stderr.ends_with(": 0 errors, 0 warnings\n"), "{stderr}");
}

const BUILD: &str = r#"# A small chest: an empty project, a case filling its envelope, a
# divider, and two drawers below it by a repeat.
new "Small chest" 700mm 1000mm 480mm
import std:casework
stock oak_19 white_oak solid 19mm max_width 200mm max_length 2700mm
stock maple_12 hard_maple solid 12mm max_width 150mm max_length 2400mm
stock ply_6 birch plywood 6mm max_length 2440mm
tools pencil marking_knife square tape marking_gauge awl calipers tenon_saw chisel mallet router_plane clamps screwdriver router straight_bit drill depth_stop
create

select region envelope
choose casework.Case
set panel_stock = oak_19
add

select region case.inside
choose casework.Divider
bind parts case.side[left] case.side[right]
set panel_stock = oak_19
add

select region divider.below
choose casework.Drawer
set mount = overlay(left: 18mm, right: 18mm, top: -1.5mm, bottom: 8mm, meeting: -0.75mm)
set front_stock = oak_19
set box_stock = maple_12
set bottom_stock = ply_6
set slide = side_mount_slide(length: 400mm)
repeat 2 up
add
"#;

#[test]
fn a_script_builds_a_chest_from_an_empty_project_and_it_checks_clean() {
    let folder = scratch_folder("build");
    std::fs::write(folder.join("build.txt"), BUILD).unwrap();
    let output = stave(&folder, &["edit", "chest", "--script", "build.txt"]);
    let printed = stdout(&output);
    assert_eq!(output.status.code(), Some(0), "{printed}");
    assert!(
        printed.starts_with(
            "> new \"Small chest\" 700mm 1000mm 480mm
new project \"Small chest\": 700mm wide, 1000mm high, 480mm deep
> import std:casework
> stock oak_19 white_oak solid 19mm max_width 200mm max_length 2700mm
> stock maple_12 hard_maple solid 12mm max_width 150mm max_length 2400mm
> stock ply_6 birch plywood 6mm max_length 2440mm
> tools pencil marking_knife square tape marking_gauge awl calipers tenon_saw chisel mallet router_plane clamps screwdriver router straight_bit drill depth_stop
> create
created chest/small_chest.stave
the project checks with 0 errors
> select region envelope
In this space: envelope
"
        ),
        "{printed}"
    );
    assert!(
        printed.ends_with(
            "> repeat 2 up
fits
> add
wrote chest/small_chest.stave
committed: the design checks with 0 errors
"
        ),
        "{printed}"
    );
    assert_eq!(
        printed
            .matches("committed: the design checks with 0 errors")
            .count(),
        3
    );
    checks_clean(&folder, "chest");
    let text = std::fs::read_to_string(folder.join("chest/small_chest.stave")).unwrap();
    for expected in [
        "import \"std:casework\"\n",
        // The two regions are one aligned run, as `stave fmt` lays it out.
        "region envelope                = box(across: overall_width, up: overall_height, deep: overall_depth)\n",
        "region drawer_bay[bottom, top] = split(divider.below, up)\n",
        "assembly case : casework.Case {\n",
        "assembly divider : casework.Divider {\n",
        "assembly drawer[bottom, top] : casework.Drawer {\n  opening:      drawer_bay[same]\n",
    ] {
        assert!(text.contains(expected), "{expected}\n{text}");
    }
    // The same script on another empty folder prints the same.
    let again = scratch_folder("build-again");
    std::fs::write(again.join("build.txt"), BUILD).unwrap();
    let output = stave(&again, &["edit", "chest", "--script", "build.txt"]);
    assert_eq!(stdout(&output), printed);
    // A project is never created over one that is there.
    std::fs::write(
        folder.join("again.txt"),
        "new \"Small chest\" 1m 1m 1m\ncreate\n",
    )
    .unwrap();
    let output = stave(&folder, &["edit", "chest", "--script", "again.txt"]);
    assert_eq!(output.status.code(), Some(1), "{}", stdout(&output));
    assert!(
        stdout(&output).contains("changed since the edit began"),
        "{}",
        stdout(&output)
    );
}

#[test]
fn a_script_changes_a_chest_and_each_change_checks_clean() {
    let folder = scratch_folder("changes");
    copy_example("chest_of_drawers", &folder);
    std::fs::write(
        folder.join("steps.txt"),
        r#"repeat drawer 3
add
shares drawer_bay 1 300mm
show
add
hold drawer[top].front.top case.top.bottom max 40mm "the top drawer stays close under the top"
add
rename feet bun_feet
add
remove back
show
include dependants
add
"#,
    )
    .unwrap();
    let output = stave(&folder, &["edit", "project", "--script", "steps.txt"]);
    let printed = stdout(&output);
    assert_eq!(output.status.code(), Some(0), "{printed}");
    assert!(
        printed.starts_with("> repeat drawer 3\nMake 3 of drawer: fits\n> add\nwrote project/chest.stave\ncommitted: the design checks with 0 errors\n> shares drawer_bay 1 300mm\nMove the boundary between drawer_bay[bottom] and drawer_bay[middle]: fits\n"),
        "{printed}"
    );
    assert!(
        printed.contains("\n> remove back\nRemove back: used by "),
        "{printed}"
    );
    assert!(
        printed.contains("'include dependants' removes them too\n> show\nRemove back\n  used by "),
        "{printed}"
    );
    assert_eq!(
        printed
            .matches("committed: the design checks with 0 errors")
            .count(),
        5,
        "{printed}"
    );
    checks_clean(&folder, "project");
    let text = std::fs::read_to_string(folder.join("project/chest.stave")).unwrap();
    assert!(
        text.contains("assembly bun_feet : casework.Feet {"),
        "{text}"
    );
    assert!(!text.contains("assembly back"), "{text}");
}

#[test]
fn a_script_offers_and_takes_a_door_s_mirror_image() {
    let folder = scratch_folder("mirror");
    copy_example("clothing_cabinet_assemblies", &folder);
    let doors = folder.join("project/doors.stave");
    let text = std::fs::read_to_string(&doors)
        .unwrap()
        .replace("assembly door_l :", "assembly door :")
        .replace(
            "assembly door_r = mirror(door_l, about: mid(base, across))\n",
            "",
        );
    std::fs::write(&doors, text).unwrap();
    let cabinet = folder.join("project/cabinet.stave");
    let text = std::fs::read_to_string(&cabinet).unwrap().replace(
        "  pose door_l = open\n  pose door_r = open\n",
        "  pose door = open\n",
    );
    std::fs::write(&cabinet, text).unwrap();
    std::fs::write(
        folder.join("steps.txt"),
        "offer mirror door\noffer mirror divider\nmirror door\nshow\nadd\n",
    )
    .unwrap();
    let output = stave(&folder, &["edit", "project", "--script", "steps.txt"]);
    let printed = stdout(&output);
    assert_eq!(output.status.code(), Some(0), "{printed}");
    assert!(
        printed.starts_with(
            "> offer mirror door
door has a mirror image: it becomes door_left and door_right is added, mirrored about mid(base, across)
> offer mirror divider
divider has no mirror image to add
> mirror door
Add the mirror image of door: fits
> show
Add the mirror image of door
  renames the original to door_left, adds door_right mirrored about mid(base, across)
  renames in project/cabinet.stave
  renames in project/doors.stave
verdict: fits
"
        ),
        "{printed}"
    );
    assert!(
        printed.ends_with("committed: the design checks with 0 errors\n"),
        "{printed}"
    );
    checks_clean(&folder, "project");

    // Nothing in progress: a fix has nothing to apply to.
    std::fs::write(folder.join("fix.txt"), "fix 1 1\n").unwrap();
    let output = stave(&folder, &["edit", "project", "--script", "fix.txt"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        stdout(&output),
        "> fix 1 1\nerror: nothing is being edited: start with a selection ('select region <name>')\n"
    );
}

/// A cabinet against a wall as far forward as a door would reach, with a
/// `Door` component that takes its mount.
const DOOR_PROJECT: &str =
    include_str!("../../stave-app/tests/projects/door_hits_wall/cabinet.stave");

#[test]
fn a_script_applies_a_refusal_s_fix_by_number() {
    let folder = scratch_folder("fix");
    std::fs::create_dir_all(folder.join("door")).unwrap();
    std::fs::write(folder.join("door/cabinet.stave"), DOOR_PROJECT).unwrap();
    std::fs::write(
        folder.join("steps.txt"),
        "select region bay\nchoose Door\nbind face side_l.right\nset mount = overlay(left: 17.8mm, right: 17.8mm)\nshow\nfix 1 1\nadd\n",
    )
    .unwrap();
    let output = stave(&folder, &["edit", "door", "--script", "steps.txt"]);
    let printed = stdout(&output);
    assert_eq!(output.status.code(), Some(0), "{printed}");
    assert!(
        printed.contains("> set mount = overlay(left: 17.8mm, right: 17.8mm)\nrefused: 1 error\n"),
        "{printed}"
    );
    assert!(
        printed.contains("\nerror[motion]: door.leaf hits wall"),
        "{printed}"
    );
    assert!(printed.contains("\nfix 1 1: Use left: "), "{printed}");
    // The door now joins the side it hangs on, so the loose side's
    // warning names it too; warnings never block.
    assert!(
        printed.contains("\n> fix 1 1\nfits, with 1 warning\n> add\nwrote door/cabinet.stave\ncommitted: the design checks with 0 errors\n"),
        "{printed}"
    );
    let check = stave(&folder, &["check", "door"]);
    assert!(
        check.status.success(),
        "{}",
        String::from_utf8_lossy(&check.stderr)
    );
}

#[test]
fn script_lines_that_are_not_steps_are_errors_before_anything_runs() {
    let folder = scratch_folder("bad");
    for (line, says) in [
        ("repeat drawer 1", "is not a count of two or more"),
        ("repeat 2", "say which way to split the space"),
        ("hold a b min 1mm", "a held distance is written"),
        ("stock oak white_oak solid 19mm", "say the longest board"),
        ("new Hall 1m 1m 1m", "a new project is written"),
        ("shares drawer_bay 0 10mm", "is not a number counted from 1"),
    ] {
        std::fs::write(folder.join("bad.txt"), format!("{line}\n")).unwrap();
        let output = stave(&folder, &["edit", "project", "--script", "bad.txt"]);
        assert_eq!(output.status.code(), Some(2), "{line}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(says), "{line}: {stderr}");
    }
}

/// The owner's case by hand: the joinery's options listed with whether
/// each fits, chigiri typed and blamed, and the joinery put back.
const CASE_JOINERY: &str = r#"new "Case test" 940mm 1780mm 580mm
import std:casework
stock oak_19 white_oak solid 19mm max_width 200mm max_length 2700mm
tools pencil marking_knife square tape marking_gauge awl calipers tenon_saw chisel mallet router_plane clamps screwdriver router straight_bit drill depth_stop
create
select region envelope
choose casework.Case
set panel_stock = oak_19
options joinery
set joinery = chigiri
show
put back
put back
"#;

#[test]
fn a_script_lists_a_settings_options_blames_a_refusal_and_puts_it_back() {
    let folder = scratch_folder("case-joinery");
    std::fs::write(folder.join("case.txt"), CASE_JOINERY).unwrap();
    let output = stave(&folder, &["edit", "project", "--script", "case.txt"]);
    let text = stdout(&output);
    // The second 'put back' has nothing to put back: the run stops there.
    assert_eq!(output.status.code(), Some(1), "{text}");

    let listing = text
        .split("> options joinery\n")
        .nth(1)
        .and_then(|rest| rest.split("> set joinery").next())
        .unwrap();
    let mut lines = listing.lines();
    assert_eq!(lines.next(), Some("joinery: 21 options, 7 fit"));
    let rows: Vec<&str> = lines.collect();
    assert_eq!(rows.len(), 21, "{listing}");
    let row = |name: &str| -> &str {
        rows.iter()
            .find(|row| row.split_whitespace().nth(1) == Some(name))
            .unwrap_or_else(|| panic!("no row for {name}: {listing}"))
    };
    for fits in ["dowel", "dado", "screw"] {
        assert!(row(fits).ends_with("  fits"), "{}", row(fits));
    }
    for edge in ["chigiri", "splined_edge"] {
        assert!(row(edge).contains("  refused: error["), "{}", row(edge));
    }
    // A kind that can never be a Case's joinery says why in one sentence.
    assert!(
        row("half_lap").ends_with(
            "refused: Case hands its joint two parts as piece (case.side[left] and case.side[right]), and half_lap takes one part there"
        ),
        "{}",
        row("half_lap")
    );
    assert!(row("mortise_tenon").contains("refused: error[wall]:"));
    // The seven that fit first, then the thirteen refused: glued_edge is an
    // edge joint, and a case's corner is not edge to edge.
    assert!(
        rows[..7].iter().all(|row| row.ends_with("  fits")),
        "{listing}"
    );
    assert!(
        rows[7..].iter().all(|row| row.contains("  refused: ")),
        "{listing}"
    );
    for edge in ["glued_edge", "chigiri"] {
        assert!(
            row(edge).contains("refused: error[require]: 'side_by_side' fails"),
            "{}",
            row(edge)
        );
    }
    assert!(rows[0].starts_with("  1. "), "{listing}");

    let blamed = "> set joinery = chigiri\nrefused: 2 errors\n  It stopped fitting when joinery became chigiri.\n  'put back': Put joinery back to dowel(count: 4), the default\n";
    assert!(text.contains(blamed), "{text}");
    // The card puts the blame right under the verdict, the checker's own
    // words below it.
    let card = text.split("> show\n").nth(1).unwrap();
    assert!(
        card.contains("verdict: refused: 2 errors\n  It stopped fitting when joinery became chigiri.\n  'put back': Put joinery back to dowel(count: 4), the default\nerror["),
        "{card}"
    );
    assert!(text.contains("> put back\nfits\n> put back\nrejected: nothing to put back: no setting change made it stop fitting\n"), "{text}");
}

#[test]
fn options_of_a_setting_that_is_not_there_is_an_error() {
    let folder = scratch_folder("no-setting");
    let script = CASE_JOINERY.replace("options joinery", "options nothing");
    std::fs::write(folder.join("case.txt"), script).unwrap();
    let output = stave(&folder, &["edit", "project", "--script", "case.txt"]);
    let text = stdout(&output);
    assert_eq!(output.status.code(), Some(1), "{text}");
    assert!(
        text.contains("> options nothing\nerror: there is no setting called 'nothing'\n"),
        "{text}"
    );
}

const CASE_CHOICES: &str = r#"new "Case test" 940mm 1780mm 580mm
import std:casework
stock oak_19 white_oak solid 19mm max_width 200mm max_length 2700mm
tools pencil marking_knife square tape marking_gauge awl calipers tenon_saw chisel mallet router_plane clamps screwdriver router straight_bit drill depth_stop
create
select region envelope
choose casework.Case
set panel_stock = oak_19
choices joinery
search joinery key
search joinery dow
set joinery = screw
add
select region envelope
choose casework.Case
choices joinery
set joinery = dado
choices joinery
default joinery
choices joinery
"#;

#[test]
fn a_script_prints_a_settings_short_list_and_a_searchs_matches() {
    let folder = scratch_folder("case-choices");
    std::fs::write(folder.join("case.txt"), CASE_CHOICES).unwrap();
    let output = stave(&folder, &["edit", "project", "--script", "case.txt"]);
    let text = stdout(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    let after = |step: &str, nth: usize| -> String {
        text.split(&format!("> {step}\n"))
            .nth(nth)
            .and_then(|rest| rest.split("\n> ").next())
            .unwrap_or_else(|| panic!("no step {step}: {text}"))
            .to_owned()
    };
    // A new project: the default only, nothing used yet.
    assert_eq!(
        after("choices joinery", 1),
        "joinery: 1 to choose from (0 used in this project), then search 21 options\n  1. dowel(count: 4)  default, now"
    );
    // Chigiri is found by its description, refused with the checker's
    // reason; dowel by its name, and it fits.
    let key = after("search joinery key", 1);
    assert!(
        key.starts_with("joinery: 1 of 21 options match \"key\"\n  Standard · joinery\n    chigiri  by description  refused: error[require]: "),
        "{key}"
    );
    assert!(
        key.ends_with("Chigiri: a butterfly key let into the face across a seam."),
        "{key}"
    );
    let dow = after("search joinery dow", 1);
    assert!(
        dow.starts_with("joinery: 1 of 21 options match \"dow\"\n  Standard · joinery\n    dowel  by name  fits\n"),
        "{dow}"
    );
    // With a case that uses screws in the project, screw is offered as
    // used; the value chosen is listed as now, and the default again.
    assert!(
        after("choices joinery", 2).starts_with(
            "joinery: 2 to choose from (1 used in this project), then search 21 options\n  1. dowel(count: 4)  default, now\n  2. screw            used, "
        ),
        "{text}"
    );
    let chosen = after("choices joinery", 3);
    assert!(
        chosen.contains("  1. dowel(count: 4)  default\n"),
        "{chosen}"
    );
    assert!(chosen.contains("\n  3. dado             now"), "{chosen}");
    assert!(
        after("choices joinery", 4).contains("  1. dowel(count: 4)  default, now"),
        "{text}"
    );
}
