//! The queries an editor asks: what a name at a byte offset means, where
//! it is declared, every use of it, what to show on hover, where the
//! cursor is (breadcrumbs), the colours of a file, the lenses above its
//! declarations, and its diagnostics.
//!
//! Positions are byte offsets into a file's text ([`FilePosition`]);
//! lines are worked out only for what is shown ([`Location::line`]).

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use stave_lang::ast::Span;
use stave_lang::{Diagnostic, FileId, SyntaxKind, SyntaxNode};
use stave_modules::{FileSource, LoadError, ModuleSet};
use stave_schema::words::WordSet;

use crate::completion::Completion;
use crate::document::Document;
use crate::host::{AnalysisHost, build_fresh};
use crate::index::{
    FileData, Index, Reference, Symbol, SymbolId, SymbolKind, UseKind, declared_name, span_of,
    word_of,
};
use crate::tokens::SemanticToken;
use crate::words;

/// A byte offset in a loaded file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FilePosition {
    /// The file.
    pub file: FileId,
    /// The byte offset into its text.
    pub offset: u32,
}

/// A span in a document, with its line worked out.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Location {
    /// The file.
    pub file: FileId,
    /// The document it is.
    pub document: Document,
    /// The span.
    pub span: Span,
    /// The span's first line, from 1.
    pub line: usize,
    /// The span's first column, in characters, from 1.
    pub column: usize,
}

impl Location {
    /// `casework/frame_panel_door.stave:4`.
    pub fn label(&self, folder: &Path) -> String {
        format!("{}:{}", self.document.display(folder), self.line)
    }
}

/// One use of a name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Use {
    /// Where it is.
    pub location: Location,
    /// What it does there.
    pub kind: UseKind,
    /// What it does there, in words: "declared", "door_r mirrors it",
    /// "scene all_open poses it".
    pub description: String,
    /// The line it is on, trimmed.
    pub line_text: String,
}

/// Every use of a symbol: its declaration first, then the others by file
/// and position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Uses {
    /// The symbol.
    pub symbol: SymbolId,
    /// Its name.
    pub name: String,
    /// Its uses.
    pub uses: Vec<Use>,
}

impl Uses {
    /// How many uses there are besides the declaration.
    pub fn count(&self) -> usize {
        self.uses
            .iter()
            .filter(|u| u.kind != UseKind::Declared)
            .count()
    }
}

/// What to show when the pointer rests on a word.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hover {
    /// The word.
    pub span: Span,
    /// What it is: "part", "built-in function", "an axis".
    pub kind: String,
    /// Its declaration on one line: `component FramePanelDoor`.
    pub signature: String,
    /// Its doc comment, as one paragraph.
    pub docs: Option<String>,
    /// Where it is declared, for a symbol.
    pub location: Option<Location>,
    /// What it works out to in the project's units (`= 844mm`), when the
    /// background check has worked it out ([`Analysis::hover_with_values`]).
    pub value: Option<String>,
}

/// One step of the breadcrumbs: `doors.stave › assembly door_l › mount`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Crumb {
    /// Its words.
    pub label: String,
    /// What it covers.
    pub span: Span,
}

/// The note above a declaration: "2 uses · show in 3D · show work card".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lens {
    /// The declaration.
    pub symbol: SymbolId,
    /// What it declares.
    pub kind: SymbolKind,
    /// Its name.
    pub name: String,
    /// Where its name is.
    pub location: Location,
    /// How many uses it has besides its declaration.
    pub uses: usize,
}

impl Lens {
    /// "2 uses", "1 use", "no uses".
    pub fn uses_text(&self) -> String {
        match self.uses {
            0 => "no uses".to_owned(),
            1 => "1 use".to_owned(),
            n => format!("{n} uses"),
        }
    }

    /// `true` when the 3D view can show it.
    pub fn shows_in_3d(&self) -> bool {
        self.kind.is_shown_in_3d()
    }

