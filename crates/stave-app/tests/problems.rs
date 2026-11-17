//! The problems list (the parts tree showing only parts with problems,
//! one row per kind of problem) and a problem's details.

use std::path::{Path, PathBuf};
use std::time::Instant;

use stave_app::check::{self, Checked, Outcome};
use stave_app::problems::{self, Category, Filter, ProblemEntry, ProblemsTree};
use stave_app::session::{ProblemIndex, Selection};
use stave_lang::Severity;

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn checked(folder: &Path) -> Checked {
    match check::run(folder) {
        Outcome::Checked(checked) => *checked,
        Outcome::CantOpen { message, .. } => panic!("can't open {}: {message}", folder.display()),
    }
}

fn broken(name: &str) -> Checked {
    checked(&workspace().join("tests/broken").join(name))
}

fn several() -> Checked {
    checked(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/projects/several_problems"))
}

/// The tree as lines of text, for comparing whole.
fn outline(tree: &ProblemsTree) -> Vec<String> {
    fn entry(lines: &mut Vec<String>, depth: usize, entry: &ProblemEntry) {
        let pad = "  ".repeat(depth);
        let mark = if entry.involved() {
            "involved".to_owned()
        } else {
            entry.about.to_string()
        };
        lines.push(format!("{pad}{} [{mark}]", entry.entry.label));
        for row in &entry.rows {
            lines.push(format!("{pad}  {}  {}", row.title(), row.places_text()));
        }
    }
    fn assembly(lines: &mut Vec<String>, depth: usize, node: &problems::ProblemAssembly) {
        lines.push(format!("{}{}/", "  ".repeat(depth), node.label));
        for e in &node.entries {
            entry(lines, depth + 1, e);
        }
        for child in &node.children {
            assembly(lines, depth + 1, child);
        }
    }
    let mut lines = Vec::new();
    for e in &tree.carcass {
        entry(&mut lines, 0, e);
    }
    for node in &tree.assemblies {
        assembly(&mut lines, 0, node);
    }
    for row in &tree.elsewhere {
        lines.push(format!("elsewhere: {}  {}", row.title(), row.places_text()));
    }
    lines
}

#[test]
fn a_door_that_hits_lists_the_door_and_what_it_hits() {
    let checked = broken("door_hits_side");
    let tree = problems::tree(&checked, Filter::All);
    assert_eq!(tree.counts.all, 1);
    assert_eq!(tree.counts.errors, 1);
    assert_eq!(tree.counts.warnings, 0);
    assert_eq!(
        outline(&tree),
        [
            "wall [involved]",
            "  Motion  project.stave[82]",
            "door/",
            "  panel [1]",
            "    Motion  project.stave[82]",
        ]
    );
    let row = &tree.assemblies[0].entries[0].rows[0];
    assert_eq!(row.category, Category::Motion);
    assert_eq!(row.severity, Severity::Error);
    assert!(row.holds(Some(&Selection::Problem(ProblemIndex(0)))));
    assert!(!row.holds(None));
    assert_eq!(
        row.messages,
        "door.panel hits wall when door is opened past 0.6deg"
    );
    assert!(row.places[0].location.path.ends_with("project.stave"));
    assert_eq!(row.places[0].location.line.get(), 82);
}

#[test]
fn several_problems_are_grouped_by_part_and_kind() {
    let checked = several();
    let tree = problems::tree(&checked, Filter::All);
    assert_eq!(tree.counts.all, 4);
    assert_eq!(
        outline(&tree),
        [
            "side_l [1]",
            "  Overlap  project.stave[86]",
            "wall [involved]",
            "  Motion  project.stave[81]",
            "side_r [1]",
            "  Overlap  project.stave[86]",
            "shelf [2]",
            "  Overlap (2)  project.stave[86], project.stave[100]",
            "block [1]",
            "  Overlap  project.stave[100]",
            "post [involved]",
            "  No contact  project.stave[144]",
            "rail [1]",
            "  No contact  project.stave[144]",
            "door/",
            "  panel [1]",
            "    Motion  project.stave[81]",
        ]
    );
}

#[test]
fn the_filters_count_and_choose_by_severity() {
    let checked = several();
    assert!(problems::tree(&checked, Filter::Warnings).is_empty());
    let errors = problems::tree(&checked, Filter::Errors);
    assert_eq!(errors, problems::tree(&checked, Filter::All));
    // The examples have warnings only (cuts not drawn yet).
    let cabinet = checked_with_warnings();
    let tree = problems::tree(&cabinet, Filter::Errors);
    assert!(tree.is_empty());
    assert!(tree.counts.warnings > 0);
    assert!(!problems::tree(&cabinet, Filter::Warnings).is_empty());
}

fn checked_with_warnings() -> Checked {
    for name in ["joint_samples", "japanese_joinery", "box", "nightstand"] {
        let found = checked(&workspace().join("examples").join(name));
        if found.summary.warnings > 0 {
            return found;
        }
    }
    panic!("no example has a warning");
}

#[test]
fn problems_that_are_not_about_a_part_come_after_the_tree() {
    let checked = broken("unknown_name");
    let tree = problems::tree(&checked, Filter::All);
    assert!(tree.carcass.is_empty() && tree.assemblies.is_empty());
    assert_eq!(tree.elsewhere.len(), 1);
    assert_eq!(tree.elsewhere[0].category, Category::Name);
}

#[test]
fn a_motion_problem_has_its_details_and_its_card() {
    let checked = broken("door_hits_side");
    let details = problems::details(&checked, ProblemIndex(0)).expect("details");
    assert_eq!(details.eyebrow, "ERROR · MOTION");
    assert_eq!(details.title, "door hits wall as it opens");
    assert_eq!(
        details.explanation,
        "door.panel hits wall when door is opened past 0.6deg. The hinges open to 110deg."
    );
    // Exactly what `stave check .` prints from inside the folder.
    assert!(
        details
            .text
            .starts_with("error[motion]: door.panel hits wall when door is opened past 0.6deg\n")
    );
    assert!(details.text.contains("82 | assembly door : Door {"));
    assert!(details.text.contains("  --> project.stave:82:10\n"));
    assert!(
        details
            .text
            .ends_with("= help: leave at least 1mm between door and wall")
    );
    assert_eq!(details.fixes.len(), 1);
    assert_eq!(
        details.fixes[0].text,
        "Leave at least 1mm between door and wall"
    );
    assert_eq!(details.fixes[0].location, details.location);
    let card = details.motion.expect("a motion card");
    assert_eq!(card.name, "door");
    assert!(card.turns);
    assert_eq!(card.lead(), "Swing door");
    assert_eq!(card.start, "closed 0°");
    assert_eq!(card.blocked, "blocked from 0.6°");
    assert_eq!(card.end, "opens to 110°");
    assert!(card.contact > 0.0 && card.contact < card.shown && card.shown < card.full);
    assert_eq!(card.amount_text(card.full), "110°");
}

#[test]
fn other_problems_have_plain_titles_and_no_card() {
    let checked = several();
    let overlap = problems::details(&checked, ProblemIndex(1)).expect("details");
    assert_eq!(overlap.eyebrow, "ERROR · OVERLAP");
    assert_eq!(overlap.title, "Wood passes through wood");
    assert!(overlap.motion.is_none());
    let syntax = broken("syntax_error");
    let details = problems::details(&syntax, ProblemIndex(0)).expect("details");
    assert_eq!(details.category, Category::Syntax);
    assert_eq!(details.title, "Stave can't read the text here");
    assert!(problems::details(&syntax, ProblemIndex(99)).is_none());
}

#[test]
fn the_list_is_the_same_every_time() {
    let started = Instant::now();
    let a = problems::tree(&several(), Filter::All);
    let b = problems::tree(&several(), Filter::All);
    assert_eq!(a, b);
    assert!(started.elapsed().as_secs() < 120);
}
