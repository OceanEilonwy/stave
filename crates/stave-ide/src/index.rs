//! The index: every declaration of every loaded file as a [`Symbol`], the
//! scopes they live in, and every name written in the source resolved to
//! the symbol it means, as a [`Reference`].
//!
//! The index works on the lossless syntax tree, so it sees items the
//! parser recovered from as well as correct ones: a half-typed line while
//! editing loses only what it breaks. Names resolve by the grammar's
//! namespaces ("Names"): a bare name looks in the enclosing `component` or
//! `kind` body, then the module (every file of the folder), then the
//! modules this file imports (by their import name), then the prelude. A
//! member after `.` looks in what is on its left: a module's public items,
//! or the component an assembly is an instance of. A word in `[ ]` looks
//! among the array's labels; a named argument of a kind call among the
//! kind's params and roles; a field of an assembly among its component's
//! params. Enum words, built-in functions and the fixed words of
//! placement terms and cuts are never items.
//!
//! The reading is syntactic: where only the types tell (an enum word that
//! is also an item's name), the checker's diagnostics have the last word.
//!
//! The index is made of per-file and per-module parts so that
//! [`crate::AnalysisHost`] can keep them from one change to the next:
//!
//! - [`FileItems`], a file's declarations: a function of the file's tree
//!   alone. A [`SymbolId`] is the file and the declaration's place in it,
//!   so the ids of one file's symbols do not move when another file
//!   changes.
//! - [`ModuleNames`], the names a module declares at its top level: a
//!   function of its files' items.
//! - [`FileResolution`], a file's references, with every name lookup the
//!   resolving made and what it found ([`Read`], [`Seen`]). The
//!   resolution of an unchanged file is kept when every lookup it made
//!   still finds the same thing, so a change re-resolves only the files
//!   that named what changed.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use stave_lang::ast::Span;
use stave_lang::{Diagnostic, FileId, GreenNode, SourceMap, SyntaxKind, SyntaxNode};
use stave_modules::ModuleId;

use crate::tokens::SemanticToken;

/// Identifies a [`Symbol`] in an [`crate::Analysis`]: the file it is
/// declared in and its place among that file's declarations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SymbolId {
    file: FileId,
    local: u32,
}

impl SymbolId {
    /// The `local` of a module's own symbol, which sits on its `library`
    /// or `project` item.
    const MODULE: u32 = u32::MAX;

    /// The symbol of the module whose header is in `file`.
    fn module_in(file: FileId) -> SymbolId {
        SymbolId {
            file,
            local: SymbolId::MODULE,
        }
    }
}

/// What a symbol declares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SymbolKind {
    /// A module: a folder, used as `casework.FramePanelDoor`.
    Module,
    /// `component`
    Component,
    /// `kind`
    Kind,
    /// `recipe`
    Recipe,
    /// `stock`
    Stock,
    /// `profile`
    Profile,
    /// `scene`
    Scene,
    /// `param`, at the top level, in a component or in a kind.
    Param,
    /// `let`
    Let,
    /// `region`
    Region,
    /// `part`
    Part,
    /// `joint`
    Joint,
    /// `hardware`
    Hardware,
    /// `assembly`
    Assembly,
    /// `require`
    Require,
    /// A role of a kind: `piece: Part`.
    Role,
    /// A part a kind makes: `makes part pin : ...`.
    MadePart,
    /// A named piece of a hardware kind: `member cup`.
    Member,
    /// A label of an array: `bottom` in `rail[bottom, top]`.
    Label,
}

impl SymbolKind {
    /// The word for it, as written in the language where it has a keyword:
    /// `part`, `assembly`, `module`, `role`, `label`.
    pub fn word(self) -> &'static str {
        match self {
            SymbolKind::Module => "module",
            SymbolKind::Component => "component",
            SymbolKind::Kind => "kind",
            SymbolKind::Recipe => "recipe",
            SymbolKind::Stock => "stock",
            SymbolKind::Profile => "profile",
            SymbolKind::Scene => "scene",
            SymbolKind::Param => "param",
            SymbolKind::Let => "let",
            SymbolKind::Region => "region",
            SymbolKind::Part => "part",
            SymbolKind::Joint => "joint",
            SymbolKind::Hardware => "hardware",
            SymbolKind::Assembly => "assembly",
            SymbolKind::Require => "require",
            SymbolKind::Role => "role",
            SymbolKind::MadePart => "made part",
            SymbolKind::Member => "member",
            SymbolKind::Label => "label",
        }
    }

    /// `true` for what the 3D view draws and can select: parts,
    /// assemblies and hardware.
    pub fn is_shown_in_3d(self) -> bool {
        match self {
            SymbolKind::Part | SymbolKind::Assembly | SymbolKind::Hardware => true,
            SymbolKind::Module
            | SymbolKind::Component
            | SymbolKind::Kind
            | SymbolKind::Recipe
            | SymbolKind::Stock
            | SymbolKind::Profile
            | SymbolKind::Scene
            | SymbolKind::Param
            | SymbolKind::Let
            | SymbolKind::Region
            | SymbolKind::Joint
            | SymbolKind::Require
            | SymbolKind::Role
            | SymbolKind::MadePart
            | SymbolKind::Member
            | SymbolKind::Label => false,
        }
    }

    /// The kind an item or entry node declares.
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "SyntaxKind is a flat set of about 140 token and node tags: this tests the few it needs and treats the rest alike"
    )]
    pub(crate) fn of_node(kind: SyntaxKind) -> Option<SymbolKind> {
        Some(match kind {
            SyntaxKind::COMPONENT => SymbolKind::Component,
            SyntaxKind::KIND => SymbolKind::Kind,
            SyntaxKind::RECIPE => SymbolKind::Recipe,
            SyntaxKind::STOCK => SymbolKind::Stock,
            SyntaxKind::PROFILE => SymbolKind::Profile,
            SyntaxKind::SCENE => SymbolKind::Scene,
            SyntaxKind::PARAM => SymbolKind::Param,
            SyntaxKind::LET => SymbolKind::Let,
            SyntaxKind::REGION => SymbolKind::Region,
            SyntaxKind::PART => SymbolKind::Part,
            SyntaxKind::JOINT => SymbolKind::Joint,
            SyntaxKind::HARDWARE => SymbolKind::Hardware,
            SyntaxKind::ASSEMBLY => SymbolKind::Assembly,
            SyntaxKind::REQUIRE => SymbolKind::Require,
            SyntaxKind::ROLE => SymbolKind::Role,
            SyntaxKind::MAKES => SymbolKind::MadePart,
            SyntaxKind::MEMBER => SymbolKind::Member,
            _ => return None,
        })
    }
}

