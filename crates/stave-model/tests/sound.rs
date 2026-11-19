//! A declaration with a name or type error is poisoned, and the accessors
//! a later stage evaluates through ([`stave_model::Defs::sound_part`] and
//! the rest) refuse it, while the plain accessors still show it to the
//! editor.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use stave_model::defs::{DefId, Unusable};
use stave_model::ids::PartDefId;
use stave_model::resolve;
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

fn part_named(defs: &stave_model::Defs, name: &str) -> PartDefId {
    let index = defs
        .parts
        .iter()
        .position(|part| part.decl.name == name)
        .unwrap();
    PartDefId::from_usize(index)
}

#[test]
fn a_poisoned_part_is_refused_by_the_sound_accessor_and_its_neighbours_are_not() {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/wall_shelf");
    let file = std::fs::canonicalize(folder.join("shelf.stave")).unwrap();
    let text = std::fs::read_to_string(&file).unwrap();
    let broken = text.replacen("length: shelf_length", "length: shelf_lenght", 1);
    assert_ne!(
        broken, text,
        "the example still has the line this test breaks"
    );
    let changed = Changed(BTreeMap::from([(file, broken)]));
    let modules = ModuleSet::load_with(&folder, &changed).unwrap();
    let (defs, diagnostics) = resolve(&modules);
    assert!(diagnostics.iter().any(|d| d.is_error()));

    let shelf = part_named(&defs, "shelf");
    assert!(defs.is_poisoned(DefId::Part(shelf)));
    assert!(defs.part(shelf).is_some(), "the editor still sees it");
    assert_eq!(defs.sound_part(shelf).err(), Some(Unusable::Poisoned));
    for (index, part) in defs.parts.iter().enumerate() {
        let id = PartDefId::from_usize(index);
        if id != shelf {
            assert!(defs.sound_part(id).is_ok(), "{}", part.decl.name);
        }
    }
    let past_the_end = PartDefId::from_usize(defs.parts.len());
    assert_eq!(defs.sound_part(past_the_end).err(), Some(Unusable::Missing));
}

#[test]
fn every_declaration_of_a_clean_example_is_sound() {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/wall_shelf");
    let modules = ModuleSet::load(&folder).unwrap();
    let (defs, diagnostics) = resolve(&modules);
    assert!(!diagnostics.iter().any(|d| d.is_error()));
    assert!(!defs.any_poisoned());
    for index in 0..defs.parts.len() {
        assert!(defs.sound_part(PartDefId::from_usize(index)).is_ok());
    }
}
