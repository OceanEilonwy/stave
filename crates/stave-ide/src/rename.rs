//! Renaming an item and every reference to it, and what depends on an
//! item: the two questions a `rename` patch and a removal ask of the
//! names (grammar, "Patches"; `docs/editing.md`, "Remove").
//!
//! - [`Analysis::rename`] gives, for every file of the project that
//!   names a symbol, the text edits that rename it there: its declaration,
//!   each use (a placement, a joint's role, a scene's pose, a field key of
//!   an assembly for a component param, a `{name}` in a message) and the
//!   labels' uses through it. A new name that is not a word, is reserved,
//!   or would clash with a name already visible where the symbol is
//!   declared or used is refused, so a rename never changes what another
//!   name means.
//! - [`Analysis::dependants`] lists the project's items whose text names
//!   an item (or one of its labels), and the items that name those, and so
//!   on: everything that would be left naming nothing if the item were
//!   removed.
//!
//! Both read the index only; they write nothing.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use stave_lang::ast::Span;
use stave_lang::{FileId, TextEdit};

use crate::analysis::{Analysis, Location};
use crate::document::Document;
use crate::index::{Symbol, SymbolId, SymbolKind, UseKind};
use crate::words;

/// The edits a rename makes in one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenameEdit {
    /// The file.
    pub file: FileId,
    /// The document it is.
    pub document: Document,
    /// The edits, last in the file first, as
    /// [`stave_lang::apply_text_edits`] takes them.
    pub edits: Vec<TextEdit>,
    /// The file's text after the edits.
    pub text: String,
}

/// Why a rename is refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RenameError {
    /// The new name is not a word: letters, digits and `_`, starting with
    /// a letter.
    #[error("'{0}' is not a name: a name is letters, digits and '_', starting with a letter")]
    NotAName(String),
    /// The new name is a keyword, a built-in function's name or a fixed
    /// word of the language.
    #[error("'{0}' is a word of the language, so it can't name an item")]
    Reserved(String),
    /// The new name is the old one.
    #[error("'{0}' already has that name")]
    Unchanged(String),
    /// The symbol is not one the project declares (the standard library's,
    /// or an imported module's), so its text is not the project's to change.
    #[error("'{0}' is declared outside this project, so it can't be renamed here")]
    ReadOnly(String),
    /// A module or a project header, which a rename doesn't change.
    #[error("'{name}' is a {kind}, which can't be renamed")]
    NotRenamable {
        /// Its name.
        name: String,
        /// What it is.
        kind: &'static str,
    },
    /// The new name is already used where the symbol is declared or used.
    #[error("'{name}' is already {what}; choose another name")]
    Clash {
        /// The new name.
        name: String,
        /// What has the name, in words: "the name of part side_l".
        what: String,
        /// Where that is declared, when it is in a file.
        existing: Option<Location>,
    },
    /// The symbol is not in this analysis.
    #[error("there is no such name in this project")]
    Unknown,
}

impl Analysis {
    /// The project's own top-level items (not modules, not what is
    /// declared inside a component or kind), in file and position order.
    pub fn project_items(&self) -> Vec<&Symbol> {
        let mut items: Vec<&Symbol> = self
            .index
            .symbols
            .iter()
            .filter(|s| {
                s.module == self.index.root && s.container.is_none() && s.kind != SymbolKind::Module
            })
            .collect();
        items.sort_by_key(|s| (s.file, s.span.start()));
        items
    }

    /// The edits that rename a symbol to `to` in every file that names it.
    /// Files come in file order; a file that doesn't name it is left out.
    ///
    /// # Errors
    ///
    /// A new name that is not a word, is reserved or is already taken
    /// where the symbol is declared or used; a symbol the project doesn't
    /// declare; see [`RenameError`].
    pub fn rename(&self, symbol: SymbolId, to: &str) -> Result<Vec<RenameEdit>, RenameError> {
        let target = self.symbol(symbol).ok_or(RenameError::Unknown)?;
        if target.kind == SymbolKind::Module {
            return Err(RenameError::NotRenamable {
                name: target.name.clone(),
                kind: target.kind.word(),
            });
        }
        if target.module != self.index.root {
            return Err(RenameError::ReadOnly(target.name.clone()));
        }
        if !stave_lang::ast::is_word(to) {
            return Err(RenameError::NotAName(to.to_owned()));
        }
        if stave_lang::is_keyword(to)
            || stave_lang::BUILTIN_FUNCTIONS.contains(&to)
            || words::is_enum_word(to)
            || words::is_fixed_call(to)
        {
            return Err(RenameError::Reserved(to.to_owned()));
        }
        if target.name == to {
            return Err(RenameError::Unchanged(to.to_owned()));
        }
        self.check_clash(target, to)?;

        // Every span that writes the name, by file.
        let mut spans: BTreeMap<FileId, BTreeSet<(u32, u32)>> = BTreeMap::new();
        for reference in self.references_to(symbol) {
            spans
                .entry(reference.0)
                .or_default()
                .insert((reference.1.start(), reference.1.end()));
        }
        let mut out = Vec::new();
        for (file, spans) in spans {
            let Some(text) = self.text(file) else {
                continue;
            };
            let Some(document) = self.document(file) else {
                continue;
            };
            let mut edits: Vec<TextEdit> = spans
                .into_iter()
                .filter_map(|(start, end)| {
                    let range = usize::try_from(start).ok()?..usize::try_from(end).ok()?;
                    // Only where the text is the name itself: a module's
                    // title or a recovered node is never rewritten.
                    (text.get(range.clone())? == target.name).then(|| TextEdit {
                        range,
                        text: to.to_owned(),
                    })
                })
                .collect();
            if edits.is_empty() {
                continue;
            }
            edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
            let Some(new_text) = stave_lang::apply_text_edits(text, &edits) else {
                continue;
            };
            out.push(RenameEdit {
                file,
                document,
                edits,
                text: new_text,
            });
        }
        Ok(out)
    }

