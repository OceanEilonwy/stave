//! Loading again with kept parses (`ModuleSet::load_cached`): the same
//! result as a fresh load, parsing only what changed.

use std::fmt::Write as _;
use std::path::Path;

use stave_lang::{FileId, render_diagnostics};
use stave_modules::{MemoryFiles, ModuleSet, Parses};

const PROJECT: &str = "stave 1\n\nimport \"./parts\"\n\nproject \"Shop\" {\n  units: mm\n}\n";
const SHELF: &str = "stave 1\n\nparam width = 600mm\n";
const BROKEN_SHELF: &str = "stave 1\n\nparam width = \n";
const PARTS: &str = "stave 1\n\nlibrary \"Parts\"\n\nlet depth = 20mm\n";

fn files(shelf: &str) -> MemoryFiles {
    MemoryFiles::new()
        .with("shop/project.stave", PROJECT)
        .with("shop/shelf.stave", shelf)
        .with("shop/parts/parts.stave", PARTS)
}

/// Everything a load gives, written out.
fn written(set: &ModuleSet) -> String {
    let mut out = render_diagnostics(&set.sources, &set.diagnostics);
    for module in &set.modules {
        writeln!(out, "{:?} {} {:?}", module.id, module.name, module.header).unwrap();
        for file in &module.files {
            writeln!(
                out,
                "  {:?} {} {:?} {:?} {:?}",
                file.file, file.path, file.ast, file.imports, file.broken
            )
            .unwrap();
        }
    }
    out
}

fn fresh(shelf: &str) -> ModuleSet {
    ModuleSet::load_with(Path::new("shop"), &files(shelf)).unwrap()
}

fn cached(shelf: &str, parses: &mut Parses) -> ModuleSet {
    ModuleSet::load_cached(Path::new("shop"), &files(shelf), parses).unwrap()
}

fn shelf_file(set: &ModuleSet) -> FileId {
    set.modules[0]
        .files
        .iter()
        .find(|file| file.path.ends_with("shelf.stave"))
        .unwrap()
        .file
}

#[test]
fn a_cached_load_gives_what_a_fresh_load_gives() {
    let mut parses = Parses::new();
    for shelf in [SHELF, BROKEN_SHELF, BROKEN_SHELF, SHELF] {
        let mut set = cached(shelf, &mut parses);
        assert_eq!(written(&set), written(&fresh(shelf)));
        parses.reclaim(&mut set);
    }
}

#[test]
fn a_cached_load_parses_only_the_changed_files() {
    let mut parses = Parses::new();
    let mut set = cached(SHELF, &mut parses);
    let everything = parses.parsed().len();
    assert!(everything > 3, "the first load parses every file");
    parses.reclaim(&mut set);
    assert!(
        set.modules
            .iter()
            .flat_map(|module| &module.files)
            .all(|file| file.ast.is_none()),
        "the typed trees went back"
    );

    // An edit: only that file is parsed.
    let mut set = cached(BROKEN_SHELF, &mut parses);
    let shelf = shelf_file(&set);
    assert_eq!(parses.parsed(), [shelf]);
    assert!(parses.green(shelf).is_some());
    parses.reclaim(&mut set);

    // No change: nothing is parsed, the file with a syntax error included.
    let mut set = cached(BROKEN_SHELF, &mut parses);
    assert!(parses.parsed().is_empty());
    assert!(!set.diagnostics.is_empty(), "its diagnostics are kept");
    parses.reclaim(&mut set);
}

#[test]
fn trees_not_handed_back_are_parsed_again() {
    let mut parses = Parses::new();
    let first = cached(SHELF, &mut parses);
    let everything = parses.parsed().len();
    let again = cached(SHELF, &mut parses);
    assert_eq!(parses.parsed().len(), everything);
    assert_eq!(written(&again), written(&first));
}
