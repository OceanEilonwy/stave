//! Syntax tree for `.stave` files (grammar draft 3, see `docs/grammar.md`).
//!
//! This is a faithful, unresolved representation of the source: names are
//! still strings and expressions are not yet type checked. Name resolution,
//! unit checking and placement lowering happen in later stages, which have
//! their own types.
//!
//! The tree mirrors the grammar rule for rule, and makes what the grammar
//! forbids unrepresentable where it can:
//!
//! - every `{ }` block is an ordered `Vec` of an entry enum, so the formatter
//!   can print a file back in the order it was written;
//! - only part bodies (and hardware bodies) can hold a [`PlaceBlock`];
//!   project, library, stock and assembly bodies are [`FieldBlock`]s;
//! - a component holds [`DesignItem`]s only, so it cannot contain a `kind`;
//! - an [`Ident`] is never a keyword, a [`Labels`] list always has two or more
//!   distinct labels, and a [`ParamSpec`] always has a type or a default.
//!
//! Every node carries a [`Span`] (byte offsets into the source) so that
//! errors can point at the exact text, which matters both for people and for
//! feeding errors back to an AI.
//!
//! The tree is built from the lossless concrete syntax tree
//! ([`crate::syntax`]) by [`crate::parse_file`]: it is an owned, typed view
//! of the well-formed items, and its spans lead back to the source text the
//! concrete tree keeps byte for byte.

use std::fmt;
use std::num::NonZeroU64;

use crate::is_keyword;

// ---------------------------------------------------------------------------
// Spans and names
// ---------------------------------------------------------------------------

/// Byte range `start..end` in one source file. `start <= end` always holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Span {
    start: u32,
    end: u32,
}

impl Span {
    /// A span between two byte offsets, given in either order.
    pub const fn new(a: u32, b: u32) -> Self {
        if a <= b {
            Self { start: a, end: b }
        } else {
            Self { start: b, end: a }
        }
    }

    /// First byte of the span.
    pub const fn start(self) -> u32 {
        self.start
    }

    /// One past the last byte of the span.
    pub const fn end(self) -> u32 {
        self.end
    }

    /// Number of bytes covered.
    pub const fn length(self) -> u32 {
        self.end - self.start
    }

    /// `true` when the span covers no bytes.
    pub const fn is_empty(self) -> bool {
        self.start == self.end
    }

    /// Smallest span covering both `self` and `other`.
    pub fn to(self, other: Span) -> Span {
        Span {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }
}

/// A value together with where it was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Spanned<T> {
    /// The value.
    pub value: T,
    /// Where it was written.
    pub span: Span,
}

/// Why a word can't be used as a name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameError {
    /// The text is not a word: it is empty, starts with a digit or contains a
    /// character other than letters, digits and `_`.
    NotAWord(String),
    /// The word is a keyword, which can't name a new thing.
    Reserved(String),
}

impl fmt::Display for NameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NameError::NotAWord(text) => write!(f, "'{text}' is not a name"),
            NameError::Reserved(word) => {
                write!(f, "'{word}' is a reserved word and can't be used as a name")
            }
        }
    }
}

impl std::error::Error for NameError {}

/// `true` if `text` matches the grammar's `WORD`: a letter or `_`, then
/// letters, digits and `_`.
pub fn is_word(text: &str) -> bool {
    let mut chars = text.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The grammar's `IDENT`: a word that is not a keyword. Used wherever a new
/// name is declared and for bare names in expressions.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Ident {
    name: String,
    span: Span,
}

impl Ident {
    /// Checks that `name` is a word and not a keyword.
    pub fn new(name: impl Into<String>, span: Span) -> Result<Self, NameError> {
        let name = name.into();
        if !is_word(&name) {
            Err(NameError::NotAWord(name))
        } else if is_keyword(&name) {
            Err(NameError::Reserved(name))
        } else {
            Ok(Self { name, span })
        }
    }

    /// The name as written.
    pub fn as_str(&self) -> &str {
        &self.name
    }

    /// Where the name was written.
    pub fn span(&self) -> Span {
        self.span
    }
}

impl fmt::Display for Ident {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name)
    }
}

/// The grammar's `NAME`: any word, keywords included. Used after a `.` and
/// before a `:` (member names, field names, named arguments, role names), so
/// `into.stock` and `split(bay, up, at: divider)` are valid.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Name {
    name: String,
    span: Span,
}

impl Name {
    /// Checks that `name` is a word. Keywords are allowed.
    pub fn new(name: impl Into<String>, span: Span) -> Result<Self, NameError> {
        let name = name.into();
        if is_word(&name) {
            Ok(Self { name, span })
        } else {
            Err(NameError::NotAWord(name))
        }
    }

    /// The name as written.
    pub fn as_str(&self) -> &str {
        &self.name
    }

    /// Where the name was written.
    pub fn span(&self) -> Span {
        self.span
    }
}

impl From<Ident> for Name {
    fn from(ident: Ident) -> Self {
        Self {
            name: ident.name,
            span: ident.span,
        }
    }
}

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name)
    }
}

/// A string literal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StringLit {
    /// The text and `{name}` parts in source order. Each part has its own span,
    /// so names inside strings can be resolved, renamed and highlighted.
    pub parts: Vec<StringPart>,
    /// Where the literal was written, quotes included.
    pub span: Span,
}

impl StringLit {
    /// A literal made of plain text only.
    pub fn text(text: impl Into<String>, span: Span) -> Self {
        Self {
            parts: vec![StringPart::Text {
                text: text.into(),
                span,
            }],
            span,
        }
    }

    /// The parts that are `{name}` interpolations.
    pub fn interpolations(&self) -> impl Iterator<Item = &StringPart> {
        self.parts
            .iter()
            .filter(|part| matches!(part, StringPart::Interp { .. }))
    }
}

/// One piece of a string literal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StringPart {
    /// Literal text with escapes resolved (`\{` becomes `{`).
    Text {
        /// The resolved text.
        text: String,
        /// Where the text was written.
        span: Span,
    },
    /// `{name}` or `{name.member}`: a value inserted into the text. Allowed
    /// only in `note:`, `check:` and `require` messages; elsewhere the checker
    /// reports `error[syntax]`.
    Interp {
        /// The name looked up in the surrounding kind, component or module.
        root: Name,
        /// Members after the root, in order (`{piece.thickness}`).
        members: Vec<Name>,
        /// Where the interpolation was written, braces included.
        span: Span,
    },
}

/// `///` doc comments attached to the node that follows them: one entry per
/// line, without the `///` prefix. Kept so notes reach the inspector and the
/// build manual.
pub type Docs = Vec<String>;

// ---------------------------------------------------------------------------
// Files
// ---------------------------------------------------------------------------

/// One `.stave` file: `stave 1`, its imports, then items.
///
/// A module is every file in one folder; files in a module share their item
/// names. Imports apply only to the file they are written in.
#[derive(Debug, Clone, PartialEq)]
pub struct File {
    /// The `stave <version>` line.
    pub header: Header,
    /// Imports, which come before every item.
    pub imports: Vec<Import>,
    /// Items in source order.
    pub items: Vec<Item>,
}

/// `import "./casework"` or `import "./hardware" as hw`.
///
/// Imports a folder (a module). Its items are used qualified by the module
/// name: the folder's last segment, or `alias`. A folder whose name is a
/// keyword or not a word needs an alias.
#[derive(Debug, Clone, PartialEq)]
pub struct Import {
    /// Relative path, `std:...`, `github:owner/repo/path@ref` or
    /// `https://...#sha256=...`. Resolved by the module loader, not the parser.
    pub source: StringLit,
    /// The name after `as`.
    pub alias: Option<Ident>,
    /// The whole import.
    pub span: Span,
}

/// A patch file: `stave 1` followed by patches. This is what an AI, a
/// suggestion or a mouse tool produces.
#[derive(Debug, Clone, PartialEq)]
pub struct PatchFile {
    /// The `stave <version>` line.
    pub header: Header,
    /// Patches in source order.
    pub patches: Vec<Patch>,
}

/// `stave <version>`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    /// The format version; see [`crate::FORMAT_VERSION`].
    pub version: u32,
    /// The whole header line.
    pub span: Span,
}

// ---------------------------------------------------------------------------
// Items
// ---------------------------------------------------------------------------

/// A top-level declaration with its doc comments and visibility.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    /// Doc comments written above the item.
    pub docs: Docs,
    /// `Some` with the keyword's span for `private` items, which other
    /// modules can't see.
    pub private: Option<Span>,
    /// The declaration itself.
    pub kind: ItemKind,
    /// The whole item, doc comments included.
    pub span: Span,
}

