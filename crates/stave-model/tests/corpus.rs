//! Every module in examples/ and std/ resolves without errors.

use std::path::{Path, PathBuf};

use stave_lang::render_diagnostics;
use stave_modules::ModuleSet;

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn module_folders() -> Vec<PathBuf> {
    let root = workspace();
    let mut folders = Vec::new();
    for top in ["examples", "std"] {
        let mut stack = vec![root.join(top)];
        while let Some(folder) = stack.pop() {
            let mut has_stave = false;
            for entry in std::fs::read_dir(&folder).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "stave") {
                    has_stave = true;
                }
            }
            if has_stave {
                folders.push(folder);
            }
        }
    }
    folders.sort();
    folders
}

#[test]
fn every_module_resolves_cleanly() {
    let folders = module_folders();
    assert!(folders.len() >= 8);
    for folder in folders {
        let modules = ModuleSet::load(&folder).unwrap();
        let (_, diagnostics) = stave_model::resolve(&modules);
        let mut all = modules.diagnostics.clone();
        all.extend(diagnostics);
        assert!(
            all.is_empty(),
            "{}:\n{}",
            folder.display(),
            render_diagnostics(&modules.sources, &all)
        );
    }
}