    /// `true` when it has a work card: a part (grammar, "Tooling and
    /// editors": "show work card" on parts).
    pub fn shows_work_card(&self) -> bool {
        self.kind == SymbolKind::Part
    }
}

/// A result of "Go to file or name".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// The name or file name.
    pub label: String,
    /// "part · carcass.stave", "file".
    pub detail: String,
    /// Where it goes.
    pub location: Location,
}

/// One loaded project, indexed for the editor. Build it again when a
/// file changes, through an [`AnalysisHost`] to reuse what the change
/// did not touch; it reads open buffers through the [`FileSource`] it is
/// loaded with ([`crate::Overlay`]). Cloning it is cheap: clones share
/// the index and what has been worked out from it.
#[derive(Debug, Clone)]
pub struct Analysis {
    pub(crate) index: Arc<Index>,
    /// Completions already worked out, by position.
    completions: Arc<Mutex<BTreeMap<FilePosition, Vec<Completion>>>>,
}

/// How many positions' completions an analysis keeps.
const KEPT_COMPLETIONS: usize = 64;

impl Analysis {
    /// Loads and indexes a project folder, working everything out. An
    /// editor that loads again after every change keeps an
    /// [`AnalysisHost`] instead, which gives the same analysis with less
    /// work.
    pub fn load(folder: &Path, files: &dyn FileSource) -> Result<Analysis, LoadError> {
        AnalysisHost::new().analyse(folder, files)
    }

    /// Indexes modules already loaded.
    pub fn from_modules(folder: &Path, set: ModuleSet) -> Analysis {
        Analysis::from_index(build_fresh(folder, set))
    }

    pub(crate) fn from_index(index: Index) -> Analysis {
        Analysis {
            index: Arc::new(index),
            completions: Arc::default(),
        }
    }

    /// The completions at a position, worked out once per analysis.
    pub(crate) fn kept_completions(
        &self,
        position: FilePosition,
        work_out: impl FnOnce() -> Vec<Completion>,
    ) -> Vec<Completion> {
        if let Some(kept) = self
            .completions
            .lock()
            .ok()
            .and_then(|kept| kept.get(&position).cloned())
        {
            return kept;
        }
        let completions = work_out();
        if let Ok(mut kept) = self.completions.lock() {
            if kept.len() >= KEPT_COMPLETIONS {
                kept.clear();
            }
            kept.insert(position, completions.clone());
        }
        completions
    }

    /// The project folder.
    pub fn folder(&self) -> &Path {
        &self.index.folder
    }

    /// The file a document is loaded as, if it is.
    pub fn file(&self, document: &Document) -> Option<FileId> {
        let path = document.source_path();
        self.index
            .files
            .iter()
            .find(|(_, data)| data.path == path)
            .map(|(id, _)| *id)
    }

    /// The document a loaded file is.
    pub fn document(&self, file: FileId) -> Option<Document> {
        self.index
            .files
            .get(&file)
            .map(|data| Document::from_source_path(&data.path))
    }

    /// A loaded file's text.
    pub fn text(&self, file: FileId) -> Option<&str> {
        self.index.sources.get(file).map(|s| s.text())
    }

    /// The symbol with this id.
    pub fn symbol(&self, id: SymbolId) -> Option<&Symbol> {
        self.index.symbol(id)
    }

    /// `true` if a symbol is declared in the project's own folder (not in
    /// a module it imports or the standard library).
    pub fn is_in_project(&self, id: SymbolId) -> bool {
        self.symbol(id).is_some_and(|s| s.module == self.index.root)
    }

    /// The symbol a name declares, by module-level name in the project's
    /// own folder: `door_l`.
    pub fn project_symbol(&self, name: &str) -> Option<&Symbol> {
        self.index
            .symbols()
            .find(|s| s.module == self.index.root && s.container.is_none() && s.name == name)
    }

