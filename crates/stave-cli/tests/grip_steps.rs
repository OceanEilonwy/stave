//! The script steps for grips, the solver, a part spaced along a span and
//! a measured distance (`docs/editing.md`, "Scripts"): `grips`, `stops`,
//! `drag`, `grip`, `pick`, `solved`, `plain`, `spaced`, `measure` and
//! `hold` without a value. Each transcript is pinned, on a copy of
//! `examples/side_table_from_box` (read, never written) or a small
//! project written here, in a temporary folder.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A fresh temporary folder named for the test.
fn scratch_folder(name: &str) -> PathBuf {
    let folder =
        std::env::temp_dir().join(format!("stave-grip-steps-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
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

const SCRIPT: &str = r#"grips assembly apron_front
stops 2
drag 2 8mm
drag 2 6.2mm
cancel
grip 2 = 4mm
add
grips assembly apron_front
drag 4 380mm
pick overall_width
show
cancel
solved overall_height within 550mm..700mm
add
grips assembly legs
drag 3 560mm
drag 3 400mm
cancel
plain overall_height
add
measure slats.slat[0] apron_front.apron
hold slats.slat[0] apron_front.apron min "room for books"
add
"#;

const TRANSCRIPT: &str = r#"> grips assembly apron_front
  1. apron_front.width         90mm  moves -910mm..1090mm (up)
  2. apron_front.setback       5mm  moves -995mm..1005mm (deep)
  3. apron_front.tenon_length  20mm  moves -980mm..1020mm
  4. apron_front.apron.length  360mm  follows overall_width, legs.inset, oak_45_sq.width (a size of the stock) (across)
  5. apron_front.apron.width   90mm  follows apron_front.width (up)
> stops 2
  2. apron_front.setback
       down to 0mm (-0.5mm breaks it: error[overlap]: legs and apron_front.apron overlap)
       up to 7.5mm (8mm breaks it: error[require]: 'mortise_wall' fails: the mortise comes too close to a face of the part or another cut)
> drag 2 8mm
Set apron_front.setback to 7.5mm: fits
> drag 2 6.2mm
Set apron_front.setback to 6mm: fits
> cancel
cancelled: nothing was written
> grip 2 = 4mm
Set apron_front.setback to 4mm: fits
> add
wrote table/table.stave
committed: the design checks with 0 errors
> grips assembly apron_front
  1. apron_front.width         90mm  moves -910mm..1090mm (up)
  2. apron_front.setback       4mm  moves -996mm..1004mm (deep)
  3. apron_front.tenon_length  20mm  moves -980mm..1020mm
  4. apron_front.apron.length  360mm  follows overall_width, legs.inset, oak_45_sq.width (a size of the stock) (across)
  5. apron_front.apron.width   90mm  follows apron_front.width (up)
> drag 4 380mm
Drag apron_front.apron.length to 380mm: pick which size changes: overall_width, inset
  could change overall_width: 500mm -> 520mm
  could change legs.inset: 25mm -> 15mm
> pick overall_width
Set overall_width to 520mm: fits
  changes overall_width 500mm -> 520mm
> show
Set overall_width to 520mm
  grip 4: apron_front.apron.length
  moved to 380mm
  changes overall_width 500mm -> 520mm
verdict: fits
patch:
  stave 1

  patch "Set overall_width to 520mm" {
    set overall_width = 520mm
  }
> cancel
cancelled: nothing was written
> solved overall_height within 550mm..700mm
Let the solver change overall_height: fits
> add
wrote table/table.stave
committed: the design checks with 0 errors
> grips assembly legs
  1. legs.length                   581mm  follows overall_height (solved), oak_19.thickness (a size of the stock)
  2. legs.inset                    25mm  moves -975mm..1025mm (across)
  3. legs.leg[front_left].length   581mm  follows overall_height (solved), oak_19.thickness (a size of the stock) (up)
  4. legs.leg[front_left].width    45mm  follows oak_45_sq.width (a size of the stock) (across)
  5. legs.leg[front_right].length  581mm  follows overall_height (solved), oak_19.thickness (a size of the stock) (up)
  6. legs.leg[front_right].width   45mm  follows oak_45_sq.width (a size of the stock) (across)
  7. legs.leg[back_left].length    581mm  follows overall_height (solved), oak_19.thickness (a size of the stock) (up)
  8. legs.leg[back_left].width     45mm  follows oak_45_sq.width (a size of the stock) (across)
  9. legs.leg[back_right].length   581mm  follows overall_height (solved), oak_19.thickness (a size of the stock) (up)
 10. legs.leg[back_right].width    45mm  follows oak_45_sq.width (a size of the stock) (across)
> drag 3 560mm
Set overall_height to 579mm: fits
  changes overall_height 600mm -> 579mm
> drag 3 400mm
Drag legs.leg[front_left].length to 400mm: refused: can't reach 400mm: the rule 'knee_room' stops it (leave room under the aprons for your knees)
> cancel
cancelled: nothing was written
> plain overall_height
Make overall_height a plain param: fits
> add
wrote table/table.stave
committed: the design checks with 0 errors
> measure slats.slat[0] apron_front.apron
slats.slat[0] to apron_front.apron: 322mm
> hold slats.slat[0] apron_front.apron min "room for books"
the distance is 322mm now
Hold between(slats.slat[0], apron_front.apron) >= 322mm: fits
> add
wrote table/table.stave
committed: the design checks with 0 errors
"#;

/// Grips listed, stops searched, a drag (snapped, and held to its stops),
/// a typed value, a pick between two plain params, a param made solved,
/// a drag the solver works out and one a rule blocks, the param made plain
/// again, and a distance measured and held as it is.
#[test]
fn grips_the_solver_and_a_measured_hold_from_a_script() {
    let folder = scratch_folder("table");
    let table = folder.join("table");
    std::fs::create_dir_all(&table).unwrap();
    for entry in std::fs::read_dir(workspace().join("examples/side_table_from_box")).unwrap() {
        let path = entry.unwrap().path();
        std::fs::copy(&path, table.join(path.file_name().unwrap())).unwrap();
    }
    std::fs::write(folder.join("steps.txt"), SCRIPT).unwrap();
    let output = stave(&folder, &["edit", "table", "--script", "steps.txt"]);
    assert_eq!(output.status.code(), Some(0), "{}", stdout(&output));
    assert_eq!(stdout(&output), TRANSCRIPT);
    let check = stave(&folder, &["check", "table"]);
    assert!(
        check.status.success(),
        "{}",
        String::from_utf8_lossy(&check.stderr)
    );
    let text = std::fs::read_to_string(table.join("table.stave")).unwrap();
    assert!(text.contains("setback:     4mm"), "{text}");
    assert!(!text.contains("solved param"), "{text}");
    assert!(
        text.contains("require slats_slat_to_apron_front_min : between(slats.slat[0], apron_front.apron) >= 322mm \"room for books\""),
        "{text}"
    );
}

const SLATS: &str = r#"stave 1

project "Slat test" {
  units: mm
}

param overall_width  = 400mm
param overall_height = 300mm
param overall_depth  = 300mm

region envelope = box(across: overall_width, up: overall_height, deep: overall_depth)

stock oak_19 { species: white_oak, form: solid, thickness: 19mm, max_width: 200mm, max_length: 2700mm }

part rail_l : oak_19 {
  length: 300mm
  width:  40mm
  place {
    orient(length: back, width: up)
    flush(envelope.bottom)
    flush(envelope.left)
    flush(envelope.front)
  }
}

part rail_r : oak_19 {
  length: 300mm
  width:  40mm
  place {
    orient(length: back, width: up)
    flush(envelope.bottom)
    flush(envelope.right)
    flush(envelope.front)
  }
}

part slat : oak_19 {
  length: 300mm
  width:  40mm
  place {
    orient(length: back, width: right)
    against(rail_l.top)
    flush(rail_l.right)
    flush(envelope.front)
  }
}
"#;

/// A part repeated, spaced along a span, then its count changed.
#[test]
fn a_part_is_spaced_along_a_span_from_a_script() {
    let folder = scratch_folder("slats");
    let project = folder.join("slats");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("project.stave"), SLATS).unwrap();
    std::fs::write(
        folder.join("steps.txt"),
        "spaced slat 3 across from rail_l.right to rail_r.left\nadd\nspaced slat 4 across from rail_l.right to rail_r.left\nadd\n",
    )
    .unwrap();
    let output = stave(&folder, &["edit", "slats", "--script", "steps.txt"]);
    assert_eq!(output.status.code(), Some(0), "{}", stdout(&output));
    assert_eq!(
        stdout(&output),
        "> spaced slat 3 across from rail_l.right to rail_r.left
Make 3 of slat: fits, with 1 warning
> add
wrote slats/project.stave
committed: the design checks with 0 errors
> spaced slat 4 across from rail_l.right to rail_r.left
Make 4 of slat: fits, with 1 warning
> add
wrote slats/project.stave
committed: the design checks with 0 errors
"
    );
    let text = std::fs::read_to_string(project.join("project.stave")).unwrap();
    assert!(text.contains("part slat[4] : oak_19 {"), "{text}");
    assert!(
        text.contains("spaced(evenly, from: rail_l.right, to: rail_r.left)"),
        "{text}"
    );
}

/// A step a script can't read is refused before anything runs.
#[test]
fn the_new_steps_are_read_strictly() {
    let folder = scratch_folder("strict");
    for (line, says) in [
        ("solved overall_width within 600mm", "needs a range"),
        ("drag 0 400mm", "counted from 1"),
        ("grips everything", "is not a step"),
    ] {
        std::fs::write(folder.join("steps.txt"), format!("{line}\n")).unwrap();
        let output = stave(&folder, &["edit", "nothing", "--script", "steps.txt"]);
        assert_eq!(output.status.code(), Some(2), "{line}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(says), "{line}: {stderr}");
    }
}