/// The grammar's `item_kind`.
#[derive(Debug, Clone, PartialEq)]
pub enum ItemKind {
    /// `project "Title" { ... }`
    Project(Project),
    /// `library "Title"`
    Library(Library),
    /// `kind name : base { ... }`
    Kind(Kind),
    /// `recipe name : operation { ... }`
    Recipe(Recipe),
    /// `stock name { ... }`
    Stock(Stock),
    /// `profile name { ... }`
    Profile(Profile),
    /// `component Name { ... }`
    Component(Component),
    /// `scene name "Title" { ... }`
    Scene(Scene),
    /// An item that may also appear inside a component.
    Design(DesignItem),
}

impl ItemKind {
    /// The keyword that starts this item.
    pub fn keyword(&self) -> &'static str {
        match self {
            ItemKind::Project(_) => "project",
            ItemKind::Library(_) => "library",
            ItemKind::Kind(_) => ItemTag::Kind.keyword(),
            ItemKind::Recipe(_) => ItemTag::Recipe.keyword(),
            ItemKind::Stock(_) => ItemTag::Stock.keyword(),
            ItemKind::Profile(_) => ItemTag::Profile.keyword(),
            ItemKind::Component(_) => ItemTag::Component.keyword(),
            ItemKind::Scene(_) => ItemTag::Scene.keyword(),
            ItemKind::Design(item) => item.tag().keyword(),
        }
    }

    /// The tag a patch uses to address this item. `None` for the project and
    /// library headers, which a patch can't remove or rename.
    pub fn tag(&self) -> Option<ItemTag> {
        match self {
            ItemKind::Project(_) | ItemKind::Library(_) => None,
            ItemKind::Kind(_) => Some(ItemTag::Kind),
            ItemKind::Recipe(_) => Some(ItemTag::Recipe),
            ItemKind::Stock(_) => Some(ItemTag::Stock),
            ItemKind::Profile(_) => Some(ItemTag::Profile),
            ItemKind::Component(_) => Some(ItemTag::Component),
            ItemKind::Scene(_) => Some(ItemTag::Scene),
            ItemKind::Design(item) => Some(item.tag()),
        }
    }

    /// The item's name. Every item has one except the project and library
    /// headers, which have a title instead.
    pub fn name(&self) -> Option<&Ident> {
        match self {
            ItemKind::Project(_) | ItemKind::Library(_) => None,
            ItemKind::Kind(kind) => Some(&kind.name),
            ItemKind::Recipe(recipe) => Some(&recipe.name),
            ItemKind::Stock(stock) => Some(&stock.name),
            ItemKind::Profile(profile) => Some(&profile.name),
            ItemKind::Component(component) => Some(&component.name),
            ItemKind::Scene(scene) => Some(&scene.name),
            ItemKind::Design(item) => Some(item.name()),
        }
    }
}

/// The grammar's `design_item`: the items that describe a design and may
/// appear both at the top level of a module and inside a component.
#[derive(Debug, Clone, PartialEq)]
pub enum DesignItem {
    /// `param name ...`
    Param(Param),
    /// `let name = expr`
    Let(Let),
    /// `region name = expr`
    Region(Region),
    /// `part name : stock { ... }` or `part name = mirror(...)`
    Part(Part),
    /// `joint name : kind(...)`
    Joint(Joint),
    /// `hardware name : kind(...) { place { ... } }`
    Hardware(Hardware),
    /// `assembly name : Component { ... }` or `assembly name = mirror(...)`
    Assembly(Assembly),
    /// `require name : condition "message"`
    Require(Require),
}

impl DesignItem {
    /// The tag a patch uses to address this item.
    pub fn tag(&self) -> ItemTag {
        match self {
            DesignItem::Param(_) => ItemTag::Param,
            DesignItem::Let(_) => ItemTag::Let,
            DesignItem::Region(_) => ItemTag::Region,
            DesignItem::Part(_) => ItemTag::Part,
            DesignItem::Joint(_) => ItemTag::Joint,
            DesignItem::Hardware(_) => ItemTag::Hardware,
            DesignItem::Assembly(_) => ItemTag::Assembly,
            DesignItem::Require(_) => ItemTag::Require,
        }
    }

    /// The item's name.
    pub fn name(&self) -> &Ident {
        match self {
            DesignItem::Param(param) => &param.name,
            DesignItem::Let(binding) => &binding.name,
            DesignItem::Region(region) => &region.name,
            DesignItem::Part(part) => &part.name,
            DesignItem::Joint(joint) => &joint.name,
            DesignItem::Hardware(hardware) => &hardware.name,
            DesignItem::Assembly(assembly) => &assembly.name,
            DesignItem::Require(require) => &require.name,
        }
    }
}

/// The grammar's `item_tag`: the item keywords a patch can `remove` or
/// `rename`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ItemTag {
    /// `kind`
    Kind,
    /// `recipe`
    Recipe,
    /// `param`
    Param,
    /// `let`
    Let,
    /// `region`
    Region,
    /// `stock`
    Stock,
    /// `part`
    Part,
    /// `joint`
    Joint,
    /// `hardware`
    Hardware,
    /// `assembly`
    Assembly,
    /// `profile`
    Profile,
    /// `component`
    Component,
    /// `scene`
    Scene,
    /// `require`
    Require,
}

impl ItemTag {
    /// Every tag, in the order the grammar lists them.
    pub const ALL: [ItemTag; 14] = [
        ItemTag::Kind,
        ItemTag::Recipe,
        ItemTag::Param,
        ItemTag::Let,
        ItemTag::Region,
        ItemTag::Stock,
        ItemTag::Part,
        ItemTag::Joint,
        ItemTag::Hardware,
        ItemTag::Assembly,
        ItemTag::Profile,
        ItemTag::Component,
        ItemTag::Scene,
        ItemTag::Require,
    ];

    /// The keyword as written in source.
    pub fn keyword(self) -> &'static str {
        match self {
            ItemTag::Kind => "kind",
            ItemTag::Recipe => "recipe",
            ItemTag::Param => "param",
            ItemTag::Let => "let",
            ItemTag::Region => "region",
            ItemTag::Stock => "stock",
            ItemTag::Part => "part",
            ItemTag::Joint => "joint",
            ItemTag::Hardware => "hardware",
            ItemTag::Assembly => "assembly",
            ItemTag::Profile => "profile",
            ItemTag::Component => "component",
            ItemTag::Scene => "scene",
            ItemTag::Require => "require",
        }
    }

    /// The tag for a keyword, if it is one a patch can address.
    pub fn from_keyword(word: &str) -> Option<ItemTag> {
        ItemTag::ALL.into_iter().find(|tag| tag.keyword() == word)
    }
}

/// `project "Title" { goal: "...", units: mm, tools: [...] }`
#[derive(Debug, Clone, PartialEq)]
pub struct Project {
    /// The project's title.
    pub title: StringLit,
    /// Its fields. A project body can't hold `place`.
    pub body: FieldBlock,
}

/// `library "Shop hardware"`: marks the folder as a library module.
#[derive(Debug, Clone, PartialEq)]
pub struct Library {
    /// The library's title.
    pub title: StringLit,
    /// Its fields, when it has a body. A library body can't hold `place`.
    pub body: Option<FieldBlock>,
}

/// A joint kind, a hardware kind or a tool, written in `.stave`.
///
/// ```text
/// kind dado : joint {
///   param depth = into.thickness / 3
///   require not_too_deep : depth <= into.thickness / 2 "..."
///   insert:    [push(piece, along: width)]
///   cut_first: into
///   adds:      extend(piece, by: depth)
///   cut crossing(into, piece) {
///     notch(depth: depth)
///   }
/// }
///
/// kind side_mount_slide : slide {
///   param length = 500mm within [350mm, 400mm, 450mm, 500mm]
///   clearance: 12.7mm
///   envelope:  box(length: length, width: 45mm, thickness: 12.7mm)
/// }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Kind {
    /// The kind's name.
    pub name: Ident,
    /// What it is a kind of.
    pub base: Spanned<BaseKind>,
    /// Its entries, in source order.
    pub body: KindBody,
}

/// The grammar's `base_kind`: what a `kind` declares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize),
    serde(rename_all = "snake_case")
)]
pub enum BaseKind {
    /// A joint between parts.
    Joint,
    /// A drawer slide: gives its assembly a sliding motion.
    Slide,
    /// A hinge: gives its assembly a turning motion.
    Hinge,
    /// A knob or pull.
    Pull,
    /// A dowel pin, screw or other fastener that joints consume.
    Fastener,
    /// A tool that recipes need.
    Tool,
}

