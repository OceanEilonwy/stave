//! The acceptance test of the editor (`docs/editing.md`, "Scripts"): two
//! whole pieces of furniture built from an empty project by `stave edit
//! --script`, with only the steps a person takes with the mouse
//! (selections, menu choices, settings, repeat, mirror, a grip's value and
//! hold), never a hand-written patch. The scripts are in
//! `tests/edit-scripts/`:
//!
//! - `clothing_cabinet.txt` builds the cabinet of
//!   `examples/clothing_cabinet_from_box`;
//! - `side_table.txt` builds the table of `examples/side_table_from_box`.
//!
//! Each runs in a temporary folder. Every step must be accepted, except a
//! step the script marks `# expect: refused`, which must be refused (and
//! the script goes on to put it right or cancel it). The finished project
//! must check with no error and no warning, and make the same parts, by
//! stock and cut size, and the same joints, by kind and count, as the
//! example. The text it writes is pinned byte for byte by
//! `tests/edit-scripts/<name>.stave`, so it can be read and reviewed.
//!
//! The examples are read, never written. To bless a changed text after a
//! deliberate change, run with `STAVE_BLESS=1`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use stave_app::Opened;
use stave_lang::render_diagnostics;
use stave_lang::units::Length;
use stave_model::solved::SolvedDesign;

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn scripts() -> PathBuf {
    workspace().join("tests/edit-scripts")
}

