//! Resolving a changed project with the libraries kept from the last
//! resolve ([`stave_model::resolve_reusing`]) is taken whenever only the
//! project's files changed, and gives exactly what [`stave_model::resolve`]
//! gives.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use stave_model::{resolve, resolve_keeping, resolve_reusing};
use stave_modules::{DiskFiles, FileSource, ModuleSet};

/// The disk, with some files' texts replaced.
struct Changed(BTreeMap<PathBuf, String>);

impl FileSource for Changed {
    fn list_stave_files(&self, folder: &Path) -> io::Result<Vec<PathBuf>> {
        DiskFiles.list_stave_files(folder)
    }

    fn read(&self, path: &Path) -> io::Result<String> {
        let key = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        match self.0.get(&key) {
            Some(text) => Ok(text.clone()),
            None => DiskFiles.read(path),
        }
    }

    fn canonical(&self, folder: &Path) -> io::Result<PathBuf> {
        DiskFiles.canonical(folder)
    }
}

fn examples() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut folders: Vec<PathBuf> = std::fs::read_dir(root)
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    folders.sort();
    assert!(!folders.is_empty());
    folders
}

#[test]
fn the_kept_libraries_are_reused_and_the_result_is_the_same() {
    for folder in examples() {
        let what = folder.display().to_string();
        let modules = ModuleSet::load(&folder).unwrap();
        let (defs, diagnostics, kept) = resolve_keeping(&modules);
        assert_eq!((defs, diagnostics), resolve(&modules), "{what}");
        let kept = kept.expect("an example's libraries can be kept");
        assert!(kept.matches(&modules), "{what}");

        // The project's first file changed: a new item, and a use of a
        // name that doesn't exist.
        let mut files: Vec<PathBuf> = DiskFiles.list_stave_files(&folder).unwrap();
        files.sort();
        let first = files.first().unwrap();
        let text = std::fs::read_to_string(first).unwrap();
        let changed = Changed(BTreeMap::from([(
            first.clone(),
            format!(
                "{text}\nparam reuse_extra = 3mm\nlet reuse_twice = reuse_extra * 2 + nowhere\n"
            ),
        )]));
        let again = modules.reload_with(&folder, &changed).unwrap();
        assert_eq!(
            again.sources,
            ModuleSet::load_with(&folder, &changed).unwrap().sources,
            "{what}"
        );
        let reused = resolve_reusing(&again, &kept).expect("only the project changed");
        assert_eq!(reused, resolve(&again), "{what}");
    }
}

#[test]
fn libraries_that_changed_are_not_reused() {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/clothing_cabinet");
    let modules = ModuleSet::load(&folder).unwrap();
    let (_, _, kept) = resolve_keeping(&modules);
    let kept = kept.unwrap();
    // A file of the imported `casework` library changed.
    let library = modules
        .modules
        .iter()
        .find(|module| module.name == "casework")
        .expect("the example imports casework");
    let stave_modules::ModuleSource::Local {
        folder: library_folder,
    } = &library.source
    else {
        panic!("casework is a local module");
    };
    let file = library.files.first().unwrap();
    let name = Path::new(&file.path).file_name().unwrap();
    let text = modules.sources.get(file.file).unwrap().text().to_owned();
    let changed = Changed(BTreeMap::from([(
        library_folder.join(name),
        format!("{text}\n// changed\n"),
    )]));
    let again = modules.reload_with(&folder, &changed).unwrap();
    assert_ne!(again.sources, modules.sources, "the change was read");
    assert!(!kept.matches(&again));
    assert!(resolve_reusing(&again, &kept).is_none());

    // A standard module as the folder: nothing is kept apart.
    let std_folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../std/joinery");
    let modules = ModuleSet::load(&std_folder).unwrap();
    let (defs, diagnostics, kept) = resolve_keeping(&modules);
    assert!(kept.is_none());
    assert_eq!((defs, diagnostics), resolve(&modules));
}

/// Prints how long loading and resolving take in full and reusing, on
/// `examples/clothing_cabinet` (the table in the crate docs). Run it in a
/// release build; it asserts nothing about time.
#[test]
#[ignore = "a measurement, not a check: run with --release -- --ignored --nocapture"]
fn timing_on_the_clothing_cabinet() {
    use std::time::{Duration, Instant};
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/clothing_cabinet");
    let file = std::fs::canonicalize(folder.join("cabinet.stave")).unwrap();
    let text = std::fs::read_to_string(&file).unwrap();
    let changed = Changed(BTreeMap::from([(file, format!("{text}\n// changed\n"))]));
    let modules = ModuleSet::load(&folder).unwrap();
    let (_, _, kept) = resolve_keeping(&modules);
    let kept = kept.unwrap();
    let again = modules.reload_with(&folder, &changed).unwrap();
    let runs = 200_u32;
    let mean = |work: &dyn Fn()| -> Duration {
        for _ in 0..20 {
            work();
        }
        let started = Instant::now();
        for _ in 0..runs {
            work();
        }
        started.elapsed() / runs
    };
    let load = mean(&|| drop(ModuleSet::load_with(&folder, &changed).unwrap()));
    let reload = mean(&|| drop(modules.reload_with(&folder, &changed).unwrap()));
    let full = mean(&|| drop(resolve(&again)));
    let reusing = mean(&|| drop(resolve_reusing(&again, &kept).unwrap()));
    println!("load and parse: full {load:?}, reusing {reload:?}");
    println!("resolve: full {full:?}, reusing {reusing:?}");
}