impl BaseKind {
    /// Every base kind, in the order the grammar lists them.
    pub const ALL: [BaseKind; 6] = [
        BaseKind::Joint,
        BaseKind::Slide,
        BaseKind::Hinge,
        BaseKind::Pull,
        BaseKind::Fastener,
        BaseKind::Tool,
    ];

    /// The word as written after `kind name :`.
    pub fn word(self) -> &'static str {
        match self {
            BaseKind::Joint => "joint",
            BaseKind::Slide => "slide",
            BaseKind::Hinge => "hinge",
            BaseKind::Pull => "pull",
            BaseKind::Fastener => "fastener",
            BaseKind::Tool => "tool",
        }
    }

    /// The base kind for a word, if it is one.
    pub fn from_word(word: &str) -> Option<BaseKind> {
        BaseKind::ALL.into_iter().find(|base| base.word() == word)
    }

    /// `true` for the bases that are mounted on a part with a `hardware`
    /// declaration.
    pub fn is_mounted_hardware(self) -> bool {
        match self {
            BaseKind::Slide | BaseKind::Hinge | BaseKind::Pull => true,
            BaseKind::Joint | BaseKind::Fastener | BaseKind::Tool => false,
        }
    }
}

/// The `{ ... }` of a `kind`.
#[derive(Debug, Clone, PartialEq)]
pub struct KindBody {
    /// Entries in source order.
    pub entries: Vec<KindEntry>,
    /// The braces and everything between them.
    pub span: Span,
}

impl KindBody {
    /// The first field with this name.
    pub fn field(&self, key: &str) -> Option<&Field> {
        self.entries.iter().find_map(|entry| match entry {
            KindEntry::Field(field) if field.key.as_str() == key => Some(field),
            KindEntry::Field(_)
            | KindEntry::Param(_)
            | KindEntry::Require(_)
            | KindEntry::Makes(_)
            | KindEntry::Member(_)
            | KindEntry::Roles(_)
            | KindEntry::Cut(_) => None,
        })
    }
}

/// A node that may carry doc comments.
#[derive(Debug, Clone, PartialEq)]
pub struct Documented<T> {
    /// Doc comments written above the node.
    pub docs: Docs,
    /// The node.
    pub node: T,
    /// The node with its doc comments.
    pub span: Span,
}

/// The grammar's `kind_entry`.
#[derive(Debug, Clone, PartialEq)]
pub enum KindEntry {
    /// `param depth = into.thickness / 3`
    Param(Documented<Param>),
    /// `require not_too_deep : depth <= into.thickness / 2 "..."`
    Require(Documented<Require>),
    /// `makes part key : key_stock { ... }`
    Makes(Documented<MadePart>),
    /// `member cup`: a named, documented piece of a hardware kind.
    Member(Documented<KindMember>),
    /// `roles { piece: Part, into: Part }`
    Roles(RolesBlock),
    /// `cut end(piece, facing: into) { tenon(...) }`
    Cut(Cut),
    /// `insert: ...`, `adds: ...`, `envelope: ...`
    Field(Field),
}

/// `member cup`: one named piece of a hardware kind (a cup hinge's cup,
/// plate, arm and links), documented by the doc comments above it.
#[derive(Debug, Clone, PartialEq)]
pub struct KindMember {
    /// The member's name: one of its base's member words.
    pub name: Ident,
}

/// `roles { lock: Part, first: Part, second: Part }`
#[derive(Debug, Clone, PartialEq)]
pub struct RolesBlock {
    /// Roles in declaration order. The first one takes the positional
    /// argument of a kind call.
    pub roles: Vec<Role>,
    /// The `roles` word and its block.
    pub span: Span,
}

/// `piece: Part` or `into: Parts`
#[derive(Debug, Clone, PartialEq)]
pub struct Role {
    /// The role's name; may be any word.
    pub name: Name,
    /// The type name as written: `Part` or `Parts`. Checked with
    /// [`TypeName::from_word`] during type checking.
    pub ty: Ident,
    /// The whole entry.
    pub span: Span,
}

/// `cut end(piece, facing: into) { tenon(...) }`
#[derive(Debug, Clone, PartialEq)]
pub struct Cut {
    /// Face selector: `end(...)`, `side(...)` or `crossing(...)`.
    pub selector: Call,
    /// Cut operations, one per line: `notch(depth: depth)`, `outline(...)`.
    pub operations: Vec<Call>,
    /// The whole entry.
    pub span: Span,
}

/// A part a joint creates (a key, a wedge, a pin), once per pair:
/// `makes part key : key_stock { ... }`. Its `place` is relative to the
/// joint's own parts.
#[derive(Debug, Clone, PartialEq)]
pub struct MadePart {
    /// The made part's name.
    pub name: Ident,
    /// The stock it is cut from; often a param of the kind.
    pub stock: Ref,
    /// Its fields and placement.
    pub body: PartBody,
}

/// How to make one operation with a set of tools. Work cards are built from
/// these.
///
/// ```text
/// recipe notch_by_hand : notch {
///   tools: [marking_knife, square, marking_gauge, tenon_saw, chisel, mallet, router_plane]
///   rank:  20
///   mark {
///     knife(walls)
///     gauge(depth, on: open_edges)
///   }
///   cut {
///     saw(walls, to: depth)
///     chop(waste, with: chisel)
///   }
///   check: "the mating part pushes in by hand"
/// }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Recipe {
    /// The recipe's name.
    pub name: Ident,
    /// The operation this recipe makes, as written. Checked with
    /// [`Operation::from_word`] during type checking.
    pub operation: Ident,
    /// Its entries, in source order.
    pub body: RecipeBody,
}

/// The `{ ... }` of a `recipe`.
#[derive(Debug, Clone, PartialEq)]
pub struct RecipeBody {
    /// Entries in source order.
    pub entries: Vec<RecipeEntry>,
    /// The braces and everything between them.
    pub span: Span,
}

/// The grammar's `recipe_entry`.
#[derive(Debug, Clone, PartialEq)]
pub enum RecipeEntry {
    /// `mark { ... }`: marking-out steps.
    Mark(StepBlock),
    /// `cut { ... }`: cutting steps.
    Cut(StepBlock),
    /// `tools: [...]`, `rank: 10`, `needs: [...]`, `check: "..."`
    Field(Field),
}

/// The steps of a `mark { }` or `cut { }` block, one per line.
#[derive(Debug, Clone, PartialEq)]
pub struct StepBlock {
    /// Steps in the order they are done: `knife(walls)`, `saw(walls, to: depth)`.
    pub steps: Vec<Call>,
    /// The `mark` or `cut` word and its block.
    pub span: Span,
}

/// `param cabinet_width = 900mm within 600mm..1200mm`
/// `param slide : SlideKind`
/// `param reveal = 1.5mm locked`
///
/// The same form in modules, components and kinds. The domain lets the
/// checker verify the design for every allowed value; `locked` stops smart
/// edits from changing it.
#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    /// The param's name.
    pub name: Ident,
    /// Its type, its default, or both.
    pub spec: ParamSpec,
    /// The allowed values.
    pub within: Option<Domain>,
    /// `Some` with the keyword's span when the param is `locked`.
    pub locked: Option<Span>,
}

/// What a `param` declares: a type, a default, or both. A param with neither
/// can't be written.
#[derive(Debug, Clone, PartialEq)]
pub enum ParamSpec {
    /// `param opening : Region`: must be supplied by the user.
    Type(Ident),
    /// `param box_height = 180mm`: the type is the default's type.
    Default(Expr),
    /// `param joinery : JointKind = cope_and_stick`
    TypeAndDefault {
        /// The type name as written.
        ty: Ident,
        /// The default value.
        default: Expr,
    },
}

impl ParamSpec {
    /// The type name as written, if one was. Checked with
    /// [`TypeName::from_word`] during type checking.
    pub fn ty(&self) -> Option<&Ident> {
        match self {
            ParamSpec::Type(ty) | ParamSpec::TypeAndDefault { ty, .. } => Some(ty),
            ParamSpec::Default(_) => None,
        }
    }

    /// The default value, if one was written.
    pub fn default(&self) -> Option<&Expr> {
        match self {
            ParamSpec::Default(default) | ParamSpec::TypeAndDefault { default, .. } => {
                Some(default)
            }
            ParamSpec::Type(_) => None,
        }
    }
}

