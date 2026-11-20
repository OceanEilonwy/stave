//! What an editor gets from a background check: each diagnostic's fixes
//! as text edits, and the values worked out for hover and inlay hints.

use std::path::{Path, PathBuf};

use stave_app::editor_check::editor_check;
use stave_ide::{Document, ValueKind};
use stave_lang::apply_text_edits;
use stave_lang::placement::PartAxis;
use stave_modules::DiskFiles;

fn workspace(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path)
        .canonicalize()
        .unwrap()
}

#[test]
fn a_fix_becomes_text_edits_that_make_the_problem_go_away() {
    let folder = workspace("tests/broken/out_of_range");
    let checked = editor_check(&folder, &DiskFiles).unwrap();
    let (index, diagnostic) = checked
        .opened
        .diagnostics
        .iter()
        .enumerate()
        .find(|(_, d)| d.code.as_str() == "out-of-range")
        .unwrap();
    let fixes = &checked.fixes[index];
    assert_eq!(fixes.len(), diagnostic.fixes.len());
    let fix = &fixes[0];
    assert_eq!(fix.label, "Use 300mm");
    let [file] = fix.files.as_slice() else {
        panic!("one file changes: {:?}", fix.files);
    };
    assert_eq!(file.path, folder.join("project.stave"));
    let text = std::fs::read_to_string(&file.path).unwrap();
    let fixed = apply_text_edits(&text, &file.edits).unwrap();
    assert!(fixed.contains("length: 300mm"), "{fixed}");
    assert_eq!(fixed.len(), text.len(), "only the number changes");
}

#[test]
fn values_are_worked_out_in_the_project_units() {
    let folder = workspace("examples/clothing_cabinet");
    let checked = editor_check(&folder, &DiskFiles).unwrap();
    let carcass = Document::File(folder.join("carcass.stave"));
    let notes = checked.values.notes(&carcass);
    let fits: Vec<_> = notes
        .iter()
        .filter(|note| note.covers == "fit" && note.kind == ValueKind::Size(PartAxis::Length))
        .collect();
    assert!(!fits.is_empty(), "{notes:#?}");
    for note in &fits {
        assert!(note.text.ends_with("mm"), "{}", note.text);
        assert!(!note.literal);
    }
    // A size that a joint adds to shows its cut length too.
    assert!(
        fits.iter().any(|note| note.text.contains(", cut ")),
        "{fits:#?}"
    );
    // Every between(...) has its value.
    let betweens: Vec<_> = notes
        .iter()
        .filter(|note| note.kind == ValueKind::Between)
        .collect();
    assert!(!betweens.is_empty(), "{notes:#?}");
    assert!(
        betweens
            .iter()
            .all(|note| note.covers.starts_with("between("))
    );
    // A param has its value, and its name to hover.
    let param = notes
        .iter()
        .chain(
            checked
                .values
                .notes(&Document::File(folder.join("cabinet.stave"))),
        )
        .find(|note| note.kind == ValueKind::Param)
        .unwrap();
    assert!(param.name.is_some());
}

#[test]
fn values_are_the_same_on_every_run() {
    let folder = workspace("examples/clothing_cabinet");
    let first = editor_check(&folder, &DiskFiles).unwrap().values;
    let second = editor_check(&folder, &DiskFiles).unwrap().values;
    assert_eq!(first, second);
}