/// A fresh, empty temporary folder named for the test.
fn scratch_folder(name: &str) -> PathBuf {
    let folder = std::env::temp_dir().join(format!(
        "stave-built-by-script-{}-{name}",
        std::process::id()
    ));
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

/// One step of a script as the transcript shows it, with what it printed.
struct Ran {
    /// The step as written, without its comment.
    line: String,
    /// Whether the script marks it `# expect: refused`.
    expect_refused: bool,
    /// What it printed.
    printed: String,
}

/// The script's steps (one a line, `#` comments and blank lines left
/// out), each paired with what the transcript printed after its echo.
fn steps(script: &str, transcript: &str) -> Vec<Ran> {
    let lines: Vec<(String, bool)> = script
        .lines()
        .filter_map(|line| {
            let (step, comment) = line.split_once('#').unwrap_or((line, ""));
            let step = step.trim();
            (!step.is_empty()).then(|| (step.to_owned(), comment.trim() == "expect: refused"))
        })
        .collect();
    let mut printed: Vec<(String, String)> = Vec::new();
    for line in transcript.lines() {
        if let Some(step) = line.strip_prefix("> ") {
            printed.push((step.to_owned(), String::new()));
        } else if let Some((_, text)) = printed.last_mut() {
            text.push_str(line);
            text.push('\n');
        }
    }
    assert_eq!(
        lines.len(),
        printed.len(),
        "the run stopped early:\n{transcript}"
    );
    lines
        .into_iter()
        .zip(printed)
        .map(|((line, expect_refused), (echoed, printed))| {
            assert_eq!(line, echoed);
            Ran {
                line,
                expect_refused,
                printed,
            }
        })
        .collect()
}

/// Whether a step's output says it was refused or failed: a candidate or
/// change that doesn't fit, a rejected command, a commit that wasn't
/// written, or a step that couldn't run.
fn refused(printed: &str) -> bool {
    printed.lines().any(|line| {
        // A step's own line starts at the margin; a card's verdict line
        // repeats what the step that made it said.
        let change = !line.starts_with(' ') && !line.starts_with("verdict:");
        line.starts_with("refused")
            || (change && line.contains(": refused:"))
            || line.starts_with("rejected:")
            || line.starts_with("not committed:")
            || line.starts_with("error:")
    })
}

/// Opens a project folder; panics unless it checks with no diagnostic at
/// all.
fn clean(folder: &Path) -> Opened {
    let opened = stave_app::open(folder).unwrap();
    assert!(
        opened.diagnostics.is_empty(),
        "{}:\n{}",
        folder.display(),
        render_diagnostics(&opened.sources, &opened.diagnostics)
    );
    opened
}

fn solved(opened: &Opened) -> &SolvedDesign {
    opened.design.as_ref().expect("a solved design")
}

/// One part with its name left out: its stock, its cut length, width and
/// thickness, and how many cuts it receives.
type PartKey = (String, Length, Length, Length, usize);

/// Every part as a [`PartKey`], sorted, so two designs that differ only in
/// names compare equal.
fn parts(design: &SolvedDesign) -> Vec<PartKey> {
    let mut parts: Vec<PartKey> = design
        .parts
        .iter()
        .map(|part| {
            (
                part.stock.name.clone(),
                part.cut.length,
                part.cut.width,
                part.cut.thickness,
                part.cuts.len(),
            )
        })
        .collect();
    parts.sort();
    parts
}

/// The joints by kind: how many connections (pairs of parts that meet)
/// each kind makes.
fn joints(design: &SolvedDesign) -> BTreeMap<String, usize> {
    let mut joints = BTreeMap::new();
    for joint in &design.joints {
        *joints.entry(joint.kind.clone()).or_insert(0) += joint.pairs.len();
    }
    joints
}

/// Runs `tests/edit-scripts/<name>.txt` into `<folder>/<project>` and
/// checks it as the module's notes say, against `examples/<example>`.
fn build(name: &str, project: &str, file: &str, example: &str) {
    let folder = scratch_folder(name);
    let script_path = scripts().join(format!("{name}.txt"));
    let script = std::fs::read_to_string(&script_path).unwrap();
    let output = stave(
        &folder,
        &["edit", project, "--script", script_path.to_str().unwrap()],
    );
    let transcript = String::from_utf8_lossy(&output.stdout).into_owned();
    assert_eq!(
        output.status.code(),
        Some(0),
        "{transcript}{}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Every step is accepted, or refused exactly where the script says.
    for ran in steps(&script, &transcript) {
        assert_eq!(
            refused(&ran.printed),
            ran.expect_refused,
            "step '{}' printed:\n{}",
            ran.line,
            ran.printed
        );
    }

    // The finished project checks clean, and `stave check` agrees.
    let built_folder = folder.join(project);
    let built = clean(&built_folder);
    let check = stave(&folder, &["check", project]);
    let stderr = String::from_utf8_lossy(&check.stderr);
    assert!(check.status.success(), "{stderr}");
    assert!(stderr.ends_with(": 0 errors, 0 warnings\n"), "{stderr}");
    // The editor writes text as `stave fmt` lays it out.
    let format = stave(&folder, &["fmt", "--check", project]);
    assert!(
        format.status.success(),
        "{}",
        String::from_utf8_lossy(&format.stdout)
    );

    // The same parts and joints as the example.
    let example = clean(&workspace().join("examples").join(example));
    assert_eq!(parts(solved(&built)), parts(solved(&example)));
    assert_eq!(joints(solved(&built)), joints(solved(&example)));

    // The text it wrote, byte for byte.
    let text = std::fs::read_to_string(built_folder.join(file)).unwrap();
    let expected_path = scripts().join(format!("{name}.stave"));
    if std::env::var_os("STAVE_BLESS").is_some() {
        std::fs::write(&expected_path, &text).unwrap();
    }
    let expected = std::fs::read_to_string(&expected_path).unwrap();
    assert!(
        text == expected,
        "the script wrote a different text than {}:\n{text}",
        expected_path.display()
    );
    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn a_script_builds_the_clothing_cabinet_from_an_empty_project() {
    build(
        "clothing_cabinet",
        "cabinet",
        "clothing_cabinet.stave",
        "clothing_cabinet_from_box",
    );
}

#[test]
fn a_script_builds_the_side_table_from_an_empty_project() {
    build(
        "side_table",
        "table",
        "side_table.stave",
        "side_table_from_box",
    );
}

#[test]
fn a_step_marked_refused_must_be_refused_and_others_must_not() {
    assert!(refused("refused: 1 error\n"));
    assert!(refused(
        "Set apron_front.setback to 8mm: refused: 1 error\n"
    ));
    assert!(refused("not committed: the edit doesn't fit\n"));
    assert!(refused("rejected: no such entry\n"));
    assert!(!refused("fits\n"));
    assert!(!refused(
        "Make frame_panel_door a mirrored pair: fits, with 1 warning\n"
    ));
    assert!(!refused("needs a choice: run\n"));
    assert!(!refused("verdict: refused: 1 error\n"));
    // A menu's greyed entries are not the step being refused.
    assert!(!refused(
        " 10. hardware.knob              refused\n       error[axis-not-set]: knob_2\n"
    ));
}