/// The values a param may take.
#[derive(Debug, Clone, PartialEq)]
pub enum Domain {
    /// `lo..hi`, inclusive on both ends.
    Range {
        /// Smallest allowed value.
        lo: Expr,
        /// Largest allowed value.
        hi: Expr,
        /// The whole range.
        span: Span,
    },
    /// `[a, b, c]`: exactly one of these values.
    OneOf {
        /// The allowed values.
        values: Vec<Expr>,
        /// The whole list.
        span: Span,
    },
}

/// `let overhang = 20mm`
#[derive(Debug, Clone, PartialEq)]
pub struct Let {
    /// The value's name.
    pub name: Ident,
    /// The value.
    pub value: Expr,
}

/// `region bay[drawers, doors] = split(front_opening, up, at: divider)`
#[derive(Debug, Clone, PartialEq)]
pub struct Region {
    /// The region's name.
    pub name: Ident,
    /// `None` for a single region.
    pub array: Option<ArraySpec>,
    /// `opening(...)`, `split(...)` or `cover(...)`.
    pub value: Expr,
}

/// `stock oak_19 { species: white_oak, form: solid, thickness: 19mm }`
#[derive(Debug, Clone, PartialEq)]
pub struct Stock {
    /// The stock's name.
    pub name: Ident,
    /// Its fields. A stock body can't hold `place`.
    pub body: FieldBlock,
}

/// `part shelf : oak_19 { ... }` or `part side_r = mirror(side_l, about: ...)`
#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    /// The part's name.
    pub name: Ident,
    /// `None` for a single part.
    pub array: Option<ArraySpec>,
    /// How the part is defined.
    pub def: PartDef,
}

/// The two ways to define a part.
#[derive(Debug, Clone, PartialEq)]
pub enum PartDef {
    /// Cut from stock: `: oak_19 { length: ..., place { ... } }`
    FromStock {
        /// The stock it is cut from; may be qualified or a component param.
        stock: Ref,
        /// Its fields and placement.
        body: PartBody,
    },
    /// The mirror image of another part.
    Mirror(Mirror),
}

/// `mirror(side_l, about: mid(base, across))`: the only way to derive a part
/// or assembly from another. Defined on results: the solved source is
/// reflected through the plane.
#[derive(Debug, Clone, PartialEq)]
pub struct Mirror {
    /// The part or assembly to mirror.
    pub source: Expr,
    /// The plane to mirror through: an anchor.
    pub about: Expr,
    /// The whole call.
    pub span: Span,
}

/// The `{ ... }` of a part: fields and at most one `place`.
#[derive(Debug, Clone, PartialEq)]
pub struct PartBody {
    /// Entries in source order.
    pub entries: Vec<PartEntry>,
    /// The braces and everything between them.
    pub span: Span,
}

impl PartBody {
    /// The first field with this name.
    pub fn field(&self, key: &str) -> Option<&Field> {
        self.fields().find(|field| field.key.as_str() == key)
    }

    /// The fields, in source order.
    pub fn fields(&self) -> impl Iterator<Item = &Field> {
        self.entries.iter().filter_map(|entry| match entry {
            PartEntry::Field(field) => Some(field),
            PartEntry::Place(_) => None,
        })
    }

    /// The `place { }` block. The parser rejects a second one.
    pub fn place(&self) -> Option<&PlaceBlock> {
        self.entries.iter().find_map(|entry| match entry {
            PartEntry::Place(place) => Some(place),
            PartEntry::Field(_) => None,
        })
    }
}

/// The grammar's `part_entry`.
#[derive(Debug, Clone, PartialEq)]
pub enum PartEntry {
    /// `length: fit`
    Field(Field),
    /// `place { ... }`
    Place(PlaceBlock),
}

/// How an array of parts, regions, assemblies, joints or hardware is declared.
#[derive(Debug, Clone, PartialEq)]
pub enum ArraySpec {
    /// `shelf[3]`, `shelf[shelf_count]`. Elements are `shelf[0]`, `shelf[1]`, ...
    Count(Expr),
    /// `drawer[bottom, top]`. Elements are indexed by label only.
    Labels(Labels),
}

/// The labels of a labelled array: two or more, all different.
#[derive(Debug, Clone, PartialEq)]
pub struct Labels {
    labels: Vec<Ident>,
    span: Span,
}

/// Why a list of names can't be the labels of an array.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LabelsError {
    /// Fewer than two labels. A single label isn't allowed: just name the item.
    TooFew,
    /// The same label twice; `span` is the second one.
    Duplicate {
        /// The repeated label.
        label: String,
        /// Where it is repeated.
        span: Span,
    },
}

impl fmt::Display for LabelsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LabelsError::TooFew => write!(f, "a labelled array needs two or more labels"),
            LabelsError::Duplicate { label, .. } => write!(f, "'{label}' is declared twice"),
        }
    }
}

impl std::error::Error for LabelsError {}

impl Labels {
    /// Checks that there are two or more labels and that none repeats.
    pub fn new(labels: Vec<Ident>, span: Span) -> Result<Self, LabelsError> {
        if labels.len() < 2 {
            return Err(LabelsError::TooFew);
        }
        for (index, label) in labels.iter().enumerate() {
            if labels
                .iter()
                .take(index)
                .any(|earlier| earlier.as_str() == label.as_str())
            {
                return Err(LabelsError::Duplicate {
                    label: label.as_str().to_owned(),
                    span: label.span(),
                });
            }
        }
        Ok(Self { labels, span })
    }

    /// The labels in declaration order.
    pub fn as_slice(&self) -> &[Ident] {
        &self.labels
    }

    /// The brackets and everything between them.
    pub fn span(&self) -> Span {
        self.span
    }

    /// Position of a label in declaration order.
    pub fn position(&self, label: &str) -> Option<usize> {
        self.labels
            .iter()
            .position(|candidate| candidate.as_str() == label)
    }
}

/// `joint back_rabbets : rabbet(back, into: [side_l, side_r], depth: 9mm)`
///
/// A joint has no body: every setting is an argument of the kind call.
#[derive(Debug, Clone, PartialEq)]
pub struct Joint {
    /// The joint's name.
    pub name: Ident,
    /// `None` for a single joint.
    pub array: Option<ArraySpec>,
    /// The kind, its parts by role and its settings.
    pub call: KindCall,
}

/// `hardware slides[left, right] : slide(side[same], in: opening)`
///
/// ```text
/// hardware handle : knob_kind(stile[latch]) {
///   place {
///     center(stile[latch], across)
///     center(stile[latch], up)
///   }
/// }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Hardware {
    /// The hardware's name.
    pub name: Ident,
    /// `None` for a single piece.
    pub array: Option<ArraySpec>,
    /// The kind, its parts by role and its settings.
    pub call: KindCall,
    /// The body, when one was written. It can hold nothing but `place`.
    pub body: Option<HardwareBody>,
}

/// The `{ ... }` of a hardware declaration: at most one `place`.
#[derive(Debug, Clone, PartialEq)]
pub struct HardwareBody {
    /// The `place { }` block, when the body isn't empty.
    pub place: Option<PlaceBlock>,
    /// The braces and everything between them.
    pub span: Span,
}

/// The grammar's `kind_call`: `rabbet(back, into: [side_l, side_r], depth: 9mm)`.
///
/// The one positional argument binds the kind's first role; named arguments
/// give the other roles and the settings.
#[derive(Debug, Clone, PartialEq)]
pub struct KindCall {
    /// A kind (`dado`, `japanese.ari_tsugi`) or a param holding a kind value.
    pub kind: Ref,
    /// Roles and settings.
    pub args: Vec<Arg>,
    /// The whole call.
    pub span: Span,
}

/// `assembly drawer[bottom, top] : casework.Drawer { ... }` or
/// `assembly door_r = mirror(door_l, about: mid(base, across))`
#[derive(Debug, Clone, PartialEq)]
pub struct Assembly {
    /// The assembly's name.
    pub name: Ident,
    /// `None` for a single assembly.
    pub array: Option<ArraySpec>,
    /// How the assembly is defined.
    pub def: AssemblyDef,
}

/// The two ways to define an assembly.
#[derive(Debug, Clone, PartialEq)]
pub enum AssemblyDef {
    /// An instance of a component; the body's fields are its params.
    Instance {
        /// The component; may come from another module: `casework.Drawer`.
        component: Ref,
        /// One field per param. An assembly body can't hold `place`: a
        /// component places its own parts against what it is handed.
        body: FieldBlock,
    },
    /// The mirror image of another assembly.
    Mirror(Mirror),
}

