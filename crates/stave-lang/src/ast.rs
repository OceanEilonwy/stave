//! Syntax tree for `.stave` files.
//!
//! This is a faithful, unresolved representation of the source: names are
//! still strings and expressions are not yet type checked. Name resolution,
//! unit checking and placement lowering happen in later passes.
//!
//! Every node carries a [`Span`] (byte offsets into the source) so that
//! errors can point at the exact text, which matters both for people and for
//! feeding errors back to an AI.

use std::num::NonZeroU64;

// ---------------------------------------------------------------------------
// Spans and identifiers
// ---------------------------------------------------------------------------

/// Byte range `start..end` in the source file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn new(start: u32, end: u32) -> Self {
        debug_assert!(start <= end);
        Self { start, end }
    }

    /// Smallest span covering both `self` and `other`.
    pub fn to(self, other: Span) -> Span {
        Span::new(self.start.min(other.start), self.end.max(other.end))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Ident {
    pub name: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StringLit {
    /// Unescaped value.
    pub value: String,
    pub span: Span,
}

// ---------------------------------------------------------------------------
// Files
// ---------------------------------------------------------------------------

/// One `.stave` file: `stave 1`, its imports, then items.
///
/// A module is every file in one folder; files in a module share their item
/// names. Imports apply only to the file they are written in.
#[derive(Debug, Clone, PartialEq)]
pub struct File {
    pub header: Header,
    pub imports: Vec<Import>,
    pub items: Vec<Item>,
}

/// `import "std:casework"` or
/// `import "github:some-maker/stave-catalog/slides@v2.1.0" as slides`
///
/// Imports a folder (a module). Its items are used qualified by the module
/// name: the folder's last segment, or `alias`.
#[derive(Debug, Clone, PartialEq)]
pub struct Import {
    /// Relative path, `std:...`, `github:owner/repo/path@ref` or
    /// `https://...#sha256=...`. Resolved by the module loader, not the parser.
    pub source: StringLit,
    pub alias: Option<Ident>,
    pub span: Span,
}

/// A patch file: `stave 1` followed by patches. This is what an AI or the
/// suggestion engine produces.
#[derive(Debug, Clone, PartialEq)]
pub struct PatchFile {
    pub header: Header,
    pub patches: Vec<Patch>,
}

/// `stave <version>`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub version: u32,
    pub span: Span,
}

// ---------------------------------------------------------------------------
// Items
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    /// `///` doc comments, one entry per line, without the `///` prefix.
    /// Kept so notes reach the build manual.
    pub docs: Vec<String>,
    /// `Some` with the keyword's span for `private` items, which other
    /// modules can't see.
    pub private: Option<Span>,
    pub kind: ItemKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ItemKind {
    Project(Project),
    Library(Library),
    Kind(Kind),
    Recipe(Recipe),
    Param(Param),
    Let(Let),
    Region(Region),
    Stock(Stock),
    Part(Part),
    Joint(Connection),
    Hardware(Connection),
    Assembly(Assembly),
    Profile(Profile),
    Component(Component),
    Scene(Scene),
    Require(Require),
}

impl ItemKind {
    pub fn tag(&self) -> ItemTag {
        match self {
            ItemKind::Project(_) => ItemTag::Project,
            ItemKind::Library(_) => ItemTag::Library,
            ItemKind::Kind(_) => ItemTag::Kind,
            ItemKind::Recipe(_) => ItemTag::Recipe,
            ItemKind::Param(_) => ItemTag::Param,
            ItemKind::Let(_) => ItemTag::Let,
            ItemKind::Region(_) => ItemTag::Region,
            ItemKind::Stock(_) => ItemTag::Stock,
            ItemKind::Part(_) => ItemTag::Part,
            ItemKind::Joint(_) => ItemTag::Joint,
            ItemKind::Hardware(_) => ItemTag::Hardware,
            ItemKind::Assembly(_) => ItemTag::Assembly,
            ItemKind::Profile(_) => ItemTag::Profile,
            ItemKind::Component(_) => ItemTag::Component,
            ItemKind::Scene(_) => ItemTag::Scene,
            ItemKind::Require(_) => ItemTag::Require,
        }
    }

    /// The item's name. Every item has one except the project and library
    /// headers, which have a title instead.
    pub fn name(&self) -> Option<&Ident> {
        match self {
            ItemKind::Project(_) | ItemKind::Library(_) => None,
            ItemKind::Kind(k) => Some(&k.name),
            ItemKind::Recipe(r) => Some(&r.name),
            ItemKind::Param(p) => Some(&p.name),
            ItemKind::Let(l) => Some(&l.name),
            ItemKind::Region(r) => Some(&r.name),
            ItemKind::Stock(s) => Some(&s.name),
            ItemKind::Part(p) => Some(&p.name),
            ItemKind::Joint(c) | ItemKind::Hardware(c) => Some(&c.name),
            ItemKind::Assembly(a) => Some(&a.name),
            ItemKind::Profile(p) => Some(&p.name),
            ItemKind::Component(c) => Some(&c.name),
            ItemKind::Scene(s) => Some(&s.name),
            ItemKind::Require(r) => Some(&r.name),
        }
    }
}

/// Item keyword, used by `remove` in patches and in diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ItemTag {
    /// Not removable by a patch; listed so every item kind has a tag.
    Project,
    /// Not removable by a patch; listed so every item kind has a tag.
    Library,
    Kind,
    Recipe,
    Param,
    Let,
    Region,
    Stock,
    Part,
    Joint,
    Hardware,
    Assembly,
    Profile,
    Component,
    Scene,
    Require,
}

