//! Loading the repository's corpus from disk.

use std::fs;
use std::path::{Path, PathBuf};

use stave_lang::render_diagnostics;
use stave_modules::{ModuleSet, ModuleSource, PRELUDE_MODULES, STD_MODULES};

/// The repository root, relative to this crate.
fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn render(set: &ModuleSet) -> String {
    render_diagnostics(&set.sources, &set.diagnostics)
}

/// The subfolders of `folder` that hold `.stave` files.
fn stave_folders(folder: &Path) -> Vec<PathBuf> {
    let mut folders: Vec<PathBuf> = fs::read_dir(folder)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_dir())
        .filter(|path| {
            fs::read_dir(path).unwrap().any(|entry| {
                entry
                    .unwrap()
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "stave")
            })
        })
        .collect();
    folders.sort();
    folders
}

#[test]
fn clothing_cabinet() {
    let folder = repository().join("examples/clothing_cabinet");
    let set = ModuleSet::load(&folder).unwrap();
    assert!(set.diagnostics.is_empty(), "{}", render(&set));
    assert_eq!(set.modules.len(), 6);
    let root = set.module(set.root).unwrap();
    assert_eq!(root.name, "clothing_cabinet");
    let prefix = folder.display().to_string();
    assert_eq!(root.files[0].path, format!("{prefix}/cabinet.stave"));

    let names: Vec<&str> = set.modules.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "clothing_cabinet",
            "casework",
            "hardware",
            "joinery",
            "hardware",
            "tools"
        ]
    );
    // `std:casework` is the embedded standard module, shown under `std/`.
    let casework = &set.modules[1];
    assert_eq!(casework.source, ModuleSource::Std { name: "casework" });
    assert!(
        casework
            .files
            .iter()
            .any(|file| file.path == "std/casework/drawer.stave")
    );
    // Imported folders are normalised lexically (the `..` in `repository()`
    // is resolved); the root keeps the folder as given.
    let normalized = stave_modules::normalize(&folder).display().to_string();
    let shop_hardware = &set.modules[2];
    assert!(
        shop_hardware
            .files
            .iter()
            .any(|file| file.path == format!("{normalized}/hardware/handles.stave"))
    );
    let imports: Vec<(&str, &str)> = root
        .files
        .iter()
        .flat_map(|file| &file.imports)
        .map(|import| {
            (
                import.name.as_str(),
                set.module(import.module).unwrap().name.as_str(),
            )
        })
        .collect();
    assert_eq!(
        imports,
        [
            ("casework", "casework"),
            ("hw", "hardware"),
            ("casework", "casework"),
            ("hw", "hardware")
        ]
    );
    assert!(matches!(set.modules[2].source, ModuleSource::Local { .. }));
}

#[test]
fn clothing_cabinet_display_paths_follow_the_given_folder() {
    // A relative folder keeps its spelling in display paths.
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let relative = Path::new("../../examples/clothing_cabinet");
    let set = ModuleSet::load(&manifest.join(relative)).unwrap();
    let base = manifest.join(relative).display().to_string();
    let shop_hardware = &set.modules[2];
    assert_eq!(
        shop_hardware.files[0].path,
        // `normalize` resolves the `..`s lexically.
        stave_modules::normalize(Path::new(&base))
            .join("hardware/handles.stave")
            .display()
            .to_string()
    );
}

#[test]
fn joint_samples_imports_japanese_joinery() {
    let folder = repository().join("examples/joint_samples");
    let set = ModuleSet::load(&folder).unwrap();
    assert!(!set.has_errors(), "{}", render(&set));
    let root = set.module(set.root).unwrap();
    let import = &root.files[0].imports[0];
    assert_eq!(import.name, "japanese");
    let japanese = set.module(import.module).unwrap();
    assert_eq!(japanese.name, "japanese_joinery");
    let expected = stave_modules::normalize(&folder.join("../japanese_joinery"))
        .join("library.stave")
        .display()
        .to_string();
    assert!(
        japanese.files.iter().any(|file| file.path == expected),
        "{expected}"
    );
    assert!(expected.ends_with("examples/japanese_joinery/library.stave"));
}

#[test]
fn every_example_and_std_folder_loads_without_errors() {
    let repository = repository();
    let mut folders = stave_folders(&repository.join("examples"));
    folders.extend(stave_folders(&repository.join("examples/clothing_cabinet")));
    folders.extend(stave_folders(&repository.join("std")));
    assert!(folders.len() >= 8, "{folders:?}");
    for folder in folders {
        let set = ModuleSet::load(&folder).unwrap();
        assert!(!set.has_errors(), "{}:\n{}", folder.display(), render(&set));
    }
}

#[test]
fn std_folders_are_the_embedded_modules() {
    for module in STD_MODULES {
        let set = ModuleSet::load(&repository().join("std").join(module)).unwrap();
        // The prelude, plus the module itself when it is not one of them.
        let outside_prelude = usize::from(!PRELUDE_MODULES.contains(&module));
        assert_eq!(set.modules.len(), PRELUDE_MODULES.len() + outside_prelude);
        assert_eq!(
            set.module(set.root).unwrap().source,
            ModuleSource::Std { name: module }
        );
    }
}

#[test]
fn broken_corpus_loads() {
    // Each design is one module, plus the three standard ones, plus the
    // local folders it imports (the designs for module errors have some).
    for folder in stave_folders(&repository().join("tests/broken")) {
        let set = ModuleSet::load(&folder).unwrap();
        let local = stave_folders(&folder).len();
        assert_eq!(set.modules.len(), 4 + local, "{}", folder.display());
    }
}

#[test]
fn relative_root_display_path_is_kept_as_given() {
    // Paths are relative to the crate when tests run.
    let set = ModuleSet::load(Path::new("../../tests/broken/missing_value")).unwrap();
    let root = set.module(set.root).unwrap();
    assert_eq!(
        root.files[0].path,
        "../../tests/broken/missing_value/project.stave"
    );
}