/// A profile for turned shapes.
///
/// Stations are measurements along the part (`at 34mm: 70mm`), not world
/// coordinates. The `measure:` field says whether values are diameters or
/// radii.
#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    /// The profile's name.
    pub name: Ident,
    /// Stations and fields in source order.
    pub entries: Vec<ProfileEntry>,
    /// The braces and everything between them.
    pub span: Span,
}

/// The grammar's `profile_entry`.
#[derive(Debug, Clone, PartialEq)]
pub enum ProfileEntry {
    /// `at <station>: <value>`
    Station {
        /// Distance from the start of the part.
        at: Expr,
        /// Diameter or radius there.
        value: Expr,
        /// The whole entry.
        span: Span,
    },
    /// `measure: diameter`, `smooth: true`
    Field(Field),
}

/// A reusable sub-design: `component Drawer { param opening : Region ... }`.
#[derive(Debug, Clone, PartialEq)]
pub struct Component {
    /// The component's name.
    pub name: Ident,
    /// Its params, parts, joints and so on, in source order.
    pub items: Vec<ComponentItem>,
    /// The braces and everything between them.
    pub span: Span,
}

/// An item inside a component, with its doc comments.
pub type ComponentItem = Documented<DesignItem>;

/// A named view of the design: motion states and visibility.
///
/// ```text
/// scene all_open "All open" {
///   pose door_l = open
///   pose drawer[top] = open
/// }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Scene {
    /// The scene's name.
    pub name: Ident,
    /// The title shown in the app.
    pub title: Option<StringLit>,
    /// Poses and fields in source order.
    pub entries: Vec<SceneEntry>,
    /// The braces and everything between them.
    pub span: Span,
}

/// The grammar's `scene_entry`.
#[derive(Debug, Clone, PartialEq)]
pub enum SceneEntry {
    /// `pose drawer[top] = open`, `pose door_l = 90deg`
    Pose {
        /// The assembly to pose.
        target: Path,
        /// `open`, `closed`, an angle or a length.
        value: Expr,
        /// The whole entry.
        span: Span,
    },
    /// `show: drawer[top]`, `explode: true`
    Field(Field),
}

/// `require hanger_room : cabinet_depth - back.thickness >= 540mm "room for coat hangers"`
#[derive(Debug, Clone, PartialEq)]
pub struct Require {
    /// The check's name.
    pub name: Ident,
    /// What must stay true.
    pub condition: Expr,
    /// What to tell the user when it isn't.
    pub message: StringLit,
}

// ---------------------------------------------------------------------------
// Fields, placement, calls and references
// ---------------------------------------------------------------------------

/// A `{ ... }` that holds fields only: the body of a project, library, stock
/// or assembly. It has no way to hold `place`.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldBlock {
    /// Fields in source order.
    pub fields: Vec<Field>,
    /// The braces and everything between them.
    pub span: Span,
}

impl FieldBlock {
    /// The first field with this name.
    pub fn field(&self, key: &str) -> Option<&Field> {
        self.fields.iter().find(|field| field.key.as_str() == key)
    }
}

/// `key: value`. Always exactly one expression; lists use `[ ]`.
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    /// The field's name; may be any word.
    pub key: Name,
    /// Its value.
    pub value: Expr,
    /// The whole field.
    pub span: Span,
}

/// `place { against(base.top) \n flush(base.left) \n ... }`
///
/// One term per line. Because each axis is set by exactly one term, a patch
/// can address a single term by axis: `set side_l.place.deep = ...`.
#[derive(Debug, Clone, PartialEq)]
pub struct PlaceBlock {
    /// Placement terms in source order.
    pub terms: Vec<Call>,
    /// The `place` keyword and its block.
    pub span: Span,
}

/// The grammar's `call`: a head fixed by its position, with optional
/// arguments. Placement terms, face selectors, cut operations and recipe
/// steps are calls. `ground` is a call without parentheses.
#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    /// The head: `flush`, `crossing`, `notch`, `saw`, ... Looked up in the
    /// vocabulary of the position it is written in, never among items.
    pub head: Ident,
    /// The arguments; empty when there are no parentheses.
    pub args: Vec<Arg>,
    /// The whole call.
    pub span: Span,
}

/// The grammar's `ref`: a possibly qualified name without indexes, such as
/// `oak_19`, `casework.Drawer` or `japanese.ari_tsugi`.
#[derive(Debug, Clone, PartialEq)]
pub struct Ref {
    /// The first segment: an item or a module.
    pub root: Ident,
    /// The segments after each `.`.
    pub rest: Vec<Name>,
    /// The whole reference.
    pub span: Span,
}

/// The grammar's `path`: `side_l`, `base.top`, `drawer[top]`,
/// `side_l.place.deep`.
///
/// Used where the grammar requires a reference that may be indexed (`pose`
/// targets and patch targets). Inside expressions the same text is a postfix
/// chain of [`ExprKind`]; see [`Expr::as_path`].
#[derive(Debug, Clone, PartialEq)]
pub struct Path {
    /// The first segment.
    pub root: Ident,
    /// Members and indexes after it.
    pub segments: Vec<PathSegment>,
    /// The whole path.
    pub span: Span,
}

/// One step of a [`Path`].
#[derive(Debug, Clone, PartialEq)]
pub enum PathSegment {
    /// `.name`
    Member(Name),
    /// `[index]`
    Index(Index),
}

/// What is written between `[` and `]` after a name.
#[derive(Debug, Clone, PartialEq)]
pub enum Index {
    /// `[*]`: every element. Inside a list it flattens.
    All(Span),
    /// `[same]`: inside an array declaration, the element with the same label
    /// or number as the element being declared.
    Same(Span),
    /// `[top]` or `[0]`: a label of a labelled array, or a count for a counted
    /// array.
    ///
    /// The parser can't tell which, so a label is an [`ExprKind::Name`] here.
    /// Resolution looks the word up among the labels of a labelled array and
    /// nowhere else.
    At(Box<Expr>),
}

// ---------------------------------------------------------------------------
// Expressions
// ---------------------------------------------------------------------------

/// An expression with its span.
#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    /// What kind of expression it is.
    pub kind: ExprKind,
    /// The whole expression.
    pub span: Span,
}

/// The grammar's expression forms.
#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    /// `3/4in`, `19mm`, `110deg`
    Quantity(Quantity),
    /// Unitless: `3`, `0.5`
    Number(Number),
    /// `"text"` or `"Halve {first}"`: text with escapes resolved, and the
    /// `{name}` parts with their spans. Interpolation is allowed only in
    /// `note:`, `check:` and `require` messages; the parser reports it
    /// anywhere else.
    String(StringLit),
    /// `true` or `false`
    Bool(bool),
    /// `fit`: the size is decided by the part's joints or its profile.
    Fit,
    /// `cabinet_width`, `evenly`, `up`: an item or a bare enum word; which one
    /// depends on the type expected at this position.
    Name(Ident),
    /// `base.top`
    Member {
        /// What is left of the `.`.
        base: Box<Expr>,
        /// The member's name; may be any word.
        member: Name,
    },
    /// `shelf[0]`, `drawer[top]`, `side[*]`, `drawer_bay[same]`
    Index {
        /// What is indexed.
        base: Box<Expr>,
        /// The index.
        index: Index,
    },
    /// `between(side_l, side_r)`, `side_mount_slide(length: 450mm)`
    Call {
        /// What is called: a built-in function or a kind.
        callee: Box<Expr>,
        /// Positional arguments first, then named ones.
        args: Vec<Arg>,
    },
    /// `-x`, `!x`
    Unary {
        /// The operator.
        op: UnaryOp,
        /// What it applies to.
        operand: Box<Expr>,
    },
    /// `a + b`, `a <= b`, `a && b`
    Binary {
        /// The operator.
        op: BinaryOp,
        /// Left operand.
        lhs: Box<Expr>,
        /// Right operand.
        rhs: Box<Expr>,
    },
    /// `[a, b, c]`
    List(Vec<Expr>),
    /// `( expr )`, kept so the formatter can preserve intent.
    Paren(Box<Expr>),
}