/// One declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    /// Its id.
    pub id: SymbolId,
    /// What it declares.
    pub kind: SymbolKind,
    /// Its name as written.
    pub name: String,
    /// The file it is written in.
    pub file: FileId,
    /// Where its name is written (for a module: its `library` or
    /// `project` item's title).
    pub name_span: Span,
    /// The whole declaration, doc comments included.
    pub span: Span,
    /// The component or kind it is declared in, or the item whose label
    /// it is.
    pub container: Option<SymbolId>,
    /// The module it is declared in; for a module, the module itself.
    pub module: ModuleId,
    /// Its doc comment, one entry per `///` line, without the slashes.
    pub docs: Vec<String>,
    /// `private`.
    pub private: bool,
    /// Its declaration on one line, without its body:
    /// `assembly door_l : casework.FramePanelDoor`.
    pub signature: String,
}

/// What a use of a name does there, for the list of uses.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum UseKind {
    /// The declaration itself.
    Declared,
    /// `import "./casework"` or its `as` name.
    Imported,
    /// `door_r = mirror(door_l, ...)`: the item doing the mirroring.
    Mirrored(SymbolId),
    /// `pose door_l = open` in a scene.
    Posed(SymbolId),
    /// A field of a scene: `show: drawer[top]`.
    SceneField(SymbolId, String),
    /// The stock, component or kind an item is made from.
    TypeOf(SymbolId),
    /// One of a joint's parts.
    Joined(SymbolId),
    /// What a piece of hardware is fixed to.
    Holds(SymbolId),
    /// In a `place { }` block of an item.
    Placed(SymbolId),
    /// In a field of an item: the item and the field.
    Field(SymbolId, String),
    /// In a `require`.
    Checked(SymbolId),
    /// In the value of a param, let or region, or elsewhere in an item.
    Value(SymbolId),
    /// Inside the text of a string: `"{piece}"`.
    Text(SymbolId),
    /// Somewhere outside any named item.
    Other,
}

/// One name written in the source and what it means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    /// The file it is written in.
    pub file: FileId,
    /// The name's own span.
    pub span: Span,
    /// The symbol it means.
    pub symbol: SymbolId,
    /// What it does there.
    pub kind: UseKind,
}

/// Where a [`Reference`] is kept: its file and its place among that
/// file's references.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ReferenceId {
    file: FileId,
    position: usize,
}

/// A loaded file and what is worked out from it.
#[derive(Debug, Clone)]
pub(crate) struct FileData {
    pub(crate) path: String,
    pub(crate) module: Option<ModuleId>,
    pub(crate) green: GreenNode,
    /// Its declarations.
    pub(crate) items: Arc<FileItems>,
    /// Its import names, in source order: the name, where it is written
    /// and the module's symbol.
    pub(crate) imports: Vec<(String, Span, SymbolId)>,
    /// Its references and the lookups they took.
    pub(crate) resolution: Arc<FileResolution>,
    /// Its coloured tokens, worked out the first time they are asked for
    /// and kept while its resolution is.
    pub(crate) tokens: TokensMemo,
}

/// A file's coloured tokens, once worked out; shared by the analyses
/// that keep its resolution.
pub(crate) type TokensMemo = Arc<OnceLock<Vec<SemanticToken>>>;

impl FileData {
    pub(crate) fn root(&self) -> SyntaxNode {
        SyntaxNode::new_root(self.green.clone())
    }
}

/// A loaded module.
#[derive(Debug, Clone)]
pub(crate) struct ModuleData {
    /// Its own symbol, on its `library` or `project` item.
    pub(crate) symbol: Option<Symbol>,
    pub(crate) files: Vec<FileId>,
    pub(crate) standard: bool,
    /// The names it declares at its top level.
    pub(crate) names: Arc<ModuleNames>,
}

/// Names declared in each scope, keyed first by the scope's id so a
/// lookup by `&str` needs no allocation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Scopes<K: Ord>(BTreeMap<K, BTreeMap<String, SymbolId>>);

impl<K: Ord + Copy> Scopes<K> {
    /// The symbol `name` means in `scope`, if it declares it.
    pub(crate) fn get(&self, scope: K, name: &str) -> Option<SymbolId> {
        self.0
            .get(&scope)
            .and_then(|names| names.get(name))
            .copied()
    }

    /// Records `name` in `scope` unless it already means something there:
    /// the first declaration wins. The name is copied only when it is new.
    fn declare(&mut self, scope: K, name: &str, symbol: SymbolId) {
        let names = self.0.entry(scope).or_default();
        if !names.contains_key(name) {
            names.insert(name.to_owned(), symbol);
        }
    }
}

/// A module's `library` or `project` item, where its symbol sits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Header {
    name_span: Span,
    span: Span,
    docs: Vec<String>,
    signature: String,
}

/// The items query: one file's declarations, worked out from its tree
/// alone.
#[derive(Debug, Clone)]
pub(crate) struct FileItems {
    file: FileId,
    module: ModuleId,
    /// Its symbols in declaration order; a symbol's `local` is its place.
    pub(crate) symbols: Vec<Symbol>,
    /// The symbol an item node declares, by node start.
    item_at: BTreeMap<u32, SymbolId>,
    /// What each component and kind declares inside, and each item's
    /// labels.
    inner: Scopes<SymbolId>,
    /// The params written `solved param`.
    solved: BTreeSet<SymbolId>,
    /// Its `library` or `project` item.
    header: Option<Header>,
    /// Its assemblies, with the span of each node, to link to their
    /// components.
    assemblies: Vec<(SymbolId, Span)>,
}

/// The module names query: the names a module declares at its top level,
/// the first declaration of each name winning.
pub(crate) type ModuleNames = BTreeMap<String, SymbolId>;

/// A lookup resolving made, for checking later that it still finds the
/// same thing.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Read {
    /// A name at a module's top level.
    ModuleName(ModuleId, String),
    /// A name inside a component, a kind or an item's labels.
    InnerName(SymbolId, String),
    /// A file's import name.
    ImportName(FileId, String),
    /// What a symbol is.
    Symbol(SymbolId),
    /// The component an assembly is an instance of.
    Instance(SymbolId),
}