impl ItemTag {
    pub fn keyword(self) -> &'static str {
        match self {
            ItemTag::Project => "project",
            ItemTag::Library => "library",
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
}

/// `project "Title" { goal: "...", units: mm, resolution: 0.5mm }`
#[derive(Debug, Clone, PartialEq)]
pub struct Project {
    pub title: StringLit,
    pub body: Body,
}

/// `library "Shop hardware"`: marks the folder as a library module.
#[derive(Debug, Clone, PartialEq)]
pub struct Library {
    pub title: StringLit,
    pub body: Option<Body>,
}

/// A joint or hardware kind, written in `.stave` (usually in a library).
///
/// ```text
/// kind side_mount_500 : slide {
///   length:   500mm
///   envelope: box(length: 500mm, height: 45mm, thickness: 12.7mm)
///   model:    model("./models/side_mount_500.glb", align: bounds)
/// }
///
/// kind ari_tsugi : joint {
///   params { depth: Length = part.width / 2 }
///   insert: [slide(part, along: width)]
///   adds:   length(part, depth)
///   cut end(part, facing: into) { dovetail_tail(neck: neck, depth: depth, angle: angle) }
///   steps { "Drop {part} into {into} from above" }
/// }
/// ```
///
/// A hardware model is for display only; checks use `envelope`.
#[derive(Debug, Clone, PartialEq)]
pub struct Kind {
    pub name: Ident,
    /// Base kind: `joint`, `slide`, `hinge`, `knob`, `pull`, `fastener`, ...
    pub base: Ident,
    pub body: KindBody,
}

/// The `{ ... }` of a `kind`.
#[derive(Debug, Clone, PartialEq)]
pub struct KindBody {
    /// `insert:`, `adds:`, `envelope:`, `model:`, ...
    pub fields: Vec<Field>,
    /// `params { name: Type = default }`
    pub params: Vec<ParamDecl>,
    /// `roles { part: Part, into: [Part, Part] }`. Empty means the default
    /// roles `part: Parts` and `into: Parts`.
    pub roles: Vec<RoleDecl>,
    pub cuts: Vec<Cut>,
    /// Parts the joint creates: keys, wedges, pins.
    pub makes: Vec<Part>,
    pub requires: Vec<Require>,
    /// `steps { "..." }`: build-manual text with `{name}` placeholders.
    pub steps: Vec<StringLit>,
    pub span: Span,
}

/// `part: Part`, `into: Parts`, `into: [Part, Part]`
#[derive(Debug, Clone, PartialEq)]
pub struct RoleDecl {
    pub name: Ident,
    pub ty: RoleType,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RoleType {
    /// `Part` (exactly one) or `Parts` (one or more).
    Named(Ident),
    /// `[Part, Part]`: exactly this many, in order.
    Fixed(Vec<Ident>),
}

/// `cut end(part, facing: into) { tenon(...) }`
#[derive(Debug, Clone, PartialEq)]
pub struct Cut {
    /// Face selector: `end(...)`, `side(...)` or `crossing(...)`.
    pub face: Expr,
    /// Cut operations, one per line: `notch(depth: depth)`, `outline(...)`.
    pub ops: Vec<Expr>,
    pub span: Span,
}

/// How to make one cut operation with a set of tools. Work cards are built
/// from these.
///
/// ```text
/// recipe notch_by_hand : notch {
///   tools: [marking_knife, square, marking_gauge, tenon_saw, chisel, router_plane]
///   mark { knife(walls); gauge(depth, on: open_edges); hatch(waste) }
///   cut  { saw(walls, to: depth, keep: outside); chop(waste, bevel: down) }
///   check "the mating part pushes in by hand"
/// }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Recipe {
    pub name: Ident,
    /// The cut operation this recipe makes: `notch`, `drill`, `tenon`, ...
    pub op: Ident,
    /// `tools: [...]`, `prefer: ...`
    pub fields: Vec<Field>,
    /// Marking-out steps, one per line: `knife(walls)`, `gauge(depth, on: open_edges)`.
    pub mark: Vec<Expr>,
    /// Cutting steps, one per line: `saw(...)`, `chop(...)`, `rout(...)`.
    pub cut: Vec<Expr>,
    /// What tells you the cut is right.
    pub check: Option<StringLit>,
    pub span: Span,
}

/// `param cabinet_width = 900mm within 600mm..1200mm`
/// `param slide_length = 500mm within [350mm, 400mm, 450mm, 500mm]`
/// `param reveal = 1.5mm locked`
///
/// Params are the user-facing knobs. The domain lets the checker verify the
/// design for every allowed value; `locked` stops smart edits from changing it.
#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: Ident,
    pub value: Expr,
    pub within: Option<Domain>,
    /// `Some` with the keyword's span when the param is `locked`.
    pub locked: Option<Span>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Domain {
    /// `lo..hi`, inclusive on both ends.
    Range { lo: Expr, hi: Expr, span: Span },
    /// `[a, b, c]`: exactly one of these values.
    OneOf { values: Vec<Expr>, span: Span },
}

/// `let foot_tenon = 19mm`
#[derive(Debug, Clone, PartialEq)]
pub struct Let {
    pub name: Ident,
    pub value: Expr,
}

/// `region bay[drawers, doors] = split(front_opening, up, at: divider)`
#[derive(Debug, Clone, PartialEq)]
pub struct Region {
    pub name: Ident,
    pub array: Option<ArraySpec>,
    pub value: Expr,
}

/// `stock oak_19 { species: white_oak, form: solid, thickness: 19mm }`
#[derive(Debug, Clone, PartialEq)]
pub struct Stock {
    pub name: Ident,
    pub body: Body,
}

/// `part shelf : oak_19 { ... }` or `part side_r = mirror(side_l, about: ...)`
#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    pub name: Ident,
    /// `None` for a single part.
    pub array: Option<ArraySpec>,
    pub def: PartDef,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PartDef {
    /// Cut from stock: `: oak_19 { length: ..., place { ... } }`
    FromStock { stock: Path, body: Body },
    /// Derived from another part: `= mirror(side_l, about: mid(base, across))`.
    Derived(Expr),
}

/// How an array of parts, regions, assemblies or connections is declared.
#[derive(Debug, Clone, PartialEq)]
pub enum ArraySpec {
    /// `shelf[3]`, `shelf[shelf_count]`. Elements are `shelf[0]`, `shelf[1]`, ...
    Count(Expr),
    /// `drawer[bottom, top]`. Always two or more labels, in declaration order.
    /// Elements are `drawer[bottom]` or, equivalently, `drawer[0]`.
    Labels { labels: Vec<Ident>, span: Span },
}

impl ArraySpec {
    /// Position of a label, if this array is labelled and has it.
    pub fn label_index(&self, name: &str) -> Option<usize> {
        match self {
            ArraySpec::Count(_) => None,
            ArraySpec::Labels { labels, .. } => labels.iter().position(|l| l.name == name),
        }
    }
}

/// A joint or a piece of hardware. The name comes first; the kind is a call
/// whose arguments name the parts by role.
///
/// ```text
/// joint back_rabbets : rabbet(back, into: [side_l, side_r]) { depth: 9mm }
/// hardware slides[left, right] : side_mount_slide(side[*], in: opening)
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Connection {
    pub name: Ident,
    pub array: Option<ArraySpec>,
    /// Builtin kind (`dado`, `euro_hinge`, ...) or a component param holding
    /// a kind. May be a path, e.g. a kind defined in a library.
    pub kind: Path,
    /// The first positional argument is the inserted or mounted part; named
    /// arguments (`into:`, `on:`, `in:`) give the other roles and kind
    /// settings.
    pub args: Vec<Arg>,
    pub body: Option<Body>,
}

/// `assembly drawer[bottom, top] : Drawer { ... }` or
/// `assembly door_r = mirror(door_l, about: mid(base, across))`
#[derive(Debug, Clone, PartialEq)]
pub struct Assembly {
    pub name: Ident,
    pub array: Option<ArraySpec>,
    pub def: AssemblyDef,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AssemblyDef {
    /// An instance of a component; the body's fields are its arguments.
    /// The component may come from another module: `casework.Drawer`.
    Instance { component: Path, body: Body },
    /// Derived from another assembly.
    Derived(Expr),
}

/// 2D profile for turned shapes.
///
/// Stations are measurements along the part (`at 16mm: 70mm`), not world
/// coordinates. The `measure:` field says whether values are diameters or
/// radii.
#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    pub name: Ident,
    pub entries: Vec<ProfileEntry>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ProfileEntry {
    /// `at <station>: <value>`
    Station { at: Expr, value: Expr, span: Span },
    /// `measure: diameter`, `smooth: catmull_rom`
    Field(Field),
}

/// `component Drawer(opening: Region, slide: HardwareKind, box_height: Length = 180mm) { ... }`
#[derive(Debug, Clone, PartialEq)]
pub struct Component {
    pub name: Ident,
    pub params: Vec<ParamDecl>,
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParamDecl {
    pub name: Ident,
    /// A plain type name: `Length`, `Part`, `Region`, `HardwareKind`, ...
    pub ty: Ident,
    pub default: Option<Expr>,
    pub span: Span,
}

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
    pub name: Ident,
    pub title: Option<StringLit>,
    pub entries: Vec<SceneEntry>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SceneEntry {
    /// `pose drawer[top] = open`, `pose door_l = 90deg`
    Pose { target: Path, value: Expr, span: Span },
    /// `show: drawer[top]`, `explode: true`
    Field(Field),
}

/// `require hanger_room : cabinet_depth - back.thickness >= 550mm "room for coat hangers"`
#[derive(Debug, Clone, PartialEq)]
pub struct Require {
    pub name: Ident,
    pub condition: Expr,
    pub message: StringLit,
}

// ---------------------------------------------------------------------------
// Bodies, fields and placement
// ---------------------------------------------------------------------------

/// The `{ ... }` after a declaration header.
#[derive(Debug, Clone, PartialEq)]
pub struct Body {
    pub fields: Vec<Field>,
    /// `place { ... }`. Only parts, assembly instances and hardware may have
    /// one; the checker rejects it elsewhere.
    pub place: Option<PlaceBlock>,
    pub span: Span,
}

impl Body {
    pub fn get(&self, key: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.key.name == key)
    }
}

/// `key: value`. Always exactly one expression; lists use `[ ]`.
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub key: Ident,
    pub value: Expr,
    pub span: Span,
}

/// `place { on(base.top) \n flush(base.left) \n ... }`
///
/// One term per line. Because each axis is set by exactly one term, a patch
/// can address a single term by axis: `set side_l.place.deep = ...`.
#[derive(Debug, Clone, PartialEq)]
pub struct PlaceBlock {
    pub terms: Vec<Expr>,
    pub span: Span,
}

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

/// `side_l`, `base.top`, `shelf[*]`, `door_l.stile[hinge]`
///
/// Used where the grammar requires a reference (stock of a part, `set` and
/// `pose` targets, connection kinds). Inside expressions the same text is a
/// postfix chain of [`ExprKind`]; see [`Expr::as_path`].
#[derive(Debug, Clone, PartialEq)]
pub struct Path {
    pub root: Ident,
    pub segments: Vec<PathSegment>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PathSegment {
    Field(Ident),
    Index(Index),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Index {
    /// `[*]`: every element, or inside an array declaration, the matching
    /// element (elementwise).
    All(Span),
    /// `[0]`, `[i + 1]`, or a label such as `[top]`.
    ///
    /// The parser can't tell a label from a name in scope, so a label is an
    /// `ExprKind::Name` here. Resolution treats a bare name that matches a
    /// label of the indexed array as that label, and rejects a `param` or
    /// `let` that shares a name with a label.
    At(Box<Expr>),
}

// ---------------------------------------------------------------------------
// Expressions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    /// `3/4in`, `19mm`, `110deg`
    Quantity(Quantity),
    /// Unitless: `3`, `0.5`
    Number(Number),
    String(String),
    Bool(bool),
    /// `fit`: the size is decided by a housing, joints, placement or a profile.
    Fit,
    /// `cabinet_width`, `evenly`, `ground`
    Name(Ident),
    /// `base.top`
    Member { base: Box<Expr>, member: Ident },
    /// `shelf[0]`, `drawer[top]`, `side[*]`
    Index { base: Box<Expr>, index: Index },
    /// `between(side_l, side_r)`, `spaced(evenly, from: a, to: b)`
    Call { callee: Box<Expr>, args: Vec<Arg> },
    Unary { op: UnaryOp, operand: Box<Expr> },
    Binary { op: BinaryOp, lhs: Box<Expr>, rhs: Box<Expr> },
    /// `[a, b, c]`
    List(Vec<Expr>),
    /// `( expr )`, kept so the formatter can preserve intent.
    Paren(Box<Expr>),
}

impl Expr {
    /// Convert a postfix chain of names, members and indexes into a [`Path`].
    /// Returns `None` if the expression contains anything else.
    pub fn as_path(&self) -> Option<Path> {
        fn walk(e: &Expr, segments: &mut Vec<PathSegment>) -> Option<Ident> {
            match &e.kind {
                ExprKind::Name(id) => Some(id.clone()),
                ExprKind::Member { base, member } => {
                    let root = walk(base, segments)?;
                    segments.push(PathSegment::Field(member.clone()));
                    Some(root)
                }
                ExprKind::Index { base, index } => {
                    let root = walk(base, segments)?;
                    segments.push(PathSegment::Index(index.clone()));
                    Some(root)
                }
                _ => None,
            }
        }
        let mut segments = Vec::new();
        let root = walk(self, &mut segments)?;
        Some(Path { root, segments, span: self.span })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Arg {
    /// `Some` for named arguments such as `into: side_l`.
    pub name: Option<Ident>,
    pub value: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnaryOp {
    Neg,
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BinaryOp {
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Add,
    Sub,
    Mul,
    Div,
}

impl BinaryOp {
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

/// A literal with a unit, exactly as written. Conversion to an exact
/// [`crate::units::Length`] or [`crate::units::Angle`] happens in
/// [`crate::units`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Quantity {
    pub magnitude: Magnitude,
    pub unit: Unit,
}

/// Magnitude of a quantity literal. Fractions only exist with a unit:
/// a bare `3/4` is the division `3 / 4`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Magnitude {
    Integer(u64),
    Decimal(Decimal),
    /// `3/4` (whole = 0) or `1-1/2` (whole = 1)
    Fraction(Fraction),
}

/// Unitless number literal. Evaluated as an exact [`crate::units::Rational`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Number {
    Integer(u64),
    Decimal(Decimal),
}

/// `digits / 10^scale`, so `18.25` is `{ digits: 1825, scale: 2 }`.
/// Stored exactly; never a float.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Decimal {
    pub digits: u64,
    pub scale: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Fraction {
    pub whole: u64,
    pub numerator: u64,
    pub denominator: NonZeroU64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Unit {
    Millimeter,
    Centimeter,
    Meter,
    Inch,
    Foot,
    Degree,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dimension {
    Length,
    Angle,
}

impl Unit {
    pub const ALL: [Unit; 6] = [
        Unit::Millimeter,
        Unit::Centimeter,
        Unit::Meter,
        Unit::Inch,
        Unit::Foot,
        Unit::Degree,
    ];

    pub fn from_suffix(suffix: &str) -> Option<Unit> {
        Unit::ALL.into_iter().find(|u| u.suffix() == suffix)
    }

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

    pub fn dimension(self) -> Dimension {
        match self {
            Unit::Degree => Dimension::Angle,
            _ => Dimension::Length,
        }
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
    pub ops: Vec<PatchOp>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PatchOp {
    pub kind: PatchOpKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PatchOpKind {
    /// `add part shoe_shelf : oak_19 { ... }`, optionally `in "carcass.stave"`
    /// to choose the file within the module.
    Add { item: Box<Item>, file: Option<StringLit> },
    /// `set cabinet_width = 950mm`, `set side_l.place.deep = flush(base.back)`
    Set { target: Path, value: Expr },
    /// `remove joint shoe_shelf_dados`
    Remove { tag: ItemTag, name: Ident },
}