impl Expr {
    /// Convert a postfix chain of names, members and indexes into a [`Path`].
    /// Returns `None` if the expression contains anything else.
    pub fn as_path(&self) -> Option<Path> {
        fn walk(expr: &Expr, segments: &mut Vec<PathSegment>) -> Option<Ident> {
            match &expr.kind {
                ExprKind::Name(ident) => Some(ident.clone()),
                ExprKind::Member { base, member } => {
                    let root = walk(base, segments)?;
                    segments.push(PathSegment::Member(member.clone()));
                    Some(root)
                }
                ExprKind::Index { base, index } => {
                    let root = walk(base, segments)?;
                    segments.push(PathSegment::Index(index.clone()));
                    Some(root)
                }
                ExprKind::Quantity(_)
                | ExprKind::Number(_)
                | ExprKind::String(_)
                | ExprKind::Bool(_)
                | ExprKind::Fit
                | ExprKind::Call { .. }
                | ExprKind::Unary { .. }
                | ExprKind::Binary { .. }
                | ExprKind::List(_)
                | ExprKind::Paren(_) => None,
            }
        }
        let mut segments = Vec::new();
        let root = walk(self, &mut segments)?;
        Some(Path {
            root,
            segments,
            span: self.span,
        })
    }
}

/// One argument of a call.
#[derive(Debug, Clone, PartialEq)]
pub struct Arg {
    /// `Some` for named arguments such as `into: side_l`; may be any word.
    pub name: Option<Name>,
    /// The argument's value.
    pub value: Expr,
    /// The whole argument.
    pub span: Span,
}

/// A prefix operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnaryOp {
    /// `-`
    Neg,
    /// `!`
    Not,
}

impl UnaryOp {
    /// The operator as written.
    pub fn symbol(self) -> &'static str {
        match self {
            UnaryOp::Neg => "-",
            UnaryOp::Not => "!",
        }
    }
}

/// An infix operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BinaryOp {
    /// `||`
    Or,
    /// `&&`
    And,
    /// `==`
    Eq,
    /// `!=`
    Ne,
    /// `<`
    Lt,
    /// `<=`
    Le,
    /// `>`
    Gt,
    /// `>=`
    Ge,
    /// `+`
    Add,
    /// `-`
    Sub,
    /// `*`
    Mul,
    /// `/`
    Div,
}

impl BinaryOp {
    /// Every infix operator.
    pub const ALL: [BinaryOp; 12] = [
        BinaryOp::Or,
        BinaryOp::And,
        BinaryOp::Eq,
        BinaryOp::Ne,
        BinaryOp::Lt,
        BinaryOp::Le,
        BinaryOp::Gt,
        BinaryOp::Ge,
        BinaryOp::Add,
        BinaryOp::Sub,
        BinaryOp::Mul,
        BinaryOp::Div,
    ];

    /// Binding power; higher binds tighter. Shared by parser and formatter.
    pub fn precedence(self) -> u8 {
        match self {
            BinaryOp::Or => 1,
            BinaryOp::And => 2,
            BinaryOp::Eq
            | BinaryOp::Ne
            | BinaryOp::Lt
            | BinaryOp::Le
            | BinaryOp::Gt
            | BinaryOp::Ge => 3,
            BinaryOp::Add | BinaryOp::Sub => 4,
            BinaryOp::Mul | BinaryOp::Div => 5,
        }
    }

    /// `true` for the comparison operators, which don't chain: `a < b < c`
    /// is a syntax error.
    pub fn is_comparison(self) -> bool {
        self.precedence() == 3
    }

    /// The operator as written.
    pub fn symbol(self) -> &'static str {
        match self {
            BinaryOp::Or => "||",
            BinaryOp::And => "&&",
            BinaryOp::Eq => "==",
            BinaryOp::Ne => "!=",
            BinaryOp::Lt => "<",
            BinaryOp::Le => "<=",
            BinaryOp::Gt => ">",
            BinaryOp::Ge => ">=",
            BinaryOp::Add => "+",
            BinaryOp::Sub => "-",
            BinaryOp::Mul => "*",
            BinaryOp::Div => "/",
        }
    }
}

// ---------------------------------------------------------------------------
// Literals
// ---------------------------------------------------------------------------

/// A literal with a unit, exactly as written. Conversion to an exact value
/// happens in [`crate::units`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Quantity {
    /// The number before the unit.
    pub magnitude: Magnitude,
    /// The unit.
    pub unit: Unit,
}

/// Magnitude of a quantity literal. Fractions only exist with a unit:
/// a bare `3/4` is the division `3 / 4`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Magnitude {
    /// `19`
    Integer(u64),
    /// `18.5`
    Decimal(Decimal),
    /// `3/4` (whole = 0) or `1-1/2` (whole = 1)
    Fraction(Fraction),
}

/// Unitless number literal. Evaluated as an exact [`crate::units::Rational`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Number {
    /// `4`: a count.
    Integer(u64),
    /// `0.5`: a ratio.
    Decimal(Decimal),
}

/// `digits / 10^scale`, so `18.25` is `{ digits: 1825, scale: 2 }`.
/// Stored exactly; never a float.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Decimal {
    /// Every digit of the literal, without the point.
    pub digits: u64,
    /// How many of them come after the point.
    pub scale: u8,
}

/// A fraction with an optional whole part: `3/4`, `1-1/2`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Fraction {
    /// The whole part; zero for a plain fraction.
    pub whole: u64,
    /// Above the line.
    pub numerator: u64,
    /// Below the line; never zero.
    pub denominator: NonZeroU64,
}

/// A unit suffix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Unit {
    /// `mm`
    Millimeter,
    /// `cm`
    Centimeter,
    /// `m`
    Meter,
    /// `in`
    Inch,
    /// `ft`
    Foot,
    /// `deg`
    Degree,
}

/// What a unit measures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dimension {
    /// A length.
    Length,
    /// An angle.
    Angle,
}

impl Unit {
    /// Every unit, in the order the grammar lists them.
    pub const ALL: [Unit; 6] = [
        Unit::Millimeter,
        Unit::Centimeter,
        Unit::Meter,
        Unit::Inch,
        Unit::Foot,
        Unit::Degree,
    ];

    /// The unit for a suffix, if it is one.
    pub fn from_suffix(suffix: &str) -> Option<Unit> {
        Unit::ALL.into_iter().find(|unit| unit.suffix() == suffix)
    }

