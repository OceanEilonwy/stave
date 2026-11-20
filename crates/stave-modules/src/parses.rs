//! The parse query: a file's parse, kept from one load to the next.

use std::collections::{BTreeMap, btree_map};

use stave_lang::ast::File;

use crate::ModuleSet;
use stave_lang::{
    BrokenItem, Diagnostic, FileId, GreenNode, is_patch_source, parse_file, parse_patch_file,
};

/// Every file's parse, kept from one load to the next so a load parses
/// only the files whose text changed ([`crate::ModuleSet::load_cached`]).
///
/// Parsing is a function of a file's id and text alone (the id is in
/// every span and diagnostic it gives), so a parse is reused exactly when
/// both are the same as when it was made; any change of either parses the
/// file again. Unlike [`crate::ModuleSet::reload_with`], a file with
/// syntax errors is reused too: its diagnostics are kept with its tree.
/// Entries for files the last load did not reach are dropped.
///
/// The lossless tree of each file is kept as well ([`Parses::green`]), so
/// an editor reads it from here instead of parsing again.
///
/// The typed trees are not copied: a load moves each file's typed tree
/// into the [`crate::ModuleSet`] it gives. Hand the set back with
/// [`Parses::reclaim`] once done with it, and the next load moves them out
/// again; a file whose tree was not handed back is parsed again.
#[derive(Debug, Clone, Default)]
pub struct Parses {
    entries: BTreeMap<FileId, Entry>,
    /// The files the last load parsed (not reused), in the order parsed.
    parsed: Vec<FileId>,
}

#[derive(Debug, Clone)]
struct Entry {
    text: String,
    parsed: Parsed,
    /// Reached by the load in progress.
    used: bool,
}

/// What parsing one file gave.
#[derive(Debug, Clone)]
pub(crate) enum Parsed {
    /// A file of items.
    File {
        green: GreenNode,
        ast: Option<File>,
        /// `ast` has been moved into a module set and not handed back.
        lent: bool,
        diagnostics: Vec<Diagnostic>,
        broken: Vec<BrokenItem>,
    },
    /// A patch file: only its diagnostics are kept by the loader.
    Patch {
        green: GreenNode,
        diagnostics: Vec<Diagnostic>,
    },
}

impl Parsed {
    /// Parses a file.
    pub(crate) fn new(file: FileId, text: &str) -> Parsed {
        if is_patch_source(text) {
            let parse = parse_patch_file(file, text);
            Parsed::Patch {
                green: parse.green,
                diagnostics: parse.diagnostics,
            }
        } else {
            let parse = parse_file(file, text);
            Parsed::File {
                green: parse.green,
                ast: parse.ast,
                lent: false,
                diagnostics: parse.diagnostics,
                broken: parse.broken,
            }
        }
    }

    fn is_lent(&self) -> bool {
        match self {
            Parsed::File { lent, .. } => *lent,
            Parsed::Patch { .. } => false,
        }
    }

    fn green(&self) -> &GreenNode {
        match self {
            Parsed::File { green, .. } | Parsed::Patch { green, .. } => green,
        }
    }
}

impl Parses {
    /// No parses yet.
    pub fn new() -> Parses {
        Parses::default()
    }

    /// The lossless tree of a file the last load reached.
    pub fn green(&self, file: FileId) -> Option<&GreenNode> {
        self.entries.get(&file).map(|entry| entry.parsed.green())
    }

    /// The files the last load parsed, rather than reused, in the order
    /// it parsed them: after an edit to one file, just that file.
    pub fn parsed(&self) -> &[FileId] {
        &self.parsed
    }

    /// Starts a load: nothing reached yet.
    pub(crate) fn begin(&mut self) {
        self.parsed.clear();
        for entry in self.entries.values_mut() {
            entry.used = false;
        }
    }

    /// Ends a load: forgets the files it did not reach.
    pub(crate) fn end(&mut self) {
        self.entries.retain(|_, entry| entry.used);
    }

    /// The parse of a file with this id and text: the kept one when the
    /// text is the same, otherwise a new one, kept for the next load.
    pub(crate) fn parse(&mut self, file: FileId, text: &str) -> &mut Parsed {
        let fresh = || Entry {
            text: text.to_owned(),
            parsed: Parsed::new(file, text),
            used: true,
        };
        let entry = match self.entries.entry(file) {
            btree_map::Entry::Occupied(kept)
                if kept.get().text == text && !kept.get().parsed.is_lent() =>
            {
                kept.into_mut()
            }
            btree_map::Entry::Occupied(mut stale) => {
                self.parsed.push(file);
                stale.insert(fresh());
                stale.into_mut()
            }
            btree_map::Entry::Vacant(vacant) => {
                self.parsed.push(file);
                vacant.insert(fresh())
            }
        };
        entry.used = true;
        &mut entry.parsed
    }

    /// Takes back the typed trees a load moved into `set`
    /// ([`crate::ModuleSet::load_cached`]), so the next load can reuse
    /// them; the set is left without them (every [`crate::ModuleFile`]'s
    /// `ast` is `None`). A tree is taken back only for a file whose id and
    /// text are still the ones it was parsed from.
    pub fn reclaim(&mut self, set: &mut ModuleSet) {
        for file in set.modules.iter_mut().flat_map(|module| &mut module.files) {
            let Some(entry) = self.entries.get_mut(&file.file) else {
                continue;
            };
            let same_text = set
                .sources
                .get(file.file)
                .is_some_and(|source| source.text() == entry.text);
            if let Parsed::File { ast, lent, .. } = &mut entry.parsed
                && *lent
                && same_text
            {
                *ast = file.ast.take();
                *lent = false;
            }
        }
    }
}