    /// Every place a symbol's name is written: its declaration and its
    /// uses, as file and span.
    fn references_to(&self, symbol: SymbolId) -> Vec<(FileId, Span)> {
        self.index
            .uses
            .get(&symbol)
            .map(|positions| {
                positions
                    .iter()
                    .filter_map(|position| self.index.references.get(*position))
                    .map(|reference| (reference.file, reference.span))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Refuses a new name that something already has where the symbol
    /// lives or is used.
    fn check_clash(&self, target: &Symbol, to: &str) -> Result<(), RenameError> {
        let clash = |found: SymbolId| {
            let what = self
                .symbol(found)
                .map(|s| format!("the name of {} {}", s.kind.word(), s.name))
                .unwrap_or_else(|| "taken".to_owned());
            RenameError::Clash {
                name: to.to_owned(),
                what,
                existing: self.declaration(found),
            }
        };
        let key = to.to_owned();
        match target.container {
            Some(container) => {
                if let Some(found) = self.index.inner_scope.get(&(container, key.clone())) {
                    return Err(clash(*found));
                }
            }
            None => {
                if let Some(found) = self.index.module_scope.get(&(target.module, key.clone())) {
                    return Err(clash(*found));
                }
                let module_files = self
                    .index
                    .modules
                    .get(&target.module)
                    .map(|m| m.files.clone())
                    .unwrap_or_default();
                for file in module_files {
                    if self.index.imports.contains_key(&(file, key.clone())) {
                        return Err(RenameError::Clash {
                            name: to.to_owned(),
                            what: "the name a file of the project imports a module under"
                                .to_owned(),
                            existing: None,
                        });
                    }
                }
                // A name of the prelude would be hidden everywhere in the
                // project, changing what other uses of it mean.
                for prelude in &self.index.prelude {
                    if let Some(found) = self.index.module_scope.get(&(*prelude, key.clone()))
                        && self.symbol(*found).is_some_and(|s| !s.private)
                    {
                        return Err(clash(*found));
                    }
                }
            }
        }
        // A use inside a component or kind body that declares the new name
        // would mean that member after the rename.
        for (file, span) in self.references_to(target.id) {
            for body in self.index.symbols.iter().filter(|s| {
                s.file == file
                    && matches!(s.kind, SymbolKind::Component | SymbolKind::Kind)
                    && s.span.start() <= span.start()
                    && span.end() <= s.span.end()
                    && Some(s.id) != target.container
            }) {
                if let Some(found) = self.index.inner_scope.get(&(body.id, key.clone())) {
                    return Err(clash(*found));
                }
            }
        }
        Ok(())
    }

    /// The project's top-level items that depend on `symbol`: those whose
    /// text names it or one of its labels, then those that name those, and
    /// so on. Nearest first: the items that name it directly, then the
    /// items that name those, each step in file and position order. The
    /// item itself is not listed.
    pub fn dependants(&self, symbol: SymbolId) -> Vec<SymbolId> {
        let items = self.project_items();
        let mut seen: BTreeSet<SymbolId> = BTreeSet::from([symbol]);
        let mut out = Vec::new();
        let mut queue = VecDeque::from([symbol]);
        while let Some(current) = queue.pop_front() {
            let mut direct: Vec<&Symbol> = Vec::new();
            for (file, span) in self.uses_of_item(current) {
                let Some(item) = items.iter().find(|s| {
                    s.file == file && s.span.start() <= span.start() && span.end() <= s.span.end()
                }) else {
                    continue;
                };
                if seen.insert(item.id) {
                    direct.push(item);
                }
            }
            direct.sort_by_key(|s| (s.file, s.span.start()));
            for item in direct {
                out.push(item.id);
                queue.push_back(item.id);
            }
        }
        out
    }

    /// Where an item or one of its labels is used, its declarations left
    /// out.
    fn uses_of_item(&self, symbol: SymbolId) -> Vec<(FileId, Span)> {
        let labels = self
            .index
            .symbols
            .iter()
            .filter(|s| s.container == Some(symbol) && s.kind == SymbolKind::Label)
            .map(|s| s.id);
        std::iter::once(symbol)
            .chain(labels)
            .filter_map(|id| self.index.uses.get(&id))
            .flatten()
            .filter_map(|position| self.index.references.get(*position))
            .filter(|reference| reference.kind != UseKind::Declared)
            .map(|reference| (reference.file, reference.span))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use stave_modules::MemoryFiles;

    use super::RenameError;
    use crate::{Analysis, Document};

    const FOLDER: &str = "rename/project";

    fn project(files: &[(&str, &str)]) -> (Analysis, MemoryFiles) {
        let mut memory = MemoryFiles::new();
        for (name, text) in files {
            memory.insert(Path::new(FOLDER).join(name), (*text).to_owned());
        }
        let analysis = Analysis::load(Path::new(FOLDER), &memory).unwrap();
        (analysis, memory)
    }

    const MAIN: &str = r#"stave 1

project "Box" { units: mm }

param width = 400mm

stock oak_19 { species: white_oak, form: solid, thickness: 19mm, max_width: 200mm, max_length: 2700mm }

part base : oak_19 {
  length: width
  width:  200mm
  place {
    ground
  }
}

require wide : width >= 300mm "room for {base}"
"#;

    const SIDES: &str = r#"stave 1

part side_l : oak_19 {
  length: 300mm
  width:  base.width
  place {
    orient(length: up, width: back)
    against(base.top)
    flush(base.left)
    flush(base.back)
  }
}

part side_r = mirror(side_l, about: mid(base, across))
"#;

    fn path_of(document: &Document) -> PathBuf {
        match document {
            Document::File(path) => path.clone(),
            Document::Std { .. } => panic!("a standard file was edited"),
        }
    }

    #[test]
    fn a_rename_reaches_every_file_and_use() {
        let (analysis, _) = project(&[("main.stave", MAIN), ("sides.stave", SIDES)]);
        let base = analysis.project_symbol("base").unwrap().id;
        let edits = analysis.rename(base, "bottom_board").unwrap();
        assert_eq!(edits.len(), 2);
        for edit in &edits {
            assert!(!edit.text.contains("base"), "{}", edit.text);
            assert!(edit.text.contains("bottom_board"));
            let old = analysis.text(edit.file).unwrap();
            assert_eq!(
                stave_lang::apply_text_edits(old, &edit.edits).as_deref(),
                Some(edit.text.as_str())
            );
            assert!(path_of(&edit.document).starts_with(FOLDER));
        }
        let main = edits
            .iter()
            .find(|e| path_of(&e.document).ends_with("main.stave"))
            .unwrap();
        // The message's `{base}` and the field key `width:` of a part are
        // different things: only the name changes.
        assert!(main.text.contains("\"room for {bottom_board}\""));
        assert!(main.text.contains("param width = 400mm"));
    }

    #[test]
    fn a_rename_that_clashes_is_refused() {
        let (analysis, _) = project(&[("main.stave", MAIN), ("sides.stave", SIDES)]);
        let base = analysis.project_symbol("base").unwrap().id;
        assert!(matches!(
            analysis.rename(base, "side_l"),
            Err(RenameError::Clash { .. })
        ));
        assert!(matches!(
            analysis.rename(base, "wide"),
            Err(RenameError::Clash { .. })
        ));
        // A name of the standard prelude would hide it.
        assert!(matches!(
            analysis.rename(base, "dado"),
            Err(RenameError::Clash { .. })
        ));
        assert!(matches!(
            analysis.rename(base, "part"),
            Err(RenameError::Reserved(_))
        ));
        assert!(matches!(
            analysis.rename(base, "split"),
            Err(RenameError::Reserved(_))
        ));
        assert!(matches!(
            analysis.rename(base, "2x"),
            Err(RenameError::NotAName(_))
        ));
        assert!(matches!(
            analysis.rename(base, "base"),
            Err(RenameError::Unchanged(_))
        ));
        let dado = analysis
            .search("dado", 20)
            .into_iter()
            .find(|found| found.label == "dado")
            .unwrap();
        let dado = analysis
            .symbol_at(crate::FilePosition {
                file: dado.location.file,
                offset: dado.location.span.start(),
            })
            .unwrap();
        assert!(matches!(
            analysis.rename(dado, "groove"),
            Err(RenameError::ReadOnly(_))
        ));
    }

    #[test]
    fn dependants_follow_names_transitively() {
        let (analysis, _) = project(&[("main.stave", MAIN), ("sides.stave", SIDES)]);
        let name = |id| analysis.symbol(id).unwrap().name.clone();
        let side_l = analysis.project_symbol("side_l").unwrap().id;
        let names: Vec<String> = analysis.dependants(side_l).into_iter().map(name).collect();
        assert_eq!(names, ["side_r"]);
        let base = analysis.project_symbol("base").unwrap().id;
        let names: Vec<String> = analysis.dependants(base).into_iter().map(name).collect();
        assert_eq!(names, ["wide", "side_l", "side_r"]);
        let wide = analysis.project_symbol("wide").unwrap().id;
        assert!(analysis.dependants(wide).is_empty());
    }
}
