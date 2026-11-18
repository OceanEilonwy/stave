//! The fast path for changed copies of a project ([`stave_app::Prepared`])
//! gives exactly what the full pipeline gives: the same sources, model,
//! solved design, diagnostics (in the same order) and contacts, for every
//! example, broken case and design in the corpus, unchanged and changed.

use std::path::{Path, PathBuf};

use stave_app::Opened;
use stave_ide::Overlay;
use stave_modules::DiskFiles;

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Every project folder of the corpus, sorted.
fn corpus() -> Vec<PathBuf> {
    let mut folders = Vec::new();
    for group in ["examples", "tests/broken", "tests/designs"] {
        let Ok(entries) = std::fs::read_dir(workspace().join(group)) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                folders.push(path);
            }
        }
    }
    folders.sort();
    assert!(folders.len() > 50, "the corpus is there");
    folders
}

/// The `.stave` files directly in a folder, sorted.
fn stave_files(folder: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(folder)
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|e| e == "stave"))
        .collect();
    files.sort();
    files
}

fn assert_same(fast: &Opened, full: &Opened, what: &str) {
    assert_eq!(fast.sources, full.sources, "{what}: sources");
    assert_eq!(fast.diagnostics, full.diagnostics, "{what}: diagnostics");
    assert_eq!(fast.model, full.model, "{what}: model");
    assert_eq!(fast.design, full.design, "{what}: solved design");
    assert_eq!(fast.contacts, full.contacts, "{what}: contacts");
    assert_eq!(fast.root_files, full.root_files, "{what}: file count");
}

#[test]
fn prepare_and_an_unchanged_reopen_equal_the_full_pipeline() {
    for folder in corpus() {
        let what = folder.display().to_string();
        let full = stave_app::open(&folder).unwrap();
        let (opened, prepared) = stave_app::prepare(&folder, &DiskFiles).unwrap();
        assert_same(&opened, &full, &what);
        let again = prepared.reopen(&folder, &DiskFiles).unwrap();
        assert_same(&again, &full, &what);
    }
}

/// A number in the text made one larger: `18mm` becomes `19mm`. The first
/// number followed by `mm` after the header line, when there is one.
fn nudged(text: &str) -> Option<String> {
    let start = text.find('\n')?;
    let bytes = text.as_bytes();
    let mut index = start;
    while index < bytes.len() {
        if bytes.get(index).is_some_and(u8::is_ascii_digit) {
            let begin = index;
            while bytes.get(index).is_some_and(u8::is_ascii_digit) {
                index += 1;
            }
            if text.get(index..index + 2) == Some("mm")
                && !bytes
                    .get(begin - 1)
                    .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'.')
            {
                let value: u64 = text.get(begin..index)?.parse().ok()?;
                let mut changed = String::with_capacity(text.len() + 1);
                changed.push_str(text.get(..begin)?);
                changed.push_str(&(value + 1).to_string());
                changed.push_str(text.get(index..)?);
                return Some(changed);
            }
        } else {
            index += 1;
        }
    }
    None
}

#[test]
fn reopening_changed_copies_equals_the_full_pipeline() {
    for folder in corpus() {
        let Ok((_, prepared)) = stave_app::prepare(&folder, &DiskFiles) else {
            continue;
        };
        // Every kind of change in the first file; a size in the others.
        for (index, file) in stave_files(&folder).into_iter().enumerate() {
            let text = std::fs::read_to_string(&file).unwrap();
            let mut changes = Vec::new();
            if index == 0 {
                changes.push(("a comment added", format!("{text}\n// changed\n")));
                changes.push(("a syntax error added", format!("{text}\npart = {{\n")));
                changes.push((
                    "an unknown name added",
                    format!("{text}\nlet changed_copy = no_such_name * 2\n"),
                ));
            }
            if let Some(changed) = nudged(&text) {
                changes.push(("a size changed", changed));
            }
            for (change, changed) in changes {
                let what = format!("{} with {change}", file.display());
                let mut overlay = Overlay::new();
                overlay.insert(&file, changed);
                let full = stave_app::open_with(&folder, &overlay).unwrap();
                let fast = prepared.reopen(&folder, &overlay).unwrap();
                assert_same(&fast, &full, &what);
            }
        }
    }
}

#[test]
fn reopening_with_a_new_file_or_import_falls_back_and_still_equals() {
    let folder = workspace().join("examples/clothing_cabinet");
    let (_, prepared) = stave_app::prepare(&folder, &DiskFiles).unwrap();
    // A new file renumbers the libraries' files: nothing resolved is
    // reused, and the result is still the full pipeline's.
    let mut overlay = Overlay::new();
    overlay.insert(
        folder.join("zz_extra.stave"),
        "stave 1\n\nparam extra_gap = 2mm\n",
    );
    let full = stave_app::open_with(&folder, &overlay).unwrap();
    let fast = prepared.reopen(&folder, &overlay).unwrap();
    assert_same(&fast, &full, "a new file");
    // A new import adds a module.
    let file = folder.join("stock.stave");
    let text = std::fs::read_to_string(&file).unwrap();
    let with_import = text.replacen("stave 1\n", "stave 1\n\nimport \"../bookshelf\"\n", 1);
    assert_ne!(with_import, text);
    let mut overlay = Overlay::new();
    overlay.insert(&file, with_import);
    let full = stave_app::open_with(&folder, &overlay).unwrap();
    let fast = prepared.reopen(&folder, &overlay).unwrap();
    assert_same(&fast, &full, "a new import");
}