    /// A location for a span of a file.
    pub fn location(&self, file: FileId, span: Span) -> Option<Location> {
        let source = self.index.sources.get(file)?;
        let position = source.line_column(span.start());
        Some(Location {
            file,
            document: self.document(file)?,
            span,
            line: position.line,
            column: position.column,
        })
    }

    /// The byte offset of a 1-based line's start.
    pub fn line_start(&self, file: FileId, line: usize) -> Option<u32> {
        let text = self.text(file)?;
        let mut offset = 0usize;
        for _ in 1..line {
            offset = offset
                .checked_add(text.get(offset..)?.find('\n')?)?
                .checked_add(1)?;
        }
        u32::try_from(offset).ok()
    }

    // ---- names ----

    /// The reference under a position: a name covers the offsets from its
    /// first byte to just after its last, so a cursor at the end of a word
    /// is on it.
    fn reference_at(&self, position: FilePosition) -> Option<&Reference> {
        let references = self.index.references(position.file);
        let after = references.partition_point(|r| r.span.start() <= position.offset);
        let candidate = references.get(after.checked_sub(1)?)?;
        (candidate.span.end() >= position.offset).then_some(candidate)
    }

    /// The symbol a name at a position means (or declares).
    pub fn symbol_at(&self, position: FilePosition) -> Option<SymbolId> {
        self.reference_at(position).map(|r| r.symbol)
    }

    /// Where the name at a position is declared.
    pub fn definition(&self, position: FilePosition) -> Option<Location> {
        let symbol = self.symbol(self.symbol_at(position)?)?;
        self.location(symbol.file, symbol.name_span)
    }

    /// Where a symbol is declared.
    pub fn declaration(&self, id: SymbolId) -> Option<Location> {
        let symbol = self.symbol(id)?;
        self.location(symbol.file, symbol.name_span)
    }

