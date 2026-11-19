//! The translation template's file list: `po/POTFILES.in` names every
//! source file whose words are marked for translation, so `po/update-pot`
//! collects them all, and names no file that is gone.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The repository's root.
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Every `.rs` file under `folder`, relative to the root.
fn rust_files(folder: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            out.push(path);
        }
    }
}

/// Whether the text calls one of the marking functions.
fn marks_words(text: &str) -> bool {
    ["tr(", "ntr(", "trc(", "tr_noop("].iter().any(|call| {
        text.match_indices(call).any(|(at, _)| {
            text.get(..at)
                .and_then(|before| before.chars().next_back())
                .is_none_or(|c| !(c.is_alphanumeric() || c == '_'))
        })
    })
}

#[test]
fn the_template_lists_every_file_with_words_to_translate() {
    let root = root();
    let listed: BTreeSet<String> = std::fs::read_to_string(root.join("po/POTFILES.in"))
        .expect("po/POTFILES.in is there")
        .lines()
        .filter(|line| !line.starts_with('#') && !line.trim().is_empty())
        .map(str::to_owned)
        .collect();
    for file in &listed {
        assert!(
            root.join(file).is_file(),
            "po/POTFILES.in names {file}, which is gone"
        );
    }
    let mut files = Vec::new();
    for crate_folder in ["crates/stave-app/src", "crates/stave-gtk/src"] {
        rust_files(&root.join(crate_folder), &mut files);
    }
    let mut missing = Vec::new();
    for path in files {
        let relative = path
            .strip_prefix(&root)
            .expect("under the root")
            .to_string_lossy()
            .replace('\\', "/");
        if relative.ends_with("/i18n.rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("the file reads");
        if marks_words(&text) && !listed.contains(&relative) {
            missing.push(relative);
        }
    }
    assert!(
        missing.is_empty(),
        "these files mark words for translation but po/POTFILES.in doesn't list them; \
         run po/update-pot: {missing:?}"
    );
}