    /// The suffix as written.
    pub fn suffix(self) -> &'static str {
        match self {
            Unit::Millimeter => "mm",
            Unit::Centimeter => "cm",
            Unit::Meter => "m",
            Unit::Inch => "in",
            Unit::Foot => "ft",
            Unit::Degree => "deg",
        }
    }

    /// What the unit measures.
    pub fn dimension(self) -> Dimension {
        match self {
            Unit::Degree => Dimension::Angle,
            Unit::Millimeter | Unit::Centimeter | Unit::Meter | Unit::Inch | Unit::Foot => {
                Dimension::Length
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Closed vocabularies used after parsing
// ---------------------------------------------------------------------------

/// The complete list of types a `param` or a role may name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TypeName {
    /// A length: `19mm`.
    Length,
    /// An angle: `110deg`.
    Angle,
    /// An exact fraction without a unit.
    Ratio,
    /// A whole number; usable wherever a ratio is expected.
    Count,
    /// `true` or `false`.
    Bool,
    /// A string.
    Text,
    /// Exactly one part.
    Part,
    /// One part or a list of parts.
    Parts,
    /// A `stock` item.
    Stock,
    /// A region.
    Region,
    /// A `profile` item.
    Profile,
    /// A face or a mid-plane.
    Anchor,
    /// `up`, `across` or `deep`.
    Axis,
    /// `up`, `down`, `left`, `right`, `front` or `back`.
    Direction,
    /// `overlay(...)` or `inset(...)`.
    Mount,
    /// `turned(profile)`.
    Shape,
    /// `box(...)` or `cylinder(...)`.
    Envelope,
    /// `model(...)`.
    Model,
    /// `evenly`, `ends`, `pitch(d)` or `gap(d)`.
    Spacing,
    /// A kind value of base `joint`.
    JointKind,
    /// A kind value of base `slide`, `hinge` or `pull`.
    HardwareKind,
    /// A kind value of base `slide`.
    SlideKind,
    /// A kind value of base `hinge`.
    HingeKind,
    /// A kind value of base `pull`.
    PullKind,
    /// A kind value of base `fastener`.
    FastenerKind,
    /// A kind of base `tool`.
    Tool,
    /// A `recipe` item.
    Recipe,
}

impl TypeName {
    /// Every type, in the order the grammar's type table lists them.
    pub const ALL: [TypeName; 27] = [
        TypeName::Length,
        TypeName::Angle,
        TypeName::Ratio,
        TypeName::Count,
        TypeName::Bool,
        TypeName::Text,
        TypeName::Part,
        TypeName::Parts,
        TypeName::Stock,
        TypeName::Region,
        TypeName::Profile,
        TypeName::Anchor,
        TypeName::Axis,
        TypeName::Direction,
        TypeName::Mount,
        TypeName::Shape,
        TypeName::Envelope,
        TypeName::Model,
        TypeName::Spacing,
        TypeName::JointKind,
        TypeName::HardwareKind,
        TypeName::SlideKind,
        TypeName::HingeKind,
        TypeName::PullKind,
        TypeName::FastenerKind,
        TypeName::Tool,
        TypeName::Recipe,
    ];

    /// The type's name as written in source.
    pub fn word(self) -> &'static str {
        match self {
            TypeName::Length => "Length",
            TypeName::Angle => "Angle",
            TypeName::Ratio => "Ratio",
            TypeName::Count => "Count",
            TypeName::Bool => "Bool",
            TypeName::Text => "Text",
            TypeName::Part => "Part",
            TypeName::Parts => "Parts",
            TypeName::Stock => "Stock",
            TypeName::Region => "Region",
            TypeName::Profile => "Profile",
            TypeName::Anchor => "Anchor",
            TypeName::Axis => "Axis",
            TypeName::Direction => "Direction",
            TypeName::Mount => "Mount",
            TypeName::Shape => "Shape",
            TypeName::Envelope => "Envelope",
            TypeName::Model => "Model",
            TypeName::Spacing => "Spacing",
            TypeName::JointKind => "JointKind",
            TypeName::HardwareKind => "HardwareKind",
            TypeName::SlideKind => "SlideKind",
            TypeName::HingeKind => "HingeKind",
            TypeName::PullKind => "PullKind",
            TypeName::FastenerKind => "FastenerKind",
            TypeName::Tool => "Tool",
            TypeName::Recipe => "Recipe",
        }
    }

    /// The type for a name, if it is one.
    pub fn from_word(word: &str) -> Option<TypeName> {
        TypeName::ALL.into_iter().find(|ty| ty.word() == word)
    }

    /// The type of a kind value with this base. `None` for tools, which are
    /// never kind values with settings.
    pub fn of_kind(base: BaseKind) -> Option<TypeName> {
        match base {
            BaseKind::Joint => Some(TypeName::JointKind),
            BaseKind::Slide => Some(TypeName::SlideKind),
            BaseKind::Hinge => Some(TypeName::HingeKind),
            BaseKind::Pull => Some(TypeName::PullKind),
            BaseKind::Fastener => Some(TypeName::FastenerKind),
            BaseKind::Tool => None,
        }
    }
}

/// A cut operation: what may be written inside a `cut selector { }` block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize),
    serde(rename_all = "snake_case")
)]
pub enum CutOperation {
    /// `notch(depth:)`: the whole footprint, to a depth.
    Notch,
    /// `slot(width:, depth:)`: a slot centred in the footprint.
    Slot,
    /// `pocket(width:, thickness:, depth:)`: a rectangular mortise.
    Pocket,
    /// `tenon(length:, width:, thickness:)`: a rectangular tenon.
    Tenon,
    /// `spigot(length:, diameter:)`: a round tenon.
    Spigot,
    /// `round(diameter:, flats:, flats_angle:)`: a length of a square part
    /// turned round where another part passes, optionally with two flats.
    Round,
    /// `drill(diameter:, depth:, count:)`: round holes.
    Drill,
    /// `dovetail_socket(neck:, depth:, angle:)`
    DovetailSocket,
    /// `dovetail_tail(neck:, depth:, angle:)`
    DovetailTail,
    /// `outline(marks:, depth:)`: a marked-out shape.
    Outline,
}

impl CutOperation {
    /// Every cut operation, in the order the grammar lists them.
    pub const ALL: [CutOperation; 10] = [
        CutOperation::Notch,
        CutOperation::Slot,
        CutOperation::Pocket,
        CutOperation::Tenon,
        CutOperation::Spigot,
        CutOperation::Round,
        CutOperation::Drill,
        CutOperation::DovetailSocket,
        CutOperation::DovetailTail,
        CutOperation::Outline,
    ];

    /// The operation's name as written in source.
    pub fn word(self) -> &'static str {
        match self {
            CutOperation::Notch => "notch",
            CutOperation::Slot => "slot",
            CutOperation::Pocket => "pocket",
            CutOperation::Tenon => "tenon",
            CutOperation::Spigot => "spigot",
            CutOperation::Round => "round",
            CutOperation::Drill => "drill",
            CutOperation::DovetailSocket => "dovetail_socket",
            CutOperation::DovetailTail => "dovetail_tail",
            CutOperation::Outline => "outline",
        }
    }

    /// The cut operation for a name, if it is one.
    pub fn from_word(word: &str) -> Option<CutOperation> {
        CutOperation::ALL
            .into_iter()
            .find(|operation| operation.word() == word)
    }

    /// `true` when the operation removes only axis-aligned boxes and
    /// cylinders, which the geometry makes exactly.
    pub fn is_axis_aligned(self) -> bool {
        match self {
            CutOperation::Notch
            | CutOperation::Slot
            | CutOperation::Pocket
            | CutOperation::Tenon
            | CutOperation::Spigot
            | CutOperation::Drill => true,
            // A round section is a cylinder, but its flats may sit at any angle.
            CutOperation::Round
            | CutOperation::DovetailSocket
            | CutOperation::DovetailTail
            | CutOperation::Outline => false,
        }
    }
}

/// What a recipe can be written for: a cut operation, or turning a part to a
/// profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize),
    serde(rename_all = "snake_case")
)]
pub enum Operation {
    /// One of the cut operations.
    Cut(CutOperation),
    /// `turning`: a part with `shape: turned(profile)`.
    Turning,
}

impl Operation {
    /// The operation's name as written after `recipe name :`.
    pub fn word(self) -> &'static str {
        match self {
            Operation::Cut(operation) => operation.word(),
            Operation::Turning => "turning",
        }
    }

    /// The operation for a name, if it is one.
    pub fn from_word(word: &str) -> Option<Operation> {
        if word == Operation::Turning.word() {
            Some(Operation::Turning)
        } else {
            CutOperation::from_word(word).map(Operation::Cut)
        }
    }

    /// Every operation: the cut operations, then turning.
    pub fn all() -> impl Iterator<Item = Operation> {
        CutOperation::ALL
            .into_iter()
            .map(Operation::Cut)
            .chain([Operation::Turning])
    }
}

// ---------------------------------------------------------------------------
// Patches
// ---------------------------------------------------------------------------

/// `patch "Add a shoe shelf" { add ..., set ..., remove ... }`
#[derive(Debug, Clone, PartialEq)]
pub struct Patch {
    /// Human-readable summary shown in the accept/reject preview.
    pub summary: StringLit,
    /// Operations, applied in order.
    pub ops: Vec<PatchOp>,
    /// The whole patch.
    pub span: Span,
}

/// One operation of a patch.
#[derive(Debug, Clone, PartialEq)]
pub struct PatchOp {
    /// What it does.
    pub kind: PatchOpKind,
    /// The whole operation.
    pub span: Span,
}