    /// Every use of a symbol.
    pub fn uses(&self, id: SymbolId) -> Option<Uses> {
        let symbol = self.symbol(id)?;
        let mut uses: Vec<Use> = self
            .index
            .uses()
            .get(&id)
            .map(|positions| {
                positions
                    .iter()
                    .filter_map(|position| self.index.reference(*position))
                    .filter_map(|reference| {
                        let location = self.location(reference.file, reference.span)?;
                        let line_text = self
                            .index
                            .sources
                            .get(reference.file)
                            .map(|s| s.line_text(location.line).trim().to_owned())
                            .unwrap_or_default();
                        Some(Use {
                            description: self.describe(&reference.kind, symbol),
                            kind: reference.kind.clone(),
                            location,
                            line_text,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        uses.sort_by_key(|u| u.kind != UseKind::Declared);
        Some(Uses {
            symbol: id,
            name: symbol.name.clone(),
            uses,
        })
    }

    /// Every use of the name at a position.
    pub fn uses_at(&self, position: FilePosition) -> Option<Uses> {
        self.uses(self.symbol_at(position)?)
    }

    /// What a use does, in words.
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "the variants it does not name take the general path written last"
    )]
    fn describe(&self, kind: &UseKind, target: &Symbol) -> String {
        let name = |id: &SymbolId| self.symbol(*id).map_or("", |s| s.name.as_str());
        let kind_name = |id: &SymbolId| {
            self.symbol(*id)
                .map_or_else(String::new, |s| format!("{} {}", s.kind.word(), s.name))
        };
        match kind {
            UseKind::Declared => "declared".to_owned(),
            UseKind::Imported => "imported".to_owned(),
            UseKind::Mirrored(by) => format!("{} mirrors it", name(by)),
            UseKind::Posed(scene) => format!("scene {} poses it", name(scene)),
            UseKind::SceneField(scene, field) => match field.as_str() {
                "show" => format!("scene {} shows it", name(scene)),
                "hide" => format!("scene {} hides it", name(scene)),
                _ => format!("scene {} · {field}", name(scene)),
            },
            UseKind::TypeOf(item) if target.kind == SymbolKind::Module => {
                format!("{} uses it", kind_name(item))
            }
            UseKind::TypeOf(item) => match target.kind {
                SymbolKind::Stock | SymbolKind::Param => {
                    format!("{} is cut from it", kind_name(item))
                }
                _ => format!("{} is one", kind_name(item)),
            },
            UseKind::Joined(joint) => format!("joint {} joins it", name(joint)),
            UseKind::Holds(hardware) => format!("{} is fixed to it", kind_name(hardware)),
            UseKind::Placed(item) => format!("{} is placed by it", name(item)),
            UseKind::Field(item, field) => format!("{} · {field}", kind_name(item)),
            UseKind::Checked(require) => format!("require {} checks it", name(require)),
            UseKind::Value(item) => format!("{} uses it", kind_name(item)),
            UseKind::Text(item) => format!("{} mentions it", kind_name(item)),
            UseKind::Other => "used".to_owned(),
        }
    }

    // ---- hover ----

    /// What to show for the word at a position.
    pub fn hover(&self, position: FilePosition) -> Option<Hover> {
        if let Some(reference) = self.reference_at(position) {
            let symbol = self.symbol(reference.symbol)?;
            let docs = symbol.docs.join(" ");
            let docs = docs.split_whitespace().collect::<Vec<_>>().join(" ");
            return Some(Hover {
                span: reference.span,
                kind: symbol.kind.word().to_owned(),
                signature: symbol.signature.clone(),
                docs: (!docs.is_empty()).then_some(docs),
                location: self.location(symbol.file, symbol.name_span),
                value: None,
            });
        }
        // A word that is no item: a built-in function or an enum word.
        let root = self.index.files.get(&position.file).map(FileData::root)?;
        let token = token_at(&root, position.offset)?;
        if !token.kind().is_word() {
            return None;
        }
        let text = token.text();
        let range = token.text_range();
        let span = Span::new(u32::from(range.start()), u32::from(range.end()));
        let called = token
            .parent()
            .filter(|p| p.kind() == SyntaxKind::NAME_REF)
            .and_then(|p| p.parent())
            .is_some_and(|p| p.kind() == SyntaxKind::CALL_EXPR);
        if called && let Some(signature) = words::builtin_signature(text) {
            return Some(Hover {
                span,
                kind: "built-in function".to_owned(),
                signature,
                docs: None,
                location: None,
                value: None,
            });
        }
        if let Some(set) = WordSet::ALL.iter().find(|set| set.contains(text)) {
            return Some(Hover {
                span,
                kind: "word".to_owned(),
                signature: text.to_owned(),
                docs: Some(format!("'{text}' is {}.", set.description())),
                location: None,
                value: None,
            });
        }
        None
    }

    /// [`Analysis::hover`] with the value the word's param, `let`, size or
    /// `between(...)` works out to, when `values` has it: hovering
    /// `cabinet_width` or the `fit` of `length: fit` shows `= 844mm`.
    pub fn hover_with_values(
        &self,
        position: FilePosition,
        values: &crate::values::WorkedValues,
    ) -> Option<Hover> {
        let hover = self.hover(position);
        let Some(note) = values.at(self, position) else {
            return hover;
        };
        let value = Some(format!("= {}", note.text));
        Some(match hover {
            Some(hover) => Hover { value, ..hover },
            None => Hover {
                span: note.span,
                kind: note.kind.word().to_owned(),
                signature: note.covers.split_whitespace().collect::<Vec<_>>().join(" "),
                docs: None,
                location: None,
                value,
            },
        })
    }

    /// `true` when the word at a position stands where an item's name
    /// goes (a bare name in an expression, a reference, a path) and means
    /// nothing: a typo, or a name not declared yet. Enum words, built-in
    /// functions and fixed call heads are words of the language, not
    /// unresolved names. Its token carries
    /// [`crate::TokenModifier::Unresolved`] in [`Analysis::tokens`].
    pub fn is_unresolved(&self, position: FilePosition) -> bool {
        if self.reference_at(position).is_some() {
            return false;
        }
        let Some(root) = self.index.files.get(&position.file).map(FileData::root) else {
            return false;
        };
        token_at(&root, position.offset).is_some_and(|token| crate::tokens::names_nothing(&token))
    }

    // ---- breadcrumbs ----

    /// Where a position is: the file, then each item, field or block
    /// around it, outermost first.
    pub fn breadcrumbs(&self, position: FilePosition) -> Vec<Crumb> {
        let mut crumbs = Vec::new();
        let Some(data) = self.index.files.get(&position.file) else {
            return crumbs;
        };
        let root = data.root();
        let document = Document::from_source_path(&data.path);
        crumbs.push(Crumb {
            label: document.display(&self.index.folder),
            span: span_of(&root),
        });
        let Some(token) = token_at(&root, position.offset) else {
            return crumbs;
        };
        let mut inner: Vec<Crumb> = Vec::new();
        for node in token.parent_ancestors() {
            if let Some(label) = crumb_label(&node) {
                inner.push(Crumb {
                    label,
                    span: span_of(&node),
                });
            }
        }
        inner.reverse();
        crumbs.extend(inner);
        crumbs
    }

    // ---- colours, lenses, diagnostics ----

    /// Every significant token of a file, coloured.
    pub fn tokens(&self, file: FileId) -> Vec<SemanticToken> {
        match self.index.files.get(&file) {
            Some(data) => data.tokens.get_or_init(|| self.index.tokens(file)).clone(),
            None => Vec::new(),
        }
    }

    /// The lenses of a file: one above each part, assembly, piece of
    /// hardware, component and kind, in order.
    pub fn lenses(&self, file: FileId) -> Vec<Lens> {
        let Some(data) = self.index.files.get(&file) else {
            return Vec::new();
        };
        let mut lenses: Vec<Lens> = data
            .items
            .symbols
            .iter()
            .filter(|s| {
                s.kind.is_shown_in_3d()
                    || matches!(s.kind, SymbolKind::Component | SymbolKind::Kind)
            })
            .filter_map(|s| {
                let uses = self
                    .index
                    .uses_of(s.id)
                    .filter(|r| r.kind != UseKind::Declared)
                    .count();
                Some(Lens {
                    symbol: s.id,
                    kind: s.kind,
                    name: s.name.clone(),
                    location: self.location(file, s.name_span)?,
                    uses,
                })
            })
            .collect();
        lenses.sort_by_key(|l| l.location.span.start());
        lenses
    }

    /// The diagnostics found while loading (syntax, imports, module
    /// headers) in a file: the fast tier, available on every change. The
    /// checker's slower diagnostics come with a check.
    pub fn diagnostics(&self, file: FileId) -> impl Iterator<Item = &Diagnostic> {
        self.index.diagnostics.get(&file).into_iter().flatten()
    }

    /// Every loaded file of the project's own folders (not the standard
    /// library), sorted by path.
    pub fn project_files(&self) -> Vec<(FileId, Document)> {
        let mut files: Vec<(FileId, Document)> = self
            .index
            .modules
            .values()
            .filter(|m| !m.standard)
            .flat_map(|m| m.files.iter().copied())
            .filter_map(|file| Some((file, self.document(file)?)))
            .collect();
        files.sort_by(|a, b| a.1.cmp(&b.1));
        files
    }

    // ---- search ----

    /// "Go to file or name": files whose name contains `query`, then
    /// declarations whose name does (prefix matches first, the project's
    /// own before the standard library's), at most `limit`.
    pub fn search(&self, query: &str, limit: usize) -> Vec<Found> {
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            return Vec::new();
        }
        let mut found: Vec<(u8, Found)> = Vec::new();
        for (file, document) in self.project_files() {
            let name = document.display(&self.index.folder);
            if name.to_lowercase().contains(&query)
                && let Some(location) = self.location(file, Span::new(0, 0))
            {
                let rank = if document.file_name().to_lowercase().starts_with(&query) {
                    0
                } else {
                    1
                };
                found.push((
                    rank,
                    Found {
                        label: name,
                        detail: "file".to_owned(),
                        location,
                    },
                ));
            }
        }
        for symbol in self.index.symbols() {
            if symbol.kind == SymbolKind::Label || symbol.kind == SymbolKind::Module {
                continue;
            }
            let lower = symbol.name.to_lowercase();
            if !lower.contains(&query) {
                continue;
            }
            let standard = self
                .index
                .modules
                .get(&symbol.module)
                .is_some_and(|m| m.standard);
            if standard && symbol.private {
                continue;
            }
            let Some(location) = self.location(symbol.file, symbol.name_span) else {
                continue;
            };
            let rank = match (lower.starts_with(&query), standard) {
                (true, false) => 2,
                (false, false) => 3,
                (true, true) => 4,
                (false, true) => 5,
            };
            let label = match symbol.container.and_then(|c| self.symbol(c)) {
                Some(container) => format!("{}.{}", container.name, symbol.name),
                None => symbol.name.clone(),
            };
            found.push((
                rank,
                Found {
                    label,
                    detail: format!(
                        "{} · {}",
                        symbol.kind.word(),
                        location.document.display(&self.index.folder)
                    ),
                    location,
                },
            ));
        }
        found.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.label.cmp(&b.1.label)));
        found.into_iter().take(limit).map(|(_, f)| f).collect()
    }
}

/// The token at an offset, preferring a word when the offset sits between
/// two tokens.
fn token_at(root: &SyntaxNode, offset: u32) -> Option<stave_lang::SyntaxToken> {
    let length = u32::from(root.text_range().end());
    if offset > length {
        return None;
    }
    let found = root.token_at_offset(rowan::TextSize::from(offset));
    let (left, right) = match found {
        rowan::TokenAtOffset::None => return None,
        rowan::TokenAtOffset::Single(token) => return Some(token),
        rowan::TokenAtOffset::Between(left, right) => (left, right),
    };
    if right.kind().is_word() || !left.kind().is_word() {
        Some(right)
    } else {
        Some(left)
    }
}

/// The breadcrumb for a node, when it is a step worth showing.
#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "SyntaxKind is a flat set of about 140 token and node tags: this tests the few it needs and treats the rest alike"
)]
fn crumb_label(node: &SyntaxNode) -> Option<String> {
    let kind = node.kind();
    if let Some(symbol_kind) = SymbolKind::of_node(kind) {
        let (name, _) = declared_name(node)?;
        return Some(format!("{} {name}", symbol_kind.word()));
    }
    match kind {
        SyntaxKind::PROJECT | SyntaxKind::LIBRARY => {
            let title = node
                .children()
                .find(|c| c.kind() == SyntaxKind::STRING)
                .map(|s| s.to_string())
                .unwrap_or_default();
            let word = if kind == SyntaxKind::PROJECT {
                "project"
            } else {
                "library"
            };
            Some(format!("{word} {title}").trim().to_owned())
        }
        SyntaxKind::FIELD => node
            .children()
            .find(|c| c.kind() == SyntaxKind::NAME)
            .and_then(|n| word_of(&n))
            .map(|(name, _)| name),
        SyntaxKind::PLACE => Some("place".to_owned()),
        SyntaxKind::ROLES => Some("roles".to_owned()),
        SyntaxKind::CUT => Some("cut".to_owned()),
        SyntaxKind::IMPORT => Some("import".to_owned()),
        SyntaxKind::POSE => node
            .children()
            .find(|c| c.kind() == SyntaxKind::PATH)
            .map(|path| format!("pose {path}")),
        _ => None,
    }
}