/// What a [`Read`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Seen {
    /// The symbol a name or an assembly's component is, or none.
    Symbol(Option<SymbolId>),
    /// What a symbol is, or that there is none.
    Shape(Option<SymbolShape>),
}

/// What resolving and colouring read of a symbol: everything but where it
/// is written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SymbolShape {
    kind: SymbolKind,
    name: String,
    module: ModuleId,
    container: Option<SymbolId>,
    private: bool,
    solved: bool,
    standard: bool,
}

/// Records the lookups of one resolution, or none.
#[derive(Debug, Default)]
pub(crate) struct Recorder(Option<RefCell<BTreeMap<Read, Seen>>>);

impl Recorder {
    /// A recorder that records nothing.
    pub(crate) fn off() -> Recorder {
        Recorder(None)
    }

    /// A recorder that records every lookup.
    fn on() -> Recorder {
        Recorder(Some(RefCell::new(BTreeMap::new())))
    }

    fn note(&self, read: impl FnOnce() -> Read, seen: Option<SymbolId>) {
        if let Some(reads) = &self.0 {
            reads.borrow_mut().insert(read(), Seen::Symbol(seen));
        }
    }

    fn note_symbol(&self, index: &Index, id: SymbolId) {
        if let Some(reads) = &self.0 {
            let read = Read::Symbol(id);
            if !reads.borrow().contains_key(&read) {
                reads
                    .borrow_mut()
                    .insert(read, Seen::Shape(index.shape(id)));
            }
        }
    }

    fn finish(self) -> BTreeMap<Read, Seen> {
        self.0.map(RefCell::into_inner).unwrap_or_default()
    }
}

/// The resolution query: one file's references, with the lookups that
/// found them.
#[derive(Debug, Clone, Default)]
pub(crate) struct FileResolution {
    /// Its references, sorted by position, declarations included.
    pub(crate) references: Vec<Reference>,
    /// Every lookup made and what it found.
    pub(crate) reads: BTreeMap<Read, Seen>,
    /// What the reads depend on, to tell quickly that none can have
    /// changed.
    pub(crate) depends: Depends,
}

/// What a resolution's reads depend on: the files whose declarations
/// they looked at, the modules whose names they looked up, whether they
/// asked which component an assembly is an instance of, and whether they
/// looked at a module's own symbol. A change that touches none of these
/// leaves every read as it was.
#[derive(Debug, Clone, Default)]
pub(crate) struct Depends {
    pub(crate) files: BTreeSet<FileId>,
    pub(crate) modules: BTreeSet<ModuleId>,
    pub(crate) instances: bool,
    pub(crate) module_symbols: bool,
}

impl Depends {
    fn of(reads: &BTreeMap<Read, Seen>) -> Depends {
        let mut depends = Depends::default();
        for read in reads.keys() {
            match read {
                Read::ModuleName(module, _) => {
                    depends.modules.insert(*module);
                }
                Read::InnerName(container, _) => {
                    depends.files.insert(container.file);
                }
                Read::ImportName(file, _) => {
                    depends.files.insert(*file);
                }
                Read::Symbol(id) if id.local == SymbolId::MODULE => {
                    depends.module_symbols = true;
                }
                Read::Symbol(id) => {
                    depends.files.insert(id.file);
                }
                Read::Instance(_) => depends.instances = true,
            }
        }
        depends
    }
}

/// Everything the queries read: put together from the per-file and
/// per-module parts on every change.
#[derive(Debug)]
pub(crate) struct Index {
    pub(crate) folder: PathBuf,
    pub(crate) sources: SourceMap,
    pub(crate) files: BTreeMap<FileId, FileData>,
    pub(crate) modules: BTreeMap<ModuleId, ModuleData>,
    pub(crate) root: ModuleId,
    pub(crate) prelude: Vec<ModuleId>,
    /// The component each assembly is an instance of.
    pub(crate) instance_of: BTreeMap<SymbolId, SymbolId>,
    /// The loader's diagnostics (syntax, imports and module headers), by
    /// the file of their primary label.
    pub(crate) diagnostics: BTreeMap<FileId, Vec<Diagnostic>>,
    /// Every use of every symbol, worked out the first time it is asked
    /// for.
    uses: OnceLock<BTreeMap<SymbolId, Vec<ReferenceId>>>,
}

/// The start of a node, as a byte offset.
pub(crate) fn start(node: &SyntaxNode) -> u32 {
    u32::from(node.text_range().start())
}

/// A node's span.
pub(crate) fn span_of(node: &SyntaxNode) -> Span {
    let range = node.text_range();
    Span::new(u32::from(range.start()), u32::from(range.end()))
}

/// The first word (identifier or keyword) token directly inside a node,
/// with its span.
pub(crate) fn word_of(node: &SyntaxNode) -> Option<(String, Span)> {
    node.children_with_tokens()
        .filter_map(|element| element.into_token())
        .find(|token| token.kind().is_word())
        .map(|token| {
            let range = token.text_range();
            (
                token.text().to_owned(),
                Span::new(u32::from(range.start()), u32::from(range.end())),
            )
        })
}

/// The name an item node declares: its first `NAME` child.
pub(crate) fn declared_name(node: &SyntaxNode) -> Option<(String, Span)> {
    node.children()
        .find(|child| child.kind() == SyntaxKind::NAME)
        .and_then(|name| word_of(&name))
}