/// The grammar's `patch_op`.
#[derive(Debug, Clone, PartialEq)]
pub enum PatchOpKind {
    /// `add part shoe_shelf : oak_19 { ... } to casework.Drawer in "drawer.stave"`
    Add {
        /// The item to add.
        item: Box<Item>,
        /// `to <component>`: add it inside this component.
        to: Option<Ref>,
        /// `in "file.stave"`: the file within the module.
        file: Option<StringLit>,
    },
    /// `add import "./casework" in "doors.stave"`
    AddImport {
        /// The import to add.
        import: Import,
        /// The file to add it to.
        file: Option<StringLit>,
    },
    /// `replace profile bun_foot { ... }`: swaps the item of the same kind and
    /// name as a whole.
    Replace {
        /// The new item.
        item: Box<Item>,
    },
    /// `set cabinet_width = 950mm`, `set side_l.place.deep = flush(base.back)`.
    /// Adds the value if it isn't there yet.
    Set {
        /// What to set.
        target: Path,
        /// The new value.
        value: Expr,
    },
    /// `unset drawer.pull_kind`: removes a field, argument or placement term.
    Unset {
        /// What to remove.
        target: Path,
    },
    /// `remove joint shoe_shelf_dados`, `remove part casework.Drawer.back`
    Remove {
        /// The kind of item.
        tag: ItemTag,
        /// The item; may reach into a component.
        target: Path,
    },
    /// `remove import "./casework" in "doors.stave"`
    RemoveImport {
        /// The import's source string.
        source: StringLit,
        /// The file to remove it from.
        file: Option<StringLit>,
    },
    /// `rename part side_l to left_side`: renames an item and every reference
    /// to it.
    Rename {
        /// The kind of item.
        tag: ItemTag,
        /// The item to rename.
        target: Path,
        /// Its new name.
        to: Ident,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ident(name: &str) -> Ident {
        Ident::new(name, Span::default()).unwrap()
    }

    fn name_expr(name: &str) -> Expr {
        Expr {
            kind: ExprKind::Name(ident(name)),
            span: Span::default(),
        }
    }

    #[test]
    fn spans_are_ordered_and_join() {
        let span = Span::new(9, 4);
        assert_eq!((span.start(), span.end(), span.length()), (4, 9, 5));
        assert!(!span.is_empty());
        assert!(Span::new(3, 3).is_empty());
        assert_eq!(Span::new(2, 5).to(Span::new(4, 12)), Span::new(2, 12));
    }

    #[test]
    fn an_ident_is_never_a_keyword() {
        assert_eq!(ident("side_l").as_str(), "side_l");
        assert_eq!(ident("top").to_string(), "top");
        assert_eq!(
            Ident::new("joint", Span::default()),
            Err(NameError::Reserved("joint".to_owned()))
        );
        assert_eq!(
            Ident::new("joint", Span::default())
                .unwrap_err()
                .to_string(),
            "'joint' is a reserved word and can't be used as a name"
        );
        for bad in ["", "3d", "my-shop", "a b", "é"] {
            assert_eq!(
                Ident::new(bad, Span::default()),
                Err(NameError::NotAWord(bad.to_owned()))
            );
        }
    }

    #[test]
    fn a_name_may_be_a_keyword() {
        for word in ["stock", "place", "at", "part", "into", "_private", "x1"] {
            assert_eq!(Name::new(word, Span::default()).unwrap().as_str(), word);
        }
        assert!(Name::new("9mm", Span::default()).is_err());
        assert_eq!(Name::from(ident("depth")).as_str(), "depth");
    }

    #[test]
    fn labels_need_two_distinct_names() {
        let labels = Labels::new(vec![ident("bottom"), ident("top")], Span::new(0, 13)).unwrap();
        assert_eq!(labels.position("top"), Some(1));
        assert_eq!(labels.position("middle"), None);
        assert_eq!(labels.as_slice().len(), 2);
        assert_eq!(
            Labels::new(vec![ident("only")], Span::default()),
            Err(LabelsError::TooFew)
        );
        let repeated = Ident::new("top", Span::new(8, 11)).unwrap();
        assert_eq!(
            Labels::new(vec![ident("top"), repeated], Span::default()),
            Err(LabelsError::Duplicate {
                label: "top".to_owned(),
                span: Span::new(8, 11)
            })
        );
    }

    #[test]
    fn a_param_has_a_type_or_a_default() {
        let typed = ParamSpec::Type(ident("Region"));
        assert_eq!(typed.ty().map(Ident::as_str), Some("Region"));
        assert!(typed.default().is_none());
        let defaulted = ParamSpec::Default(name_expr("overhang"));
        assert!(defaulted.ty().is_none());
        assert!(defaulted.default().is_some());
        let both = ParamSpec::TypeAndDefault {
            ty: ident("JointKind"),
            default: name_expr("dado"),
        };
        assert!(both.ty().is_some() && both.default().is_some());
    }

    #[test]
    fn vocabularies_round_trip_through_their_words() {
        for base in BaseKind::ALL {
            assert_eq!(BaseKind::from_word(base.word()), Some(base));
        }
        for ty in TypeName::ALL {
            assert_eq!(TypeName::from_word(ty.word()), Some(ty));
        }
        for operation in Operation::all() {
            assert_eq!(Operation::from_word(operation.word()), Some(operation));
        }
        for tag in ItemTag::ALL {
            assert_eq!(ItemTag::from_keyword(tag.keyword()), Some(tag));
            assert!(is_keyword(tag.keyword()));
        }
        for unit in Unit::ALL {
            assert_eq!(Unit::from_suffix(unit.suffix()), Some(unit));
        }
        assert_eq!(Operation::all().count(), 11);
        assert_eq!(BaseKind::from_word("knob"), None);
        assert_eq!(TypeName::from_word("length"), None);
        assert_eq!(Operation::from_word("turning"), Some(Operation::Turning));
        assert_eq!(CutOperation::from_word("turning"), None);
        assert_eq!(Unit::from_suffix("inch"), None);
    }

    #[test]
    fn kind_types_follow_their_base() {
        assert_eq!(
            TypeName::of_kind(BaseKind::Slide),
            Some(TypeName::SlideKind)
        );
        assert_eq!(TypeName::of_kind(BaseKind::Tool), None);
        assert!(BaseKind::Hinge.is_mounted_hardware());
        assert!(!BaseKind::Fastener.is_mounted_hardware());
        assert!(CutOperation::Spigot.is_axis_aligned());
        assert!(!CutOperation::Outline.is_axis_aligned());
    }

    #[test]
    fn operators_know_their_precedence() {
        assert!(BinaryOp::Mul.precedence() > BinaryOp::Add.precedence());
        assert!(BinaryOp::Add.precedence() > BinaryOp::Le.precedence());
        assert!(BinaryOp::And.precedence() > BinaryOp::Or.precedence());
        assert!(BinaryOp::Ge.is_comparison());
        assert!(!BinaryOp::Sub.is_comparison());
        assert_eq!(BinaryOp::ALL.map(BinaryOp::symbol).len(), 12);
        assert_eq!(UnaryOp::Not.symbol(), "!");
    }

    #[test]
    fn a_postfix_chain_is_a_path() {
        // drawer[top].front
        let indexed = Expr {
            kind: ExprKind::Index {
                base: Box::new(name_expr("drawer")),
                index: Index::At(Box::new(name_expr("top"))),
            },
            span: Span::new(0, 11),
        };
        let member = Expr {
            kind: ExprKind::Member {
                base: Box::new(indexed),
                member: Name::new("front", Span::default()).unwrap(),
            },
            span: Span::new(0, 17),
        };
        let path = member.as_path().unwrap();
        assert_eq!(path.root.as_str(), "drawer");
        assert_eq!(path.segments.len(), 2);
        assert_eq!(path.span, Span::new(0, 17));

        let call = Expr {
            kind: ExprKind::Call {
                callee: Box::new(name_expr("mid")),
                args: Vec::new(),
            },
            span: Span::default(),
        };
        assert!(call.as_path().is_none());
    }

    #[test]
    fn bodies_keep_their_order_and_find_fields() {
        let field = |key: &str| Field {
            key: Name::new(key, Span::default()).unwrap(),
            value: Expr {
                kind: ExprKind::Fit,
                span: Span::default(),
            },
            span: Span::default(),
        };
        let place = PlaceBlock {
            terms: vec![Call {
                head: ident("ground"),
                args: Vec::new(),
                span: Span::default(),
            }],
            span: Span::default(),
        };
        let body = PartBody {
            entries: vec![
                PartEntry::Field(field("length")),
                PartEntry::Place(place),
                PartEntry::Field(field("width")),
            ],
            span: Span::default(),
        };
        assert_eq!(
            body.fields()
                .map(|field| field.key.as_str())
                .collect::<Vec<_>>(),
            ["length", "width"]
        );
        assert!(body.field("width").is_some());
        assert!(body.field("grain").is_none());
        assert_eq!(body.place().map(|place| place.terms.len()), Some(1));

        let block = FieldBlock {
            fields: vec![field("units")],
            span: Span::default(),
        };
        assert!(block.field("units").is_some());
        assert!(block.field("place").is_none());
    }

    #[test]
    fn items_know_their_keyword_tag_and_name() {
        let part = DesignItem::Part(Part {
            name: ident("side_r"),
            array: None,
            def: PartDef::Mirror(Mirror {
                source: name_expr("side_l"),
                about: name_expr("plane"),
                span: Span::default(),
            }),
        });
        assert_eq!(part.tag(), ItemTag::Part);
        let item = ItemKind::Design(part);
        assert_eq!(item.keyword(), "part");
        assert_eq!(item.tag(), Some(ItemTag::Part));
        assert_eq!(item.name().map(Ident::as_str), Some("side_r"));

        let library = ItemKind::Library(Library {
            title: StringLit::text("Casework", Span::default()),
            body: None,
        });
        assert_eq!(library.keyword(), "library");
        assert_eq!(library.tag(), None);
        assert!(library.name().is_none());
    }
}
