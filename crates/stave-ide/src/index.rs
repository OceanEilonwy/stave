//! Building the index: every declaration of every loaded file as a
//! [`Symbol`], the scopes they live in, and every name written in the
//! source resolved to the symbol it means, as a [`Reference`].
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

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use stave_lang::ast::Span;
use stave_lang::{Diagnostic, FileId, GreenNode, SourceMap, SyntaxKind, SyntaxNode};
use stave_modules::{ModuleId, ModuleSet, ModuleSource};

/// Identifies a [`Symbol`] in an [`crate::Analysis`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SymbolId(u32);

impl SymbolId {
    fn index(self) -> usize {
        usize::try_from(self.0).unwrap_or(usize::MAX)
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

/// A loaded file.
#[derive(Debug, Clone)]
pub(crate) struct FileData {
    pub(crate) path: String,
    pub(crate) module: Option<ModuleId>,
    pub(crate) green: GreenNode,
}

impl FileData {
    pub(crate) fn root(&self) -> SyntaxNode {
        SyntaxNode::new_root(self.green.clone())
    }
}

/// A loaded module.
#[derive(Debug, Clone)]
pub(crate) struct ModuleData {
    pub(crate) name: String,
    pub(crate) symbol: Option<SymbolId>,
    pub(crate) files: Vec<FileId>,
    pub(crate) standard: bool,
}

/// Names declared in each scope, keyed first by the scope's id so a
/// lookup by `&str` needs no allocation.
#[derive(Debug, Clone)]
pub(crate) struct Scopes<K>(BTreeMap<K, BTreeMap<String, SymbolId>>);

impl<K: Ord> Scopes<K> {
    fn new() -> Self {
        Self(BTreeMap::new())
    }

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

/// Everything the queries read: built once per change of the files.
#[derive(Debug, Clone)]
pub(crate) struct Index {
    pub(crate) folder: PathBuf,
    pub(crate) sources: SourceMap,
    pub(crate) files: BTreeMap<FileId, FileData>,
    pub(crate) modules: BTreeMap<ModuleId, ModuleData>,
    pub(crate) root: ModuleId,
    pub(crate) prelude: Vec<ModuleId>,
    pub(crate) symbols: Vec<Symbol>,
    pub(crate) module_scope: Scopes<ModuleId>,
    pub(crate) inner_scope: Scopes<SymbolId>,
    /// Each file's import names.
    pub(crate) imports: Scopes<FileId>,
    /// The symbol an item node declares, by file and node start.
    item_at: BTreeMap<(FileId, u32), SymbolId>,
    /// The component each assembly is an instance of.
    instance_of: BTreeMap<SymbolId, SymbolId>,
    /// Every reference, sorted by file and position.
    pub(crate) references: Vec<Reference>,
    /// Indices into `references`, by symbol.
    pub(crate) uses: BTreeMap<SymbolId, Vec<usize>>,
    /// The loader's diagnostics: syntax, imports and module headers.
    pub(crate) diagnostics: Vec<Diagnostic>,
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

impl Index {
    /// Indexes every file of a loaded module set.
    pub(crate) fn new(folder: &Path, set: ModuleSet) -> Index {
        let mut index = Index {
            folder: folder.to_path_buf(),
            sources: set.sources,
            files: BTreeMap::new(),
            modules: BTreeMap::new(),
            root: set.root,
            prelude: set.prelude,
            symbols: Vec::new(),
            module_scope: Scopes::new(),
            inner_scope: Scopes::new(),
            imports: Scopes::new(),
            item_at: BTreeMap::new(),
            instance_of: BTreeMap::new(),
            references: Vec::new(),
            uses: BTreeMap::new(),
            diagnostics: set.diagnostics,
        };
        // Parse every module file again for its lossless tree (the loader
        // keeps only the typed one).
        for module in &set.modules {
            let mut files = Vec::with_capacity(module.files.len());
            for file in &module.files {
                let Some(source) = index.sources.get(file.file) else {
                    continue;
                };
                let parse = stave_lang::parse_file(file.file, source.text());
                index.files.insert(
                    file.file,
                    FileData {
                        path: file.path.clone(),
                        module: Some(module.id),
                        green: parse.green,
                    },
                );
                files.push(file.file);
            }
            index.modules.insert(
                module.id,
                ModuleData {
                    name: module.name.clone(),
                    symbol: None,
                    files,
                    standard: matches!(module.source, ModuleSource::Std { .. }),
                },
            );
        }
        // Pass 1: declarations, module by module.
        let module_ids: Vec<ModuleId> = index.modules.keys().copied().collect();
        for module in &module_ids {
            index.declare_module(*module);
        }
        // Imports, which the loader already resolved.
        for module in &set.modules {
            for file in &module.files {
                for import in &file.imports {
                    let Some(symbol) = index
                        .modules
                        .get(&import.module)
                        .and_then(|data| data.symbol)
                    else {
                        continue;
                    };
                    index
                        .imports
                        .0
                        .entry(file.file)
                        .or_default()
                        .insert(import.name.clone(), symbol);
                    index.references.push(Reference {
                        file: file.file,
                        span: import.name_span,
                        symbol,
                        kind: UseKind::Imported,
                    });
                }
            }
        }
        // Pass 2: which component each assembly is an instance of, then
        // every reference.
        index.link_assemblies();
        let file_ids: Vec<FileId> = index.files.keys().copied().collect();
        for file in file_ids {
            index.resolve_file(file);
        }
        index
            .references
            .sort_by_key(|r| (r.file, r.span.start(), r.span.end()));
        index.references.dedup();
        for (position, reference) in index.references.iter().enumerate() {
            index
                .uses
                .entry(reference.symbol)
                .or_default()
                .push(position);
        }
        index
    }

    pub(crate) fn symbol(&self, id: SymbolId) -> Option<&Symbol> {
        self.symbols.get(id.index())
    }

    fn push_symbol(&mut self, mut symbol: Symbol) -> SymbolId {
        let id = SymbolId(u32::try_from(self.symbols.len()).unwrap_or(u32::MAX));
        symbol.id = id;
        self.symbols.push(symbol);
        id
    }

    // ---- pass 1: declarations ----

    fn declare_module(&mut self, module: ModuleId) {
        let Some(data) = self.modules.get(&module) else {
            return;
        };
        let name = data.name.clone();
        let files = data.files.clone();
        // The module's own symbol sits on its `library` or `project` item.
        let mut header: Option<(FileId, SyntaxNode)> = None;
        for file in &files {
            let Some(root) = self.files.get(file).map(FileData::root) else {
                continue;
            };
            if header.is_none()
                && let Some(item) = root
                    .children()
                    .find(|child| matches!(child.kind(), SyntaxKind::LIBRARY | SyntaxKind::PROJECT))
            {
                header = Some((*file, item));
            }
        }
        let symbol = header.map(|(file, item)| {
            let title = item
                .children()
                .find(|child| child.kind() == SyntaxKind::STRING)
                .map(|string| span_of(&string))
                .or_else(|| word_of(&item).map(|(_, span)| span))
                .unwrap_or_else(|| span_of(&item));
            self.push_symbol(Symbol {
                id: SymbolId(0),
                kind: SymbolKind::Module,
                name: name.clone(),
                file,
                name_span: title,
                span: span_of(&item),
                container: None,
                module,
                docs: docs(&item),
                private: false,
                signature: signature(&item),
            })
        });
        if let Some(data) = self.modules.get_mut(&module) {
            data.symbol = symbol;
        }
        for file in &files {
            let Some(root) = self.files.get(file).map(FileData::root) else {
                continue;
            };
            for item in root.children() {
                self.declare_item(&item, *file, module, None);
            }
        }
    }

    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "SyntaxKind is a flat set of about 140 token and node tags: this tests the few it needs and treats the rest alike"
    )]
    fn declare_item(
        &mut self,
        node: &SyntaxNode,
        file: FileId,
        module: ModuleId,
        container: Option<SymbolId>,
    ) {
        let Some(kind) = SymbolKind::of_node(node.kind()) else {
            return;
        };
        let Some((name, name_span)) = declared_name(node) else {
            return;
        };
        let private = node
            .children_with_tokens()
            .any(|element| element.kind() == SyntaxKind::KW_PRIVATE);
        let id = self.push_symbol(Symbol {
            id: SymbolId(0),
            kind,
            name: name.clone(),
            file,
            name_span,
            span: span_of(node),
            container,
            module,
            docs: docs(node),
            private,
            signature: signature(node),
        });
        // The first declaration of a name wins; a second is the checker's
        // `duplicate-name`.
        match container {
            Some(container) => self.inner_scope.declare(container, &name, id),
            None => self.module_scope.declare(module, &name, id),
        }
        self.item_at.insert((file, start(node)), id);
        self.references.push(Reference {
            file,
            span: name_span,
            symbol: id,
            kind: UseKind::Declared,
        });
        // An array's labels.
        if let Some(spec) = node
            .children()
            .find(|child| child.kind() == SyntaxKind::ARRAY_SPEC)
        {
            for label in spec.children().filter(|c| c.kind() == SyntaxKind::NAME) {
                let Some((text, span)) = word_of(&label) else {
                    continue;
                };
                let label_id = self.push_symbol(Symbol {
                    id: SymbolId(0),
                    kind: SymbolKind::Label,
                    name: text.clone(),
                    file,
                    name_span: span,
                    span,
                    container: Some(id),
                    module,
                    docs: Vec::new(),
                    private: false,
                    signature: format!("label {text} of {name}"),
                });
                self.inner_scope.declare(id, &text, label_id);
                self.references.push(Reference {
                    file,
                    span,
                    symbol: label_id,
                    kind: UseKind::Declared,
                });
            }
        }
        // What a component or a kind declares inside.
        match node.kind() {
            SyntaxKind::COMPONENT => {
                if let Some(block) = block_of(node) {
                    for child in block.children() {
                        self.declare_item(&child, file, module, Some(id));
                    }
                }
            }
            SyntaxKind::KIND => {
                if let Some(block) = block_of(node) {
                    for child in block.children() {
                        if child.kind() == SyntaxKind::ROLES {
                            if let Some(roles) = block_of(&child) {
                                for role in roles.children() {
                                    self.declare_item(&role, file, module, Some(id));
                                }
                            }
                        } else {
                            self.declare_item(&child, file, module, Some(id));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // ---- scopes ----

    /// The component or kind body a node is inside.
    fn container_at(&self, file: FileId, node: &SyntaxNode) -> Option<SymbolId> {
        node.ancestors()
            .find(|a| matches!(a.kind(), SyntaxKind::COMPONENT | SyntaxKind::KIND))
            .and_then(|a| self.item_at.get(&(file, start(&a))).copied())
    }

    /// The innermost named item a node is inside.
    pub(crate) fn item_around(&self, file: FileId, node: &SyntaxNode) -> Option<SymbolId> {
        node.ancestors()
            .filter(|a| SymbolKind::of_node(a.kind()).is_some())
            .find_map(|a| self.item_at.get(&(file, start(&a))).copied())
    }

    fn visible(&self, symbol: SymbolId, from: Option<ModuleId>) -> bool {
        self.symbol(symbol)
            .is_some_and(|s| !s.private || Some(s.module) == from)
    }

    /// What a bare name means at a node.
    fn lookup(&self, file: FileId, node: &SyntaxNode, name: &str) -> Option<SymbolId> {
        let module = self.files.get(&file).and_then(|f| f.module);
        // The enclosing body first, then further out.
        if let Some(container) = self.container_at(file, node)
            && let Some(found) = self.inner_scope.get(container, name)
        {
            return Some(found);
        }
        if let Some(module) = module
            && let Some(found) = self.module_scope.get(module, name)
        {
            return Some(found);
        }
        if let Some(found) = self.imports.get(file, name) {
            return Some(found);
        }
        self.prelude.iter().find_map(|prelude| {
            self.module_scope
                .get(*prelude, name)
                .filter(|found| self.visible(*found, module))
        })
    }

    /// What `name` after a `.` means when the left side is `base`.
    fn member(&self, file: FileId, base: SymbolId, name: &str) -> Option<SymbolId> {
        let symbol = self.symbol(base)?;
        match symbol.kind {
            SymbolKind::Module => {
                let from = self.files.get(&file).and_then(|f| f.module);
                self.module_scope
                    .get(symbol.module, name)
                    .filter(|found| self.visible(*found, from))
            }
            SymbolKind::Assembly => {
                let component = self.instance_of.get(&base)?;
                self.inner_scope.get(*component, name)
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
    fn label(&self, base: SymbolId, name: &str) -> Option<SymbolId> {
        let found = self.inner_scope.get(base, name)?;
        self.symbol(found)
            .filter(|s| s.kind == SymbolKind::Label)
            .map(|s| s.id)
    }

    /// What an expression names, when it names an item.
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "SyntaxKind is a flat set of about 140 token and node tags: this tests the few it needs and treats the rest alike"
    )]
    fn resolve_expr(&self, file: FileId, node: &SyntaxNode) -> Option<SymbolId> {
        match node.kind() {
            SyntaxKind::NAME_REF => {
                let (name, _) = word_of(node)?;
                self.lookup(file, node, &name)
            }
            SyntaxKind::MEMBER_EXPR => {
                let base = node.children().next()?;
                let base = self.resolve_expr(file, &base)?;
                let member = node
                    .children()
                    .filter(|c| c.kind() == SyntaxKind::NAME)
                    .last()?;
                let (name, _) = word_of(&member)?;
                self.member(file, base, &name)
            }
            SyntaxKind::INDEX_EXPR | SyntaxKind::PAREN_EXPR => {
                let inner = node.children().next()?;
                self.resolve_expr(file, &inner)
            }
            _ => None,
        }
    }

    /// The symbols a `ref` or `path` names, segment by segment, up to (not
    /// including) `until`.
    fn resolve_segments(
        &self,
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
                        self.lookup(file, &child, &name)
                    } else {
                        current.and_then(|base| self.member(file, base, &name))
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
    fn index_base(&self, file: FileId, index: &SyntaxNode) -> Option<SymbolId> {
        let parent = index.parent()?;
        match parent.kind() {
            SyntaxKind::INDEX_EXPR => {
                let base = parent.children().next()?;
                self.resolve_expr(file, &base)
            }
            SyntaxKind::PATH => self
                .resolve_segments(file, &parent, Some(index))
                .last()
                .and_then(|(_, symbol)| *symbol),
            _ => None,
        }
    }

    fn link_assemblies(&mut self) {
        // Instances of a component first, then mirrors (which take their
        // source's component), a few rounds for mirrors of mirrors.
        let mut assemblies: Vec<(FileId, SymbolId, SyntaxNode)> = Vec::new();
        for (file, data) in &self.files {
            for node in data.root().descendants() {
                if node.kind() == SyntaxKind::ASSEMBLY
                    && let Some(id) = self.item_at.get(&(*file, start(&node)))
                {
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
                    self.resolve_segments(*file, &reference, None)
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
                        .and_then(|source| self.resolve_expr(*file, &source))
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

    // ---- pass 2: references ----

    fn resolve_file(&mut self, file: FileId) {
        let Some(root) = self.files.get(&file).map(FileData::root) else {
            return;
        };
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
                            .index_base(file, &parent)
                            .and_then(|base| self.label(base, &name))
                            .or_else(|| {
                                (name != "same")
                                    .then(|| self.lookup(file, &node, &name))
                                    .flatten()
                            }),
                        _ if is_enum_word_position(&node, &name) => None,
                        _ => self.lookup(file, &node, &name),
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
                        .resolve_expr(file, &base)
                        .and_then(|base| self.member(file, base, &name))
                    {
                        found.push((span, symbol, node.clone()));
                    }
                }
                SyntaxKind::REF | SyntaxKind::PATH => {
                    for (span, symbol) in self.resolve_segments(file, &node, None) {
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
                    if let Some(symbol) = self.lookup(file, &node, &name) {
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
                        .item_at
                        .get(&(file, start(&assembly)))
                        .and_then(|a| self.instance_of.get(a))
                        .and_then(|c| self.inner_scope.get(*c, &name))
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
                                self.resolve_segments(file, &r, None)
                                    .last()
                                    .and_then(|(_, s)| *s)
                            }),
                        SyntaxKind::CALL_EXPR => call
                            .children()
                            .next()
                            .and_then(|head| self.resolve_expr(file, &head)),
                        _ => None,
                    });
                    if let Some(symbol) = callee
                        .filter(|c| self.symbol(*c).is_some_and(|s| s.kind == SymbolKind::Kind))
                        .and_then(|kind| self.inner_scope.get(kind, &name))
                    {
                        found.push((span, symbol, node.clone()));
                    }
                }
                _ => {}
            }
        }
        for (span, symbol, node) in found {
            let kind = self.use_kind(file, &node);
            self.references.push(Reference {
                file,
                span,
                symbol,
                kind,
            });
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