/// The declaration on one line: the item's text from its keyword up to its
/// body, whitespace collapsed.
fn signature(node: &SyntaxNode) -> String {
    let mut out = String::new();
    for element in node.children_with_tokens() {
        let kind = element.kind();
        if kind == SyntaxKind::DOC || kind == SyntaxKind::BLOCK {
            if kind == SyntaxKind::BLOCK {
                break;
            }
            continue;
        }
        let text = element.to_string();
        out.push_str(&text);
    }
    let one_line = out
        .lines()
        .map(|line| line.split("//").next().unwrap_or(line))
        .collect::<Vec<_>>()
        .join(" ");
    one_line.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The `///` lines directly inside a node, without the slashes.
fn docs(node: &SyntaxNode) -> Vec<String> {
    node.children_with_tokens()
        .filter_map(|element| element.into_token())
        .filter(|token| token.kind() == SyntaxKind::DOC)
        .map(|token| {
            let text = token.text();
            let text = text.strip_prefix("///").unwrap_or(text);
            text.strip_prefix(' ').unwrap_or(text).trim_end().to_owned()
        })
        .collect()
}

/// The body block of an item node.
fn block_of(node: &SyntaxNode) -> Option<SyntaxNode> {
    node.children()
        .find(|child| child.kind() == SyntaxKind::BLOCK)
}

impl FileItems {
    /// Works out a file's declarations from its tree.
    pub(crate) fn new(file: FileId, module: ModuleId, root: &SyntaxNode) -> FileItems {
        let mut items = FileItems {
            file,
            module,
            symbols: Vec::new(),
            item_at: BTreeMap::new(),
            inner: Scopes(BTreeMap::new()),
            solved: BTreeSet::new(),
            header: None,
            assemblies: Vec::new(),
        };
        items.header = root
            .children()
            .find(|child| matches!(child.kind(), SyntaxKind::LIBRARY | SyntaxKind::PROJECT))
            .map(|item| Header {
                name_span: item
                    .children()
                    .find(|child| child.kind() == SyntaxKind::STRING)
                    .map(|string| span_of(&string))
                    .or_else(|| word_of(&item).map(|(_, span)| span))
                    .unwrap_or_else(|| span_of(&item)),
                span: span_of(&item),
                docs: docs(&item),
                signature: signature(&item),
            });
        for item in root.children() {
            items.declare_item(&item, None);
        }
        items
    }

    fn push_symbol(&mut self, mut symbol: Symbol) -> Option<SymbolId> {
        let local = u32::try_from(self.symbols.len())
            .ok()
            .filter(|local| *local != SymbolId::MODULE)?;
        let id = SymbolId {
            file: self.file,
            local,
        };
        symbol.id = id;
        self.symbols.push(symbol);
        Some(id)
    }

    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "SyntaxKind is a flat set of about 140 token and node tags: this tests the few it needs and treats the rest alike"
    )]
    fn declare_item(&mut self, node: &SyntaxNode, container: Option<SymbolId>) {
        let Some(kind) = SymbolKind::of_node(node.kind()) else {
            return;
        };
        let Some((name, name_span)) = declared_name(node) else {
            return;
        };
        let private = node
            .children_with_tokens()
            .any(|element| element.kind() == SyntaxKind::KW_PRIVATE);
        let Some(id) = self.push_symbol(Symbol {
            id: SymbolId::module_in(self.file),
            kind,
            name: name.clone(),
            file: self.file,
            name_span,
            span: span_of(node),
            container,
            module: self.module,
            docs: docs(node),
            private,
            signature: signature(node),
        }) else {
            return;
        };
        if kind == SymbolKind::Param
            && node
                .children_with_tokens()
                .any(|element| element.kind() == SyntaxKind::KW_SOLVED)
        {
            self.solved.insert(id);
        }
        if kind == SymbolKind::Assembly {
            self.assemblies.push((id, span_of(node)));
        }
        // The first declaration of a name wins; a second is the checker's
        // `duplicate-name`. Top-level names go to the module's names.
        if let Some(container) = container {
            self.inner.declare(container, &name, id);
        }
        self.item_at.insert(start(node), id);
        // An array's labels.
        if let Some(spec) = node
            .children()
            .find(|child| child.kind() == SyntaxKind::ARRAY_SPEC)
        {
            for label in spec.children().filter(|c| c.kind() == SyntaxKind::NAME) {
                let Some((text, span)) = word_of(&label) else {
                    continue;
                };
                let Some(label_id) = self.push_symbol(Symbol {
                    id: SymbolId::module_in(self.file),
                    kind: SymbolKind::Label,
                    name: text.clone(),
                    file: self.file,
                    name_span: span,
                    span,
                    container: Some(id),
                    module: self.module,
                    docs: Vec::new(),
                    private: false,
                    signature: format!("label {text} of {name}"),
                }) else {
                    continue;
                };
                self.inner.declare(id, &text, label_id);
            }
        }
        // What a component or a kind declares inside.
        match node.kind() {
            SyntaxKind::COMPONENT => {
                if let Some(block) = block_of(node) {
                    for child in block.children() {
                        self.declare_item(&child, Some(id));
                    }
                }
            }
            SyntaxKind::KIND => {
                if let Some(block) = block_of(node) {
                    for child in block.children() {
                        if child.kind() == SyntaxKind::ROLES {
                            if let Some(roles) = block_of(&child) {
                                for role in roles.children() {
                                    self.declare_item(&role, Some(id));
                                }
                            }
                        } else {
                            self.declare_item(&child, Some(id));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// The symbol of the module, when its header is in this file.
    pub(crate) fn module_symbol(&self, name: &str) -> Option<Symbol> {
        let header = self.header.as_ref()?;
        Some(Symbol {
            id: SymbolId::module_in(self.file),
            kind: SymbolKind::Module,
            name: name.to_owned(),
            file: self.file,
            name_span: header.name_span,
            span: header.span,
            container: None,
            module: self.module,
            docs: header.docs.clone(),
            private: false,
            signature: header.signature.clone(),
        })
    }

    /// `true` if the file has a `library` or `project` item.
    pub(crate) fn has_header(&self) -> bool {
        self.header.is_some()
    }
}

/// Works out the names a module declares at its top level from its files'
/// items, in file order.
pub(crate) fn module_names<'a>(files: impl IntoIterator<Item = &'a FileItems>) -> ModuleNames {
    let mut names = ModuleNames::new();
    for items in files {
        for symbol in items.symbols.iter().filter(|s| s.container.is_none()) {
            if !names.contains_key(&symbol.name) {
                names.insert(symbol.name.clone(), symbol.id);
            }
        }
    }
    names
}

impl Index {
    /// An index of these files and modules, with no assembly linked and
    /// no file resolved yet ([`Index::link_assemblies`],
    /// [`Index::resolve_file`]).
    pub(crate) fn new(
        folder: PathBuf,
        sources: SourceMap,
        files: BTreeMap<FileId, FileData>,
        modules: BTreeMap<ModuleId, ModuleData>,
        root: ModuleId,
        prelude: Vec<ModuleId>,
        diagnostics: Vec<Diagnostic>,
    ) -> Index {
        let mut by_file: BTreeMap<FileId, Vec<Diagnostic>> = BTreeMap::new();
        for diagnostic in diagnostics {
            if let Some(file) = diagnostic.primary().map(|label| label.file) {
                by_file.entry(file).or_default().push(diagnostic);
            }
        }
        Index {
            folder,
            sources,
            files,
            modules,
            root,
            prelude,
            instance_of: BTreeMap::new(),
            diagnostics: by_file,
            uses: OnceLock::new(),
        }
    }

    /// The symbol with this id.
    pub(crate) fn symbol(&self, id: SymbolId) -> Option<&Symbol> {
        let data = self.files.get(&id.file)?;
        if id.local == SymbolId::MODULE {
            return self
                .modules
                .get(&data.module?)?
                .symbol
                .as_ref()
                .filter(|symbol| symbol.id == id);
        }
        data.items.symbols.get(usize::try_from(id.local).ok()?)
    }

    /// Every symbol: module by module, each module's own symbol first and
    /// then its files' declarations in order.
    pub(crate) fn symbols(&self) -> impl Iterator<Item = &Symbol> {
        self.modules.values().flat_map(move |module| {
            module.symbol.iter().chain(
                module
                    .files
                    .iter()
                    .filter_map(|file| self.files.get(file))
                    .flat_map(|data| data.items.symbols.iter()),
            )
        })
    }

    /// What a symbol is, for checking a kept resolution.
    fn shape(&self, id: SymbolId) -> Option<SymbolShape> {
        let symbol = self.symbol(id)?;
        Some(SymbolShape {
            kind: symbol.kind,
            name: symbol.name.clone(),
            module: symbol.module,
            container: symbol.container,
            private: symbol.private,
            solved: self.is_solved(id),
            standard: self
                .modules
                .get(&symbol.module)
                .is_some_and(|module| module.standard),
        })
    }

    /// `true` for a param written `solved param`.
    pub(crate) fn is_solved(&self, id: SymbolId) -> bool {
        self.files
            .get(&id.file)
            .is_some_and(|data| data.items.solved.contains(&id))
    }

    /// The symbol `name` means at a module's top level.
    pub(crate) fn module_name(&self, module: ModuleId, name: &str) -> Option<SymbolId> {
        self.modules.get(&module)?.names.get(name).copied()
    }

    /// The symbol `name` means inside a component or kind, or as a label
    /// of an item.
    pub(crate) fn inner_name(&self, container: SymbolId, name: &str) -> Option<SymbolId> {
        self.files
            .get(&container.file)?
            .items
            .inner
            .get(container, name)
    }

    /// The module symbol a file imports under `name`; the last import of a
    /// name wins.
    pub(crate) fn import_name(&self, file: FileId, name: &str) -> Option<SymbolId> {
        self.files
            .get(&file)?
            .imports
            .iter()
            .rev()
            .find(|(import, _, _)| import == name)
            .map(|(_, _, symbol)| *symbol)
    }

    /// The symbol an item node of a file declares, by the node's start.
    fn item_at(&self, file: FileId, offset: u32) -> Option<SymbolId> {
        self.files.get(&file)?.items.item_at.get(&offset).copied()
    }

    /// A file's references, sorted by position.
    pub(crate) fn references(&self, file: FileId) -> &[Reference] {
        self.files
            .get(&file)
            .map_or(&[], |data| data.resolution.references.as_slice())
    }

    /// The reference kept at this place.
    pub(crate) fn reference(&self, id: ReferenceId) -> Option<&Reference> {
        self.references(id.file).get(id.position)
    }

    /// Every use of every symbol, as places of references in file and
    /// position order.
    pub(crate) fn uses(&self) -> &BTreeMap<SymbolId, Vec<ReferenceId>> {
        self.uses.get_or_init(|| {
            let mut uses: BTreeMap<SymbolId, Vec<ReferenceId>> = BTreeMap::new();
            for (file, data) in &self.files {
                for (position, reference) in data.resolution.references.iter().enumerate() {
                    uses.entry(reference.symbol).or_default().push(ReferenceId {
                        file: *file,
                        position,
                    });
                }
            }
            uses
        })
    }

    /// The references to a symbol.
    pub(crate) fn uses_of(&self, id: SymbolId) -> impl Iterator<Item = &Reference> {
        self.uses()
            .get(&id)
            .into_iter()
            .flatten()
            .filter_map(|place| self.reference(*place))
    }

    /// `true` if a kept read still finds what it found.
    pub(crate) fn still_holds(&self, read: &Read, seen: &Seen) -> bool {
        let found = match read {
            Read::ModuleName(module, name) => self.module_name(*module, name),
            Read::InnerName(container, name) => self.inner_name(*container, name),
            Read::ImportName(file, name) => self.import_name(*file, name),
            Read::Instance(assembly) => self.instance_of.get(assembly).copied(),
            Read::Symbol(id) => {
                return match seen {
                    Seen::Shape(shape) => self.has_shape(*id, shape.as_ref()),
                    Seen::Symbol(_) => false,
                };
            }
        };
        *seen == Seen::Symbol(found)
    }

    /// `true` if a symbol is what `shape` says, or is missing as it says.
    fn has_shape(&self, id: SymbolId, shape: Option<&SymbolShape>) -> bool {
        match (self.symbol(id), shape) {
            (None, None) => true,
            (Some(symbol), Some(shape)) => {
                symbol.kind == shape.kind
                    && symbol.name == shape.name
                    && symbol.module == shape.module
                    && symbol.container == shape.container
                    && symbol.private == shape.private
                    && self.is_solved(id) == shape.solved
                    && self
                        .modules
                        .get(&symbol.module)
                        .is_some_and(|module| module.standard)
                        == shape.standard
            }
            (None, Some(_)) | (Some(_), None) => false,
        }
    }

    // ---- scopes ----

    /// The kind of a symbol, recorded.
    fn kind_of(&self, recorder: &Recorder, id: SymbolId) -> Option<SymbolKind> {
        recorder.note_symbol(self, id);
        self.symbol(id).map(|s| s.kind)
    }

    /// The component or kind body a node is inside.
    fn container_at(&self, file: FileId, node: &SyntaxNode) -> Option<SymbolId> {
        node.ancestors()
            .find(|a| matches!(a.kind(), SyntaxKind::COMPONENT | SyntaxKind::KIND))
            .and_then(|a| self.item_at(file, start(&a)))
    }

    /// The innermost named item a node is inside.
    pub(crate) fn item_around(&self, file: FileId, node: &SyntaxNode) -> Option<SymbolId> {
        node.ancestors()
            .filter(|a| SymbolKind::of_node(a.kind()).is_some())
            .find_map(|a| self.item_at(file, start(&a)))
    }

    fn visible(&self, recorder: &Recorder, symbol: SymbolId, from: Option<ModuleId>) -> bool {
        recorder.note_symbol(self, symbol);
        self.symbol(symbol)
            .is_some_and(|s| !s.private || Some(s.module) == from)
    }

    fn module_name_read(
        &self,
        recorder: &Recorder,
        module: ModuleId,
        name: &str,
    ) -> Option<SymbolId> {
        let found = self.module_name(module, name);
        recorder.note(|| Read::ModuleName(module, name.to_owned()), found);
        found
    }

    fn inner_name_read(
        &self,
        recorder: &Recorder,
        container: SymbolId,
        name: &str,
    ) -> Option<SymbolId> {
        let found = self.inner_name(container, name);
        recorder.note(|| Read::InnerName(container, name.to_owned()), found);
        found
    }

    fn instance_read(&self, recorder: &Recorder, assembly: SymbolId) -> Option<SymbolId> {
        let found = self.instance_of.get(&assembly).copied();
        recorder.note(|| Read::Instance(assembly), found);
        found
    }

    /// What a bare name means at a node.
    pub(crate) fn lookup(&self, file: FileId, node: &SyntaxNode, name: &str) -> Option<SymbolId> {
        self.lookup_read(&Recorder::off(), file, node, name)
    }

    fn lookup_read(
        &self,
        recorder: &Recorder,
        file: FileId,
        node: &SyntaxNode,
        name: &str,
    ) -> Option<SymbolId> {
        let module = self.files.get(&file).and_then(|f| f.module);
        // The enclosing body first, then further out.
        if let Some(container) = self.container_at(file, node)
            && let Some(found) = self.inner_name_read(recorder, container, name)
        {
            return Some(found);
        }
        if let Some(module) = module
            && let Some(found) = self.module_name_read(recorder, module, name)
        {
            return Some(found);
        }
        let imported = self.import_name(file, name);
        recorder.note(|| Read::ImportName(file, name.to_owned()), imported);
        if imported.is_some() {
            return imported;
        }
        self.prelude.iter().find_map(|prelude| {
            self.module_name_read(recorder, *prelude, name)
                .filter(|found| self.visible(recorder, *found, module))
        })
    }

    /// What `name` after a `.` means when the left side is `base`.
    pub(crate) fn member(&self, file: FileId, base: SymbolId, name: &str) -> Option<SymbolId> {
        self.member_read(&Recorder::off(), file, base, name)
    }

    fn member_read(
        &self,
        recorder: &Recorder,
        file: FileId,
        base: SymbolId,
        name: &str,
    ) -> Option<SymbolId> {
        recorder.note_symbol(self, base);
        let symbol = self.symbol(base)?;
        match symbol.kind {
            SymbolKind::Module => {
                let from = self.files.get(&file).and_then(|f| f.module);
                self.module_name_read(recorder, symbol.module, name)
                    .filter(|found| self.visible(recorder, *found, from))
            }
            SymbolKind::Assembly => {
                let component = self.instance_read(recorder, base)?;
                self.inner_name_read(recorder, component, name)
            }
            SymbolKind::Component
            | SymbolKind::Kind
            | SymbolKind::Recipe
            | SymbolKind::Stock
            | SymbolKind::Profile
            | SymbolKind::Scene
            | SymbolKind::Param
            | SymbolKind::Let
            | SymbolKind::Region
            | SymbolKind::Part
            | SymbolKind::Joint
            | SymbolKind::Hardware
            | SymbolKind::Require
            | SymbolKind::Role
            | SymbolKind::MadePart
            | SymbolKind::Member
            | SymbolKind::Label => None,
        }
    }

    /// A label of an array.
    fn label(&self, recorder: &Recorder, base: SymbolId, name: &str) -> Option<SymbolId> {
        let found = self.inner_name_read(recorder, base, name)?;
        (self.kind_of(recorder, found) == Some(SymbolKind::Label)).then_some(found)
    }

    /// What an expression names, when it names an item.
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "SyntaxKind is a flat set of about 140 token and node tags: this tests the few it needs and treats the rest alike"
    )]
    fn resolve_expr(
        &self,
        recorder: &Recorder,
        file: FileId,
        node: &SyntaxNode,
    ) -> Option<SymbolId> {
        match node.kind() {
            SyntaxKind::NAME_REF => {
                let (name, _) = word_of(node)?;
                self.lookup_read(recorder, file, node, &name)
            }
            SyntaxKind::MEMBER_EXPR => {
                let base = node.children().next()?;
                let base = self.resolve_expr(recorder, file, &base)?;
                let member = node
                    .children()
                    .filter(|c| c.kind() == SyntaxKind::NAME)
                    .last()?;
                let (name, _) = word_of(&member)?;
                self.member_read(recorder, file, base, &name)
            }
            SyntaxKind::INDEX_EXPR | SyntaxKind::PAREN_EXPR => {
                let inner = node.children().next()?;
                self.resolve_expr(recorder, file, &inner)
            }
            _ => None,
        }
    }

    /// The symbols a `ref` or `path` names, segment by segment, up to (not
    /// including) `until`.
    fn resolve_segments(
        &self,
        recorder: &Recorder,
        file: FileId,
        node: &SyntaxNode,
        until: Option<&SyntaxNode>,
    ) -> Vec<(Span, Option<SymbolId>)> {
        let mut out = Vec::new();
        let mut current: Option<SymbolId> = None;
        let mut first = true;
        #[expect(
            clippy::wildcard_enum_match_arm,
            reason = "SyntaxKind is a flat set of about 140 token and node tags: this tests the few it needs and treats the rest alike"
        )]
        for child in node.children() {
            if until.is_some_and(|until| *until == child) {
                break;
            }
            match child.kind() {
                SyntaxKind::NAME => {
                    let Some((name, span)) = word_of(&child) else {
                        continue;
                    };
                    current = if first {
                        self.lookup_read(recorder, file, &child, &name)
                    } else {
                        current.and_then(|base| self.member_read(recorder, file, base, &name))
                    };
                    first = false;
                    out.push((span, current));
                }
                SyntaxKind::INDEX => {
                    // An element of the same array: members still apply.
                }
                _ => {}
            }
        }
        out
    }

    /// What an index's base is: the array being indexed.
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "SyntaxKind is a flat set of about 140 token and node tags: this tests the few it needs and treats the rest alike"
    )]
    fn index_base(
        &self,
        recorder: &Recorder,
        file: FileId,
        index: &SyntaxNode,
    ) -> Option<SymbolId> {
        let parent = index.parent()?;
        match parent.kind() {
            SyntaxKind::INDEX_EXPR => {
                let base = parent.children().next()?;
                self.resolve_expr(recorder, file, &base)
            }
            SyntaxKind::PATH => self
                .resolve_segments(recorder, file, &parent, Some(index))
                .last()
                .and_then(|(_, symbol)| *symbol),
            _ => None,
        }
    }

    /// Works out which component each assembly is an instance of:
    /// instances of a component first, then mirrors (which take their
    /// source's component), a few rounds for mirrors of mirrors.
    pub(crate) fn link_assemblies(&mut self) {
        let recorder = Recorder::off();
        let mut assemblies: Vec<(FileId, SymbolId, SyntaxNode)> = Vec::new();
        for (file, data) in &self.files {
            if data.items.assemblies.is_empty() {
                continue;
            }
            let root = data.root();
            for (id, span) in &data.items.assemblies {
                if let Some(node) = node_with_span(&root, SyntaxKind::ASSEMBLY, *span) {
                    assemblies.push((*file, *id, node));
                }
            }
        }
        for _ in 0..4 {
            let mut found = Vec::new();
            for (file, id, node) in &assemblies {
                if self.instance_of.contains_key(id) {
                    continue;
                }
                let component = if let Some(reference) =
                    node.children().find(|c| c.kind() == SyntaxKind::REF)
                {
                    self.resolve_segments(&recorder, *file, &reference, None)
                        .last()
                        .and_then(|(_, symbol)| *symbol)
                        .filter(|s| {
                            self.symbol(*s)
                                .is_some_and(|s| s.kind == SymbolKind::Component)
                        })
                } else {
                    node.children()
                        .find(|c| c.kind() == SyntaxKind::MIRROR)
                        .and_then(|mirror| mirror.children().find(|c| c.kind().is_expr()))
                        .and_then(|source| self.resolve_expr(&recorder, *file, &source))
                        .and_then(|source| self.instance_of.get(&source).copied())
                };
                if let Some(component) = component {
                    found.push((*id, component));
                }
            }
            if found.is_empty() {
                break;
            }
            self.instance_of.extend(found);
        }
    }

    // ---- references ----

    /// The resolution query: resolves every name of a file, recording
    /// each lookup.
    pub(crate) fn resolve_file(&self, file: FileId) -> FileResolution {
        let Some(data) = self.files.get(&file) else {
            return FileResolution::default();
        };
        let recorder = Recorder::on();
        let mut references: Vec<Reference> = data
            .items
            .symbols
            .iter()
            .map(|symbol| Reference {
                file,
                span: symbol.name_span,
                symbol: symbol.id,
                kind: UseKind::Declared,
            })
            .collect();
        for (_, span, symbol) in &data.imports {
            references.push(Reference {
                file,
                span: *span,
                symbol: *symbol,
                kind: UseKind::Imported,
            });
        }
        let root = data.root();
        let mut found: Vec<(Span, SymbolId, SyntaxNode)> = Vec::new();
        #[expect(
            clippy::wildcard_enum_match_arm,
            reason = "SyntaxKind is a flat set of about 140 token and node tags: this tests the few it needs and treats the rest alike"
        )]
        for node in root.descendants() {
            match node.kind() {
                SyntaxKind::NAME_REF => {
                    let Some((name, span)) = word_of(&node) else {
                        continue;
                    };
                    let symbol = match node.parent() {
                        Some(parent) if parent.kind() == SyntaxKind::INDEX => self
                            .index_base(&recorder, file, &parent)
                            .and_then(|base| self.label(&recorder, base, &name))
                            .or_else(|| {
                                (name != "same")
                                    .then(|| self.lookup_read(&recorder, file, &node, &name))
                                    .flatten()
                            }),
                        _ if is_enum_word_position(&node, &name) => None,
                        _ => self.lookup_read(&recorder, file, &node, &name),
                    };
                    if let Some(symbol) = symbol {
                        found.push((span, symbol, node.clone()));
                    }
                }
                SyntaxKind::MEMBER_EXPR => {
                    let Some(base) = node.children().next() else {
                        continue;
                    };
                    let Some(member) = node
                        .children()
                        .filter(|c| c.kind() == SyntaxKind::NAME)
                        .last()
                    else {
                        continue;
                    };
                    let Some((name, span)) = word_of(&member) else {
                        continue;
                    };
                    if let Some(symbol) = self
                        .resolve_expr(&recorder, file, &base)
                        .and_then(|base| self.member_read(&recorder, file, base, &name))
                    {
                        found.push((span, symbol, node.clone()));
                    }
                }
                SyntaxKind::REF | SyntaxKind::PATH => {
                    for (span, symbol) in self.resolve_segments(&recorder, file, &node, None) {
                        if let Some(symbol) = symbol {
                            found.push((span, symbol, node.clone()));
                        }
                    }
                }
                SyntaxKind::INTERP => {
                    let Some(first) = node.children().find(|c| c.kind() == SyntaxKind::NAME) else {
                        continue;
                    };
                    let Some((name, span)) = word_of(&first) else {
                        continue;
                    };
                    if let Some(symbol) = self.lookup_read(&recorder, file, &node, &name) {
                        found.push((span, symbol, node.clone()));
                    }
                }
                SyntaxKind::FIELD => {
                    // A field of an assembly sets its component's param.
                    let Some(assembly) = node
                        .parent()
                        .filter(|p| p.kind() == SyntaxKind::BLOCK)
                        .and_then(|block| block.parent())
                        .filter(|p| p.kind() == SyntaxKind::ASSEMBLY)
                    else {
                        continue;
                    };
                    let Some(name_node) = node.children().find(|c| c.kind() == SyntaxKind::NAME)
                    else {
                        continue;
                    };
                    let Some((name, span)) = word_of(&name_node) else {
                        continue;
                    };
                    if let Some(symbol) = self
                        .item_at(file, start(&assembly))
                        .and_then(|a| self.instance_read(&recorder, a))
                        .and_then(|c| self.inner_name_read(&recorder, c, &name))
                    {
                        found.push((span, symbol, node.clone()));
                    }
                }
                SyntaxKind::ARG => {
                    // A named argument of a kind's call: its param or role.
                    let Some(name_node) = node
                        .children()
                        .next()
                        .filter(|c| c.kind() == SyntaxKind::NAME)
                    else {
                        continue;
                    };
                    let Some((name, span)) = word_of(&name_node) else {
                        continue;
                    };
                    let Some(list) = node.parent().filter(|p| p.kind() == SyntaxKind::ARG_LIST)
                    else {
                        continue;
                    };
                    #[expect(
                        clippy::wildcard_enum_match_arm,
                        reason = "SyntaxKind is a flat set of about 140 token and node tags: this tests the few it needs and treats the rest alike"
                    )]
                    let callee = list.parent().and_then(|call| match call.kind() {
                        SyntaxKind::KIND_CALL => call
                            .children()
                            .find(|c| c.kind() == SyntaxKind::REF)
                            .and_then(|r| {
                                self.resolve_segments(&recorder, file, &r, None)
                                    .last()
                                    .and_then(|(_, s)| *s)
                            }),
                        SyntaxKind::CALL_EXPR => call
                            .children()
                            .next()
                            .and_then(|head| self.resolve_expr(&recorder, file, &head)),
                        _ => None,
                    });
                    if let Some(symbol) = callee
                        .filter(|c| self.kind_of(&recorder, *c) == Some(SymbolKind::Kind))
                        .and_then(|kind| self.inner_name_read(&recorder, kind, &name))
                    {
                        found.push((span, symbol, node.clone()));
                    }
                }
                _ => {}
            }
        }
        for (span, symbol, node) in found {
            let kind = self.use_kind(file, &node);
            references.push(Reference {
                file,
                span,
                symbol,
                kind,
            });
        }
        // What every reference means is part of what colouring reads.
        for reference in &references {
            recorder.note_symbol(self, reference.symbol);
        }
        references.sort_by_key(|r| (r.span.start(), r.span.end()));
        references.dedup();
        let reads = recorder.finish();
        FileResolution {
            references,
            depends: Depends::of(&reads),
            reads,
        }
    }

    /// What a use at `node` does there.
    fn use_kind(&self, file: FileId, node: &SyntaxNode) -> UseKind {
        let item = self.item_around(file, node);
        let Some(item) = item else {
            return UseKind::Other;
        };
        let item_kind = self.symbol(item).map(|s| s.kind);
        #[expect(
            clippy::wildcard_enum_match_arm,
            reason = "SyntaxKind is a flat set of about 140 token and node tags: this tests the few it needs and treats the rest alike"
        )]
        for ancestor in node.ancestors() {
            match ancestor.kind() {
                SyntaxKind::MIRROR => return UseKind::Mirrored(item),
                SyntaxKind::POSE => return UseKind::Posed(item),
                SyntaxKind::PLACE => return UseKind::Placed(item),
                SyntaxKind::STRING => return UseKind::Text(item),
                SyntaxKind::ARG_LIST
                    if ancestor
                        .parent()
                        .is_some_and(|p| p.kind() == SyntaxKind::KIND_CALL) =>
                {
                    return match item_kind {
                        Some(SymbolKind::Joint) => UseKind::Joined(item),
                        Some(SymbolKind::Hardware) => UseKind::Holds(item),
                        _ => UseKind::Value(item),
                    };
                }
                SyntaxKind::KIND_CALL => return UseKind::TypeOf(item),
                SyntaxKind::REF
                    if ancestor.parent().is_some_and(|p| {
                        matches!(
                            p.kind(),
                            SyntaxKind::PART | SyntaxKind::ASSEMBLY | SyntaxKind::MAKES
                        )
                    }) =>
                {
                    return UseKind::TypeOf(item);
                }
                SyntaxKind::FIELD => {
                    let field = ancestor
                        .children()
                        .find(|c| c.kind() == SyntaxKind::NAME)
                        .and_then(|n| word_of(&n))
                        .map(|(name, _)| name)
                        .unwrap_or_default();
                    return if item_kind == Some(SymbolKind::Scene) {
                        UseKind::SceneField(item, field)
                    } else {
                        UseKind::Field(item, field)
                    };
                }
                SyntaxKind::REQUIRE => return UseKind::Checked(item),
                kind if kind.is_item() || kind == SyntaxKind::ROLE || kind == SyntaxKind::MAKES => {
                    return UseKind::Value(item);
                }
                _ => {}
            }
        }
        UseKind::Value(item)
    }
}

/// The node of this kind that covers exactly this span.
fn node_with_span(root: &SyntaxNode, kind: SyntaxKind, span: Span) -> Option<SyntaxNode> {
    let range = rowan::TextRange::new(span.start().into(), span.end().into());
    let element = root.covering_element(range);
    let node = match element {
        rowan::NodeOrToken::Node(node) => node,
        rowan::NodeOrToken::Token(token) => token.parent()?,
    };
    node.ancestors()
        .find(|node| node.kind() == kind && node.text_range() == range)
}

/// `true` where a bare word reads as an enum word rather than an item: a
/// word of a closed list given as an argument that is not the first one,
/// or as a named argument, or as the value of a field that takes words.
#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "SyntaxKind is a flat set of about 140 token and node tags: this tests the few it needs and treats the rest alike"
)]
fn is_enum_word_position(node: &SyntaxNode, name: &str) -> bool {
    if !crate::words::is_enum_word(name) {
        return false;
    }
    let Some(parent) = node.parent() else {
        return false;
    };
    match parent.kind() {
        SyntaxKind::ARG => {
            let named = parent
                .children()
                .next()
                .is_some_and(|c| c.kind() == SyntaxKind::NAME);
            let first = parent.parent().is_some_and(|list| {
                list.children()
                    .find(|c| c.kind() == SyntaxKind::ARG)
                    .is_some_and(|first| first == parent)
            });
            named || !first
        }
        SyntaxKind::FIELD => parent
            .children()
            .find(|c| c.kind() == SyntaxKind::NAME)
            .and_then(|n| word_of(&n))
            .is_some_and(|(field, _)| crate::words::WORD_FIELDS.contains(&field.as_str())),
        _ => false,
    }
}
