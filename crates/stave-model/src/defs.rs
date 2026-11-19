//! Resolved definitions: every item of every module with its names
//! resolved and its expressions typed, before anything is instantiated.
//!
//! Definitions are generic: a component's parts are defined once here and
//! instantiated once per assembly by [`crate::instance`]. Expressions refer
//! to items through [`Symbol`]s; which concrete item a symbol means is
//! decided by the instance it is evaluated in.

use crate::nonempty::NonEmpty;
use stave_lang::FileId;
use stave_lang::ast::BaseKind;
use stave_lang::ast::{BinaryOp, Span, UnaryOp};
use stave_lang::placement::{Axis, Direction, Face, PartAxis};
use stave_lang::units::{ExactAngle, ExactLength, Rational};
use stave_modules::ModuleId;
use stave_schema::functions::Builtin;
use stave_schema::operations::Selector;
use stave_schema::placement::AssemblyStepKind;
use stave_schema::recipes::{CuttingStep, MarkingStep, NeedKind};
use stave_schema::species::Species;
use stave_schema::words::{Grain, Measure, MotionWord, OutputUnit, StockForm};
use stave_schema::{CutOperation, Operation};

use crate::ids::{
    AssemblyDefId, ComponentId, HardwareDefId, JointDefId, KindId, LetDefId, MadeIndex, ParamDefId,
    PartDefId, ProfileId, RecipeId, RegionDefId, RequireDefId, RequireIndex, SceneDefId, StockId,
};
use crate::types::Ty;

// ---------------------------------------------------------------------------
// Symbols and expressions
// ---------------------------------------------------------------------------

/// A declaration in a module or a component body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DefId {
    /// `param`
    Param(ParamDefId),
    /// `let`
    Let(LetDefId),
    /// `region`
    Region(RegionDefId),
    /// `part`
    Part(PartDefId),
    /// `joint`
    Joint(JointDefId),
    /// `hardware`
    Hardware(HardwareDefId),
    /// `assembly`
    Assembly(AssemblyDefId),
    /// `require`
    Require(RequireDefId),
    /// `stock`
    Stock(StockId),
    /// `profile`
    Profile(ProfileId),
    /// `kind`
    Kind(KindId),
    /// `recipe`
    Recipe(RecipeId),
    /// `component`
    Component(ComponentId),
    /// `scene`
    Scene(SceneDefId),
}

/// What a resolved name refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Symbol {
    /// An item of a module or a component.
    Def(DefId),
    /// A role of the kind being defined; bound per pair.
    Role(KindId, RoleIndex),
    /// A param of the kind being defined.
    KindParam(KindId, usize),
    /// A part the kind makes.
    MadePart(KindId, MadeIndex),
    /// `mount` in a slide, hinge or pull kind: the face of its piece the
    /// hardware is mounted on, as an anchor; bound per piece of hardware.
    Mount(KindId),
}

/// Where a declaration lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Scope {
    /// At the top level of a module.
    Module(ModuleId),
    /// Inside a component body.
    Component(ComponentId),
}

/// A bare enum word, resolved for the type its position expects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Word {
    /// `up`, `across`, `deep`
    Axis(Axis),
    /// `up`, `down`, `left`, `right`, `front`, `back`
    Direction(Direction),
    /// `top`, `bottom`, `left`, `right`, `front`, `back`
    Face(Face),
    /// `length`, `width`, `thickness`
    PartAxis(PartAxis),
    /// `mm`, `cm`, `m`, `in`, `ft`
    OutputUnit(OutputUnit),
    /// A species.
    Species(Species),
    /// `solid`, `plywood`, `mdf`, `dowel`
    StockForm(StockForm),
    /// `diameter`, `radius`
    Measure(Measure),
    /// `open`, `closed`
    Motion(MotionWord),
    /// `length`, `width` (grain)
    Grain(Grain),
    /// `through`
    Through,
    /// `evenly`
    Evenly,
    /// `ends`
    Ends,
}

/// A readable member, resolved for the type on the left of the `.`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Member {
    /// `part.length`, `part.width`, `part.thickness`: drawn sizes.
    Size(PartAxis),
    /// `part.diameter`: round parts only.
    Diameter,
    /// `part.stock`
    PartStock,
    /// A face of a part or region: `base.top`.
    Face(Face),
    /// `region.across`, `region.up`, `region.deep`
    RegionSize(Axis),
    /// A field of a stock: `oak_19.thickness`.
    StockField(StockField),
    /// `profile.length`: the last station.
    ProfileLength,
    /// An item of an assembly's component: `drawer[top].front`.
    AssemblyItem(DefId),
    /// A param or field of a kind value, by name: `slide.clearance`.
    KindEntry(String),
    /// `contact(a, b).length`, `contact(a, b).width`: a size of the patch.
    PatchSize(stave_schema::functions::PatchSize),
}

/// A readable field of a stock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StockField {
    /// `species`
    Species,
    /// `form`
    Form,
    /// `thickness`
    Thickness,
    /// `diameter`
    Diameter,
    /// `width`
    Width,
    /// `max_width`
    MaxWidth,
    /// `max_length`
    MaxLength,
}

impl StockField {
    /// The field for a name, if it is one.
    pub fn from_name(name: &str) -> Option<StockField> {
        Some(match name {
            "species" => StockField::Species,
            "form" => StockField::Form,
            "thickness" => StockField::Thickness,
            "diameter" => StockField::Diameter,
            "width" => StockField::Width,
            "max_width" => StockField::MaxWidth,
            "max_length" => StockField::MaxLength,
            _ => return None,
        })
    }
}

/// What is written between `[` and `]`, resolved.
#[derive(Debug, Clone, PartialEq)]
pub enum Index {
    /// `[*]`
    All,
    /// `[same]`
    Same,
    /// A label of a labelled array, by position.
    Label(usize),
    /// A number, for a counted array.
    Number(Box<RExpr>),
}

/// A named argument or setting.
#[derive(Debug, Clone, PartialEq)]
pub struct Setting {
    /// The name as written.
    pub name: String,
    /// Where the name was written.
    pub name_span: Span,
    /// The value.
    pub value: RExpr,
}

/// Text with `{name}` values inserted.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Text {
    /// The pieces in order.
    pub parts: Vec<TextPart>,
}

/// One piece of a [`Text`].
#[derive(Debug, Clone, PartialEq)]
pub enum TextPart {
    /// Literal text.
    Literal(String),
    /// A value to format.
    Value(RExpr),
}

impl Text {
    /// The text without its interpolations, for strings that have none.
    pub fn literal(&self) -> String {
        self.parts
            .iter()
            .map(|part| match part {
                TextPart::Literal(text) => text.as_str(),
                TextPart::Value(_) => "",
            })
            .collect()
    }
}

/// A resolved, typed expression.
#[derive(Debug, Clone, PartialEq)]
pub struct RExpr {
    /// What it is.
    pub kind: RExprKind,
    /// Its static type.
    pub ty: Ty,
    /// Where it was written.
    pub span: Span,
}

/// The forms of a resolved expression.
#[derive(Debug, Clone, PartialEq)]
pub enum RExprKind {
    /// A length literal.
    Length(ExactLength),
    /// An angle literal.
    Angle(ExactAngle),
    /// A unitless literal; [`RExpr::ty`] says count or ratio.
    Number(Rational),
    /// `true` or `false`.
    Bool(bool),
    /// A string.
    Text(Text),
    /// `fit`.
    Fit,
    /// A bare enum word.
    Word(Word),
    /// A resolved name.
    Symbol(Symbol),
    /// An array's own name inside its declaration: the element being
    /// declared.
    SelfElement(DefId),
    /// `base.member`
    Member {
        /// Left of the `.`.
        base: Box<RExpr>,
        /// The member.
        member: Member,
    },
    /// `base[index]`
    Index {
        /// What is indexed.
        base: Box<RExpr>,
        /// The index.
        index: Index,
    },
    /// A built-in function call.
    Builtin {
        /// The function.
        function: Builtin,
        /// Its arguments, by parameter name (positional ones included).
        args: Vec<Setting>,
    },
    /// A kind value with settings: `side_mount_slide(length: 450mm)`.
    KindValue {
        /// The kind, or a kind value to add settings to.
        kind: Box<RExpr>,
        /// The settings.
        settings: Vec<Setting>,
    },
    /// `-x`, `!x`
    Unary {
        /// The operator.
        op: UnaryOp,
        /// Its operand.
        operand: Box<RExpr>,
    },
    /// `a + b`, `a <= b`, ...
    Binary {
        /// The operator.
        op: BinaryOp,
        /// Left operand.
        lhs: Box<RExpr>,
        /// Right operand.
        rhs: Box<RExpr>,
    },
    /// `[a, b]`; lists inside lists are flattened when evaluated.
    List(Vec<RExpr>),
    /// `wall(cut)`: the least wood round one of the enclosing kind's named
    /// cuts. Only one whole side of a comparison that is a kind's
    /// `require` condition; measured once the cuts are made.
    Wall(CutName),
    /// Something that could not be resolved; already reported.
    Error,
}

impl RExpr {
    /// An expression that failed to resolve, already reported.
    pub fn error(span: Span) -> RExpr {
        RExpr {
            kind: RExprKind::Error,
            ty: Ty::Unknown,
            span,
        }
    }
}

// ---------------------------------------------------------------------------
// Declarations
// ---------------------------------------------------------------------------

/// What every declaration records about where it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decl {
    /// The name.
    pub name: String,
    /// Where the name is written.
    pub name_span: Span,
    /// The file it is in.
    pub file: FileId,
    /// The whole declaration.
    pub span: Span,
    /// The module or component it is declared in.
    pub scope: Scope,
    /// Its doc comments.
    pub docs: Vec<String>,
    /// `true` for `private` items.
    pub private: bool,
}

/// How an array is declared.
#[derive(Debug, Clone, PartialEq)]
pub enum ArrayDef {
    /// `[bottom, top]`
    Labels(Vec<(String, Span)>),
    /// `[3]`, `[count]`
    Count(RExpr),
}

/// A `param`, in a module, a component or a kind.
#[derive(Debug, Clone, PartialEq)]
pub struct ParamDef {
    /// Where it is.
    pub decl: Decl,
    /// Its type: declared, or the default's.
    pub ty: Ty,
    /// The default, when one is written.
    pub default: Option<RExpr>,
    /// The allowed values.
    pub domain: Option<Domain>,
    /// Its kind: plain, `locked` (smart edits must not change it) or
    /// `solved` (the solver may solve for it).
    pub kind: stave_lang::ast::ParamKind,
}

/// The values a param may take.
#[derive(Debug, Clone, PartialEq)]
pub enum Domain {
    /// `lo..hi`, both ends included.
    Range {
        /// Smallest allowed value.
        low: RExpr,
        /// Largest allowed value.
        high: RExpr,
        /// The whole range.
        span: Span,
    },
    /// `[a, b, c]`
    OneOf {
        /// The allowed values.
        values: Vec<RExpr>,
        /// The whole list.
        span: Span,
    },
}

/// A `let`.
#[derive(Debug, Clone, PartialEq)]
pub struct LetDef {
    /// Where it is.
    pub decl: Decl,
    /// The value.
    pub value: RExpr,
}

/// A `region`.
#[derive(Debug, Clone, PartialEq)]
pub struct RegionDef {
    /// Where it is.
    pub decl: Decl,
    /// `Some` for an array.
    pub array: Option<ArrayDef>,
    /// `opening(...)`, `split(...)`, `cover(...)` or another region.
    pub value: RExpr,
}

/// A `stock`.
#[derive(Debug, Clone, PartialEq)]
pub struct StockDef {
    /// Where it is.
    pub decl: Decl,
    /// `species:`
    pub species: Species,
    /// `form:`
    pub form: StockForm,
    /// `thickness:`; required except for dowels.
    pub thickness: Option<RExpr>,
    /// `diameter:`; dowels only.
    pub diameter: Option<RExpr>,
    /// `width:`
    pub width: Option<RExpr>,
    /// `max_width:`
    pub max_width: Option<RExpr>,
    /// `max_length:`
    pub max_length: Option<RExpr>,
}

/// A drawn size: an expression or `fit`.
#[derive(Debug, Clone, PartialEq)]
pub enum Size {
    /// Worked out from the part's joints or profile.
    Fit(Span),
    /// A length expression.
    Expr(RExpr),
}

/// A part's body, for a part cut from stock (also a part a kind makes).
#[derive(Debug, Clone, PartialEq)]
pub struct PartBodyDef {
    /// The stock it is cut from.
    pub stock: RExpr,
    /// `length:`
    pub length: Size,
    /// `width:`; `None` when the stock gives it.
    pub width: Option<Size>,
    /// `thickness:`; `None` means the stock's.
    pub thickness: Option<RExpr>,
    /// `grain:`
    pub grain: Grain,
    /// `shape:`
    pub shape: Option<RExpr>,
    /// `face_side:`
    pub face_side: Option<(Face, Span)>,
    /// `face_edge:`
    pub face_edge: Option<(Face, Span)>,
    /// `note:`
    pub note: Option<Text>,
    /// `boards:`: how its boards are joined (grammar, "Boards").
    pub boards: Option<RExpr>,
    /// `lengths:`: where it is spliced end to end, when it is longer than
    /// its stock (grammar, "Splices").
    pub lengths: Option<RExpr>,
    /// `loose:`: `true` for a part meant to lift out, which nothing needs
    /// to hold (grammar, "Held").
    pub loose: Option<RExpr>,
    /// The `place { }` terms in order.
    pub place: Vec<PlaceTerm>,
    /// The `place { }` block, when written.
    pub place_span: Option<Span>,
    /// The body's braces.
    pub span: Span,
}

/// How a part is defined.
#[derive(Debug, Clone, PartialEq)]
pub enum PartKind {
    /// Cut from stock.
    FromStock(Box<PartBodyDef>),
    /// The mirror image of another part.
    Mirror(MirrorDef),
}

/// `mirror(source, about: plane)`
#[derive(Debug, Clone, PartialEq)]
pub struct MirrorDef {
    /// The part or assembly mirrored.
    pub source: RExpr,
    /// The plane: an anchor.
    pub about: RExpr,
    /// The whole call.
    pub span: Span,
}

/// A `part`.
#[derive(Debug, Clone, PartialEq)]
pub struct PartDef {
    /// Where it is.
    pub decl: Decl,
    /// `Some` for an array.
    pub array: Option<ArrayDef>,
    /// How it is defined.
    pub kind: PartKind,
}

/// One placement term, resolved but not yet evaluated.
#[derive(Debug, Clone, PartialEq)]
pub struct PlaceTerm {
    /// The term.
    pub kind: PlaceTermKind,
    /// The whole term.
    pub span: Span,
}

/// The placement terms.
#[derive(Debug, Clone, PartialEq)]
pub enum PlaceTermKind {
    /// `ground`
    Ground,
    /// `against(face, gap: d)`
    Against {
        /// The face.
        face: RExpr,
        /// The gap, zero when left out.
        gap: Option<RExpr>,
    },
    /// `flush(face, inset: d)`
    Flush {
        /// The face.
        face: RExpr,
        /// The inset, zero when left out.
        inset: Option<RExpr>,
    },
    /// `center(item, axis)`
    CenterOn {
        /// The part or region.
        item: RExpr,
        /// The world axis.
        axis: RExpr,
    },
    /// `center(from: a, to: b)`
    CenterBetween {
        /// One anchor.
        from: RExpr,
        /// The other.
        to: RExpr,
    },
    /// `spaced(spacing, from: a, to: b)`
    Spaced {
        /// The spacing.
        spacing: RExpr,
        /// The first anchor.
        from: RExpr,
        /// The last anchor.
        to: RExpr,
    },
    /// `corners(part, inset: d)`
    Corners {
        /// The part whose corners are used.
        part: RExpr,
        /// The inset, zero when left out.
        inset: Option<RExpr>,
    },
    /// `orient(length: dir, width: dir)`
    Orient {
        /// Where the length runs.
        length: RExpr,
        /// Where the width runs; may be left out for round parts.
        width: Option<RExpr>,
    },
}

/// A kind call in a `joint` or `hardware` declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct KindCallDef {
    /// The kind or kind value.
    pub kind: RExpr,
    /// The positional argument (the first role), if written.
    pub positional: Option<RExpr>,
    /// The named arguments: roles and settings.
    pub named: Vec<Setting>,
    /// The whole call.
    pub span: Span,
}

/// A `joint`.
#[derive(Debug, Clone, PartialEq)]
pub struct JointDef {
    /// Where it is.
    pub decl: Decl,
    /// `Some` for an array.
    pub array: Option<ArrayDef>,
    /// The kind call.
    pub call: KindCallDef,
}

/// A `hardware`.
#[derive(Debug, Clone, PartialEq)]
pub struct HardwareDef {
    /// Where it is.
    pub decl: Decl,
    /// `Some` for an array.
    pub array: Option<ArrayDef>,
    /// The kind call.
    pub call: KindCallDef,
    /// The `place { }` terms.
    pub place: Vec<PlaceTerm>,
    /// The `place { }` block, when written.
    pub place_span: Option<Span>,
}

/// An `assembly`.
#[derive(Debug, Clone, PartialEq)]
pub struct AssemblyDef {
    /// Where it is.
    pub decl: Decl,
    /// `Some` for an array.
    pub array: Option<ArrayDef>,
    /// How it is defined.
    pub kind: AssemblyKind,
}

/// How an assembly is defined.
#[derive(Debug, Clone, PartialEq)]
pub enum AssemblyKind {
    /// An instance of a component.
    Instance {
        /// The component.
        component: ComponentId,
        /// The params given, by the component's param order.
        args: Vec<(ParamDefId, RExpr)>,
    },
    /// The mirror image of another assembly.
    Mirror(MirrorDef),
    /// The component could not be resolved; already reported.
    Invalid,
}

/// A `require`.
#[derive(Debug, Clone, PartialEq)]
pub struct RequireDef {
    /// Where it is.
    pub decl: Decl,
    /// What must hold.
    pub condition: RExpr,
    /// What to say when it doesn't.
    pub message: Text,
}

/// A `profile`.
#[derive(Debug, Clone, PartialEq)]
pub struct ProfileDef {
    /// Where it is.
    pub decl: Decl,
    /// `measure:`
    pub measure: Measure,
    /// `smooth:`
    pub smooth: bool,
    /// The stations in order: distance from the start, value.
    pub stations: Vec<(RExpr, RExpr)>,
}

/// A `component`.
#[derive(Debug, Clone, PartialEq)]
pub struct ComponentDef {
    /// Where it is.
    pub decl: Decl,
    /// Its items in source order.
    pub items: Vec<DefId>,
    /// Its params in source order.
    pub params: Vec<ParamDefId>,
}

/// A `scene`.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneDef {
    /// Where it is.
    pub decl: Decl,
    /// The title.
    pub title: Option<String>,
    /// `pose target = value` entries.
    pub poses: Vec<PoseDef>,
    /// `show:`
    pub show: Option<RExpr>,
    /// `hide:`
    pub hide: Option<RExpr>,
    /// `explode:`
    pub explode: Option<RExpr>,
    /// `section:`
    pub section: Option<RExpr>,
}

/// `pose target = value`
#[derive(Debug, Clone, PartialEq)]
pub struct PoseDef {
    /// The assembly.
    pub target: RExpr,
    /// `open`, `closed`, an angle or a length.
    pub value: RExpr,
    /// The whole entry.
    pub span: Span,
}

/// A role of a kind.
#[derive(Debug, Clone, PartialEq)]
pub struct RoleDef {
    /// The name.
    pub name: String,
    /// Where it is written.
    pub span: Span,
    /// What it binds.
    pub ty: stave_schema::kinds::RoleType,
}

/// A `kind`.
#[derive(Debug, Clone, PartialEq)]
pub struct KindDef {
    /// Where it is.
    pub decl: Decl,
    /// What it is a kind of.
    pub base: BaseKind,
    /// Its roles, in order; the first takes the positional argument.
    pub roles: Vec<RoleDef>,
    /// Its params, in order.
    pub params: Vec<ParamDef>,
    /// Its requires, checked per pair.
    pub requires: Vec<RequireDef>,
    /// Fields of a hardware, fastener or tool kind, by name.
    pub fields: Vec<Setting>,
    /// The joint entries, for a joint kind.
    pub joint: Option<JointKindDef>,
    /// The members with a body (`member arm { ... }`), in source order;
    /// a [`BodyId`] indexes this list.
    pub bodies: Vec<BodyDef>,
    /// How the bodies move, when the kind says.
    pub mechanism: Option<MechanismDef>,
    /// Where it sits and what it cuts, for a slide, hinge or pull kind.
    pub hardware: Option<HardwareKindDef>,
}

impl KindDef {
    /// The role at `index`.
    pub fn role(&self, index: RoleIndex) -> Option<&RoleDef> {
        self.roles.get(index.index())
    }

    /// The rule at `index`.
    pub fn require(&self, index: RequireIndex) -> Option<&RequireDef> {
        self.requires.get(index.index())
    }

    /// Each rule with its index, in order.
    pub fn indexed_requires(&self) -> impl Iterator<Item = (RequireIndex, &RequireDef)> {
        self.requires
            .iter()
            .enumerate()
            .map(|(index, require)| (RequireIndex::from_usize(index), require))
    }
}

/// A member with a body: its index among its kind's [`KindDef::bodies`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize)]
pub struct BodyId(u32);

impl BodyId {
    /// The id of the body at `index`, if it fits.
    pub fn new(index: usize) -> Option<BodyId> {
        u32::try_from(index).ok().map(BodyId)
    }

    /// Its index in [`KindDef::bodies`].
    pub fn index(self) -> usize {
        usize::try_from(self.0).unwrap_or(usize::MAX)
    }
}

/// A role of a kind: its index among [`KindDef::roles`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RoleIndex(u32);

impl RoleIndex {
    /// The first role: the one the positional argument binds.
    pub const FIRST: RoleIndex = RoleIndex(0);

    /// The role at `index`, if it fits.
    pub fn new(index: usize) -> Option<RoleIndex> {
        u32::try_from(index).ok().map(RoleIndex)
    }

    /// Its index in [`KindDef::roles`].
    pub fn index(self) -> usize {
        usize::try_from(self.0).unwrap_or(usize::MAX)
    }
}

/// The name of a member with a body: a word, as the kind writes it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BodyName(String);

impl BodyName {
    /// The name, if `word` is a word (letters, digits and `_`, not starting
    /// with a digit).
    pub fn new(word: &str) -> Option<BodyName> {
        stave_lang::ast::is_word(word).then(|| BodyName(word.to_owned()))
    }

    /// The name as written.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for BodyName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// `member arm { with: on, shape body: box(...), ... }`: a member with a
/// body, one rigid piece of the hardware.
#[derive(Debug, Clone, PartialEq)]
pub struct BodyDef {
    /// Its name.
    pub name: BodyName,
    /// Where its name is written.
    pub span: Span,
    /// Its doc comments, one line each.
    pub docs: Vec<String>,
    /// What carries it.
    pub with: Carrier,
    /// What it is, for a joint's member: a fastener kind value
    /// (`member pin : dowel_pin(...)`), counted in the hardware list once
    /// for every copy placed.
    pub item: Option<RExpr>,
    /// Its shapes, in source order.
    pub shapes: NonEmpty<NamedShape>,
    /// Its own `mount { }`: the frame axes it is placed on from an anchor
    /// instead of from the kind's origin, each at most once.
    pub mount: Vec<MemberPlacement>,
}

/// One entry of a member's `mount { }`: `depth: from(face(on, toward:
/// piece), inset: plate_setback)`.
#[derive(Debug, Clone, PartialEq)]
pub struct MemberPlacement {
    /// The frame axis it places the member on.
    pub axis: FrameAxis,
    /// The anchor and inset.
    pub from: FromTerm,
}

/// The name of a cut: `dado` in `cut dado: crossing(into, piece) { ... }`.
/// What messages call the cut.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize)]
pub struct CutName(String);

impl CutName {
    /// The name, if `word` is a word (letters, digits and `_`, not starting
    /// with a digit).
    pub fn new(word: &str) -> Option<CutName> {
        stave_lang::ast::is_word(word).then(|| CutName(word.to_owned()))
    }

    /// The name as written.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The name as a message writes it, its underscores read as spaces:
    /// `mating_groove` is "mating groove".
    pub fn words(&self) -> String {
        self.0.replace('_', " ")
    }
}

impl std::fmt::Display for CutName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The entries of a slide, hinge or pull kind that say where it sits and
/// what it cuts (grammar, "Hardware placement" and "Faces to cut").
#[derive(Debug, Clone, PartialEq)]
pub struct HardwareKindDef {
    /// Its `mount { }`.
    pub mount: MountDef,
    /// Its `cut` entries, in source order.
    pub cuts: Vec<MemberCutDef>,
    /// `uses:` fasteners, counted and never placed.
    pub uses: Vec<UseDef>,
    /// Its `adjust` entries, in source order.
    pub adjusts: Vec<AdjustDef>,
}

/// `adjust a on b: axis within d` (grammar, "Adjustment"): member `body`
/// can be set up to `within` either way on member `on`, along `axis` of
/// the kind's frame. On that axis exactly one of the two has its own
/// mount, and each of them is adjusted on it at most once.
#[derive(Debug, Clone, PartialEq)]
pub struct AdjustDef {
    /// The member that is set.
    pub body: BodyId,
    /// The member it is set on.
    pub on: BodyId,
    /// The frame axis.
    pub axis: FrameAxis,
    /// How far either way: a length over the kind's params.
    pub within: RExpr,
    /// The whole entry.
    pub span: Span,
}

/// A hardware kind's `mount { }`: the face of its piece it sits on and
/// where its frame's origin is across and along that face.
#[derive(Debug, Clone, PartialEq)]
pub struct MountDef {
    /// `face:`
    pub face: FaceRule,
    /// `across:`
    pub across: AxisRule,
    /// `along:`
    pub along: AxisRule,
    /// The whole block.
    pub span: Span,
}

/// How a hardware kind picks the face of its piece it is mounted on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaceRule {
    /// `face: back`: always this face.
    Word(Face),
    /// `face: face(piece, nearer: in)`: of the two faces across the piece's
    /// thickness, the one nearer the same-named face of a region.
    Nearer {
        /// The role whose part the hardware is mounted on.
        part: RoleIndex,
        /// The region role.
        region: RoleIndex,
    },
}

/// One axis of a hardware kind's `mount { }`.
#[derive(Debug, Clone, PartialEq)]
pub enum AxisRule {
    /// `open`: the design's `place { }` sets it.
    Open,
    /// The kind sets it.
    Term(MountTerm),
}

/// A term of a `mount { }` entry.
#[derive(Debug, Clone, PartialEq)]
pub enum MountTerm {
    /// `from(anchor, inset: d)`
    From(FromTerm),
    /// `center(p)`: midway across the mount face of `p` along the axis.
    Centre {
        /// The role whose part it is centred on.
        part: RoleIndex,
    },
}

/// `from(anchor, inset: d)`: the anchor's plane, moved `d` into the piece
/// (from an edge or an end) or into the anchor's part (from a face).
#[derive(Debug, Clone, PartialEq)]
pub struct FromTerm {
    /// The anchor.
    pub anchor: MountSelector,
    /// `inset:`; zero when left out.
    pub inset: Option<RExpr>,
    /// The whole term.
    pub span: Span,
}

/// An anchor of a `mount { }` term (grammar, "Hardware placement").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MountSelector {
    /// `edge(p, nearer: q)`
    EdgeNearer {
        /// The part whose mount face's edge it is.
        part: RoleIndex,
        /// The part or region whose middle decides.
        other: RoleIndex,
    },
    /// `edge(p, flush_with: q)`
    EdgeFlushWith {
        /// The part whose mount face's edge it is.
        part: RoleIndex,
        /// The part a neighbouring face lies in a face of.
        other: RoleIndex,
    },
    /// `end(p, toward: dir)`
    EndToward {
        /// The part whose end it is.
        part: RoleIndex,
        /// The direction the end faces.
        toward: Direction,
    },
    /// `face(p, toward: q)`: of `p`'s two faces across the entry's axis,
    /// the one nearer the middle of `q`.
    FaceToward {
        /// The part whose face it is.
        part: RoleIndex,
        /// The part or region whose middle decides.
        other: RoleIndex,
    },
    /// `crossing(p, q)`: the face of `p` that `q` touches or enters.
    Crossing {
        /// The part whose face it is.
        part: RoleIndex,
        /// The part that touches or enters it.
        other: RoleIndex,
    },
}

/// `cut [name:] crossing(role, member) { operations }`: the wood a member
/// takes out of a role's part, in a hardware kind or a joint kind.
#[derive(Debug, Clone, PartialEq)]
pub struct MemberCutDef {
    /// Its name, when written.
    pub name: Option<CutName>,
    /// The role whose part is cut.
    pub part: RoleIndex,
    /// The member whose shapes make the footprints.
    pub body: BodyId,
    /// The operations in order.
    pub operations: Vec<OperationDef>,
    /// The whole entry.
    pub span: Span,
}

/// What carries a member with a body.
#[derive(Debug, Clone, PartialEq)]
pub enum Carrier {
    /// `with: piece`: it moves with that role's part.
    Role(RoleIndex),
    /// `with: plate`: it rides on another member.
    Body(BodyId),
    /// No `with:`: the mechanism places it (a link).
    Mechanism,
    /// `on: contact(piece, into)`: a joint's member, laid out on the
    /// patch where two of the joint's parts touch, once or several times.
    Contact(Box<LayoutDef>),
}

/// Where a joint's member goes (grammar, "Joint members"): `on:` a patch,
/// `count:` copies, `spaced:` along it.
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutDef {
    /// `on:`: a `Patch`, `contact(a, b)`.
    pub on: RExpr,
    /// `count:`: how many copies, per pair; one when left out.
    pub count: Option<RExpr>,
    /// `spaced:`: a `Spacing`; `evenly` when left out.
    pub spaced: Option<RExpr>,
    /// The `on:` entry.
    pub span: Span,
}

/// One of a member's shapes, with its name when the kind gives one.
#[derive(Debug, Clone, PartialEq)]
pub struct NamedShape {
    /// `knuckle` in `shape knuckle: box(...)`.
    pub name: Option<String>,
    /// The whole entry.
    pub span: Span,
    /// The solid.
    pub shape: ShapeDef,
}

/// A convex solid in the kind's frame (grammar, "Shapes").
#[derive(Debug, Clone, PartialEq)]
pub enum ShapeDef {
    /// `box(along:, across:, depth:)`
    Box {
        /// Its extent along `along`.
        along: RangeDef,
        /// Its extent along `across`.
        across: RangeDef,
        /// Its extent along `depth`.
        depth: RangeDef,
    },
    /// `cylinder(diameter:, depth:, at:, axis:)`
    Cylinder {
        /// Its diameter.
        diameter: RExpr,
        /// Its extent along its axis.
        extent: RangeDef,
        /// The point its axis runs through.
        at: Box<KindPoint>,
        /// The frame direction its axis runs along.
        axis: FrameAxis,
    },
    /// `prism(points: [...], along:)`
    Prism {
        /// The outline in the across/depth plane, in order around it.
        points: NonEmpty<KindPoint>,
        /// Its extent along `along`.
        along: RangeDef,
    },
    /// `bar(from:, to:, thickness:, along:)`
    Bar {
        /// One end's centre.
        from: Box<KindPoint>,
        /// The other end's centre.
        to: Box<KindPoint>,
        /// How thick it is in the across/depth plane.
        thickness: RExpr,
        /// Its extent along `along`.
        along: RangeDef,
    },
}

/// A range argument of a shape: `lo..hi`, or one length centred on the
/// origin.
#[derive(Debug, Clone, PartialEq)]
pub enum RangeDef {
    /// One length, centred on the origin.
    Centred(RExpr),
    /// `lo..hi`.
    Between {
        /// The low end.
        low: RExpr,
        /// The high end.
        high: RExpr,
    },
}

/// `point(along:, across:, depth:)`: a point in the kind's frame. Each
/// coordinate is a length expression; unwritten ones are `0mm`.
#[derive(Debug, Clone, PartialEq)]
pub struct KindPoint {
    /// Along the piece's length.
    pub along: RExpr,
    /// Across the mount face, toward the middle of the piece.
    pub across: RExpr,
    /// From the mount face into the piece.
    pub depth: RExpr,
}

/// One direction of the kind's frame (grammar, "The kind's frame").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize)]
pub enum FrameAxis {
    /// Along the piece's length.
    Along,
    /// Across the mount face.
    Across,
    /// Into the piece from the mount face.
    Depth,
}

impl FrameAxis {
    /// Every direction, in the order the grammar lists them.
    pub const ALL: [FrameAxis; 3] = [FrameAxis::Along, FrameAxis::Across, FrameAxis::Depth];

    /// The word as written.
    pub fn word(self) -> &'static str {
        match self {
            FrameAxis::Along => "along",
            FrameAxis::Across => "across",
            FrameAxis::Depth => "depth",
        }
    }

    /// The direction for a word, if it is one.
    pub fn from_word(word: &str) -> Option<FrameAxis> {
        FrameAxis::ALL.into_iter().find(|axis| axis.word() == word)
    }
}

/// `mechanism { ... }`: how a hardware kind's bodies move.
#[derive(Debug, Clone, PartialEq)]
pub struct MechanismDef {
    /// The `mechanism` word and its block.
    pub span: Span,
    /// Its pivots, slots and rails, in source order.
    pub constraints: Vec<ConstraintDef>,
    /// Its couplings, in source order.
    pub couplings: Vec<CouplingDef>,
    /// Its one input; `None` for a mechanism that follows the hardware
    /// that drives its assembly (a lid stay).
    pub drive: Option<DriveDef>,
    /// What stops the drive, when the kind says.
    pub stop: Option<StopDef>,
    /// How thick every pin is, when the kind says.
    pub pin_diameter: Option<RExpr>,
}

/// A pivot, slot or rail between two bodies.
#[derive(Debug, Clone, PartialEq)]
pub enum ConstraintDef {
    /// `pivot a, b at point(...)`
    Pivot {
        /// The two bodies on the pin.
        bodies: [BodyId; 2],
        /// Where the pin is.
        at: Box<KindPoint>,
        /// The whole entry.
        span: Span,
    },
    /// `slot a, b from point(...) to point(...)`
    Slot {
        /// The body with the pin, then the body with the slot.
        bodies: [BodyId; 2],
        /// One end of the slot.
        from: Box<KindPoint>,
        /// The other end.
        to: Box<KindPoint>,
        /// The whole entry.
        span: Span,
    },
    /// `rail a, b along <word>`
    Rail {
        /// The body that runs, then the body it runs along.
        bodies: [BodyId; 2],
        /// The direction it runs.
        along: FrameAxis,
        /// The whole entry.
        span: Span,
    },
}

/// `couple a to b: ratio`
#[derive(Debug, Clone, PartialEq)]
pub struct CouplingDef {
    /// The body that follows.
    pub body: BodyId,
    /// The body it follows.
    pub to: BodyId,
    /// The share of `to`'s travel, a ratio.
    pub ratio: RExpr,
    /// The whole entry.
    pub span: Span,
}

/// `drive: turn(...)` or `drive: travel(...)`
#[derive(Debug, Clone, PartialEq)]
pub enum DriveDef {
    /// `turn(m, from:, to:)`: angles.
    Turn {
        /// The driven body.
        body: BodyId,
        /// The closed pose's angle.
        from: RExpr,
        /// The widest angle.
        to: RExpr,
        /// The whole entry.
        span: Span,
    },
    /// `travel(m, from:, to:)`: lengths.
    Travel {
        /// The driven body.
        body: BodyId,
        /// The closed pose's travel.
        from: RExpr,
        /// The furthest travel.
        to: RExpr,
        /// The whole entry.
        span: Span,
    },
}

impl DriveDef {
    /// The driven body.
    pub fn body(&self) -> BodyId {
        match self {
            DriveDef::Turn { body, .. } | DriveDef::Travel { body, .. } => *body,
        }
    }

    /// The whole entry.
    pub fn span(&self) -> Span {
        match self {
            DriveDef::Turn { span, .. } | DriveDef::Travel { span, .. } => *span,
        }
    }
}

/// `stop: value "message"`
#[derive(Debug, Clone, PartialEq)]
pub struct StopDef {
    /// Where the drive stops: an angle for a turn, a length for a travel.
    pub value: RExpr,
    /// What stops it, for a beginner.
    pub message: Text,
    /// The whole entry.
    pub span: Span,
}

impl KindDef {
    /// The index of a param by name.
    pub fn param(&self, name: &str) -> Option<usize> {
        self.params.iter().position(|p| p.decl.name == name)
    }

    /// The role named `name`.
    pub fn role_named(&self, name: &str) -> Option<RoleIndex> {
        self.roles
            .iter()
            .position(|r| r.name == name)
            .and_then(RoleIndex::new)
    }

    /// A field by name.
    pub fn field(&self, name: &str) -> Option<&Setting> {
        self.fields.iter().find(|f| f.name == name)
    }
}

/// The entries of a joint kind.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct JointKindDef {
    /// `insert:`
    pub insert: Vec<StepDef>,
    /// `cut_first:` roles in order.
    pub cut_first: Vec<RoleIndex>,
    /// `adds:`
    pub adds: Vec<ExtendDef>,
    /// `uses:`
    pub uses: Vec<UseDef>,
    /// `cut selector { }` entries.
    pub cuts: Vec<CutDef>,
    /// `cut crossing(role, member) { }` entries: the wood the kind's
    /// members take out of its parts.
    pub member_cuts: Vec<MemberCutDef>,
    /// `makes part` entries.
    pub makes: Vec<MadePartDef>,
    /// `note:`
    pub note: Option<Text>,
}

impl JointKindDef {
    /// The part it makes at `index`.
    pub fn made(&self, index: MadeIndex) -> Option<&MadePartDef> {
        self.makes.get(index.index())
    }
}

/// Who an assembly step or selector refers to: a role or a made part.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JointMember {
    /// A role of the kind.
    Role(RoleIndex),
    /// A part the kind makes.
    Made(MadeIndex),
}

/// One step of `insert:`.
#[derive(Debug, Clone, PartialEq)]
pub struct StepDef {
    /// The part that moves.
    pub part: JointMember,
    /// `along:` (push, drive) or `about:` (rotate): an axis of the moving
    /// part.
    pub axis: PartAxis,
    /// What it does, with what that kind of step takes.
    pub motion: StepMotionDef,
    /// The whole step.
    pub span: Span,
}

/// What an assembly step does: `push`, `rotate` or `drive`, each with only
/// the arguments it takes.
#[derive(Debug, Clone, PartialEq)]
pub enum StepMotionDef {
    /// `push(part, along: axis, by: length, angle: angle)`.
    Push {
        /// `by:`, a length, when written.
        by: Option<RExpr>,
        /// `angle:`, when written.
        angle: Option<RExpr>,
    },
    /// `rotate(part, by: angle, about: axis)`.
    Rotate {
        /// `by:`, an angle; a rotate always has one.
        by: RExpr,
    },
    /// `drive(part, along: axis)`.
    Drive,
}

impl StepMotionDef {
    /// The step's word: `push`, `rotate` or `drive`.
    pub fn kind(&self) -> AssemblyStepKind {
        match self {
            StepMotionDef::Push { .. } => AssemblyStepKind::Push,
            StepMotionDef::Rotate { .. } => AssemblyStepKind::Rotate,
            StepMotionDef::Drive => AssemblyStepKind::Drive,
        }
    }
}

/// `extend(role, by: d)`
#[derive(Debug, Clone, PartialEq)]
pub struct ExtendDef {
    /// The role extended.
    pub role: RoleIndex,
    /// By how much.
    pub by: RExpr,
    /// The whole call.
    pub span: Span,
}

/// A fastener used by a joint.
#[derive(Debug, Clone, PartialEq)]
pub struct UseDef {
    /// The fastener kind value.
    pub fastener: RExpr,
    /// `count:` per pair; one when left out.
    pub count: Option<RExpr>,
    /// The whole entry.
    pub span: Span,
}

/// A face selector, resolved.
#[derive(Debug, Clone, PartialEq)]
pub struct SelectorDef {
    /// `end`, `side` or `crossing`.
    pub selector: Selector,
    /// The part cut.
    pub part: JointMember,
    /// The other part: `facing:`, `from:` or crossing's second argument.
    pub other: JointMember,
    /// `normal:`
    pub normal: Option<PartAxis>,
    /// `opposite: true`
    pub opposite: bool,
    /// `flip: true`
    pub flip: bool,
    /// The whole selector.
    pub span: Span,
}

/// A `cut selector { operations }` entry.
#[derive(Debug, Clone, PartialEq)]
pub struct CutDef {
    /// Its name, when written: what messages call the cut.
    pub name: Option<CutName>,
    /// The face.
    pub selector: SelectorDef,
    /// The operations in order.
    pub operations: Vec<OperationDef>,
    /// The whole entry.
    pub span: Span,
}

/// One cut operation with its arguments.
#[derive(Debug, Clone, PartialEq)]
pub struct OperationDef {
    /// The operation.
    pub operation: CutOperation,
    /// Arguments by name.
    pub args: Vec<Setting>,
    /// `mark(along:, across:)` points of an outline.
    pub marks: Vec<(RExpr, RExpr)>,
    /// The whole call.
    pub span: Span,
}

/// A `makes part` entry.
#[derive(Debug, Clone, PartialEq)]
pub struct MadePartDef {
    /// The name.
    pub name: String,
    /// Where the name is written.
    pub name_span: Span,
    /// The body; its place terms are made-part terms.
    pub body: Box<PartBodyDef>,
    /// The made-part placement.
    pub place: Vec<MadeTerm>,
    /// The whole entry.
    pub span: Span,
}

/// A term of a made part's `place { }`.
#[derive(Debug, Clone, PartialEq)]
pub struct MadeTerm {
    /// The term.
    pub kind: MadeTermKind,
    /// The whole term.
    pub span: Span,
}

/// The made-part placement terms.
#[derive(Debug, Clone, PartialEq)]
pub enum MadeTermKind {
    /// `orient(length: a, width: b, of: role)`
    Orient {
        /// The role part's axis the length runs along.
        length: PartAxis,
        /// The role part's axis the width runs along.
        width: PartAxis,
        /// The role.
        of: RoleIndex,
    },
    /// `center(selector)` or `center(selector, axis)`.
    Center {
        /// The place.
        selector: SelectorDef,
        /// One axis of the `of` part, or all three.
        axis: Option<PartAxis>,
    },
    /// `against(selector, gap: d)`
    Against {
        /// The face.
        selector: SelectorDef,
        /// The gap.
        gap: Option<RExpr>,
    },
    /// `flush(selector, inset: d)`
    Flush {
        /// The face.
        selector: SelectorDef,
        /// The inset.
        inset: Option<RExpr>,
    },
}

/// A `recipe`.
#[derive(Debug, Clone, PartialEq)]
pub struct RecipeDef {
    /// Where it is.
    pub decl: Decl,
    /// The operation it makes.
    pub operation: Operation,
    /// `tools:`
    pub tools: Vec<KindId>,
    /// `rank:`
    pub rank: u32,
    /// `needs:`
    pub needs: Vec<Need>,
    /// `mark { }`
    pub mark: Vec<MarkStepDef>,
    /// `cut { }`
    pub cut: Vec<CutStepDef>,
    /// `check:`
    pub check: Option<String>,
}

/// One entry of `needs:`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Need {
    /// The need.
    pub kind: NeedKind,
    /// The operation of `before(...)` or `after(...)`.
    pub operation: Option<Operation>,
}

/// A marking step.
#[derive(Debug, Clone, PartialEq)]
pub struct MarkStepDef {
    /// The step.
    pub step: MarkingStep,
    /// Its positional argument: a feature or (for `gauge`) a measurement.
    pub target: String,
    /// `on:` of `gauge`.
    pub on: Option<String>,
    /// `with:`
    pub with: Option<KindId>,
    /// `note:`
    pub note: Option<String>,
    /// The whole step.
    pub span: Span,
}

/// A cutting step.
#[derive(Debug, Clone, PartialEq)]
pub struct CutStepDef {
    /// The step.
    pub step: CuttingStep,
    /// The feature it works on.
    pub feature: String,
    /// `to:`: a measurement or a feature.
    pub to: Option<String>,
    /// `passes:`
    pub passes: Option<u32>,
    /// `guide:`
    pub guide: Option<String>,
    /// `diameter:` of `bore`.
    pub diameter: Option<String>,
    /// `with:`
    pub with: Option<KindId>,
    /// `note:`
    pub note: Option<String>,
    /// The whole step.
    pub span: Span,
}

/// The `project` item.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectDef {
    /// The title.
    pub title: String,
    /// `goal:`
    pub goal: Option<String>,
    /// `units:`
    pub units: OutputUnit,
    /// `resolution:`
    pub resolution: Option<RExpr>,
    /// `style:`
    pub style: Option<String>,
    /// `tools:`; `None` means every tool.
    pub tools: Option<Vec<KindId>>,
    /// `methods:`
    pub methods: Vec<RecipeId>,
    /// `boards:`: how boards are joined in every part that doesn't say.
    pub boards: Option<RExpr>,
    /// The file it is in.
    pub file: FileId,
    /// The whole item.
    pub span: Span,
}

/// The `library` item.
#[derive(Debug, Clone, PartialEq)]
pub struct LibraryDef {
    /// The title.
    pub title: String,
    /// `about:`
    pub about: Option<String>,
    /// `license:`
    pub license: Option<String>,
    /// `boards:`: how a part's boards are joined when neither the part nor
    /// its project says (grammar, "Boards"), for a library in the prelude.
    pub boards: Option<RExpr>,
    /// The file the item is in.
    pub file: FileId,
}

/// What a file sees besides its module: its imports.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FileScope {
    /// The module the file belongs to.
    pub module: Option<ModuleId>,
    /// Import names and the modules they name.
    pub imports: std::collections::BTreeMap<String, ModuleId>,
}

/// One module's resolved items.
#[derive(Debug, Clone, PartialEq)]
pub struct ModuleDefs {
    /// The module.
    pub id: ModuleId,
    /// Its default name.
    pub name: String,
    /// The folder its files are in, for a module on this computer; `None`
    /// for a standard module. A kind's `model(...)` path is relative to it.
    pub folder: Option<std::path::PathBuf>,
    /// Its items in file order, then source order.
    pub items: Vec<DefId>,
    /// Its `project` item, for a project module.
    pub project: Option<ProjectDef>,
    /// Its `library` item, for a library module.
    pub library: Option<LibraryDef>,
}

/// Every resolved definition of a module set.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Defs {
    /// Per module, indexed by [`ModuleId`].
    pub modules: Vec<ModuleDefs>,
    /// The standard modules whose public items are the prelude.
    pub prelude: Vec<ModuleId>,
    /// Per file: its module and what it imports under which name.
    pub files: std::collections::BTreeMap<FileId, FileScope>,
    /// Declarations that had a name or type error: anything that depends
    /// on them is skipped without further errors.
    pub poisoned: std::collections::BTreeSet<DefId>,
    /// Names of items left out because of a syntax error, per module
    /// (`Component.item` for items inside a component). Uses of them are
    /// skipped without an `unknown-name`.
    pub broken: std::collections::BTreeMap<ModuleId, std::collections::BTreeSet<String>>,
    /// `true` when loading, parsing, resolving or instantiating reported an
    /// error. A later stage that meets something an earlier check should
    /// have caught then knows the user was told; when this is `false`, such
    /// a failure is a bug, and is reported as one rather than dropped.
    pub earlier_errors: bool,
    /// `param`s of modules and components.
    pub params: Vec<ParamDef>,
    /// `let`s.
    pub lets: Vec<LetDef>,
    /// `region`s.
    pub regions: Vec<RegionDef>,
    /// `part`s.
    pub parts: Vec<PartDef>,
    /// `joint`s.
    pub joints: Vec<JointDef>,
    /// `hardware`.
    pub hardware: Vec<HardwareDef>,
    /// `assembly`s.
    pub assemblies: Vec<AssemblyDef>,
    /// `require`s of modules and components.
    pub requires: Vec<RequireDef>,
    /// `stock`s.
    pub stocks: Vec<StockDef>,
    /// `profile`s.
    pub profiles: Vec<ProfileDef>,
    /// `kind`s.
    pub kinds: Vec<KindDef>,
    /// `recipe`s.
    pub recipes: Vec<RecipeDef>,
    /// `component`s.
    pub components: Vec<ComponentDef>,
    /// `scene`s.
    pub scenes: Vec<SceneDef>,
    /// The text of each file of the project's own module, for the fixes
    /// that rewrite part of an expression as written (grammar, "Fixes").
    pub sources: std::collections::BTreeMap<FileId, String>,
}

impl Defs {
    /// The text of the project's module at `span` of `file`; `None` for a
    /// file of another module.
    pub fn source_text(&self, file: FileId, span: Span) -> Option<&str> {
        let start = usize::try_from(span.start()).ok()?;
        let end = usize::try_from(span.end()).ok()?;
        self.sources.get(&file)?.get(start..end)
    }
}

macro_rules! getters {
    ($( $method:ident($id:ty) -> $def:ty, $field:ident; )*) => {
        impl Defs {
            $(
                #[doc = concat!("The definition behind a [`", stringify!($id), "`].")]
                pub fn $method(&self, id: $id) -> Option<&$def> {
                    self.$field.get(id.index())
                }
            )*
        }
    };
}

getters! {
    param(ParamDefId) -> ParamDef, params;
    let_def(LetDefId) -> LetDef, lets;
    region(RegionDefId) -> RegionDef, regions;
    part(PartDefId) -> PartDef, parts;
    joint(JointDefId) -> JointDef, joints;
    hardware_def(HardwareDefId) -> HardwareDef, hardware;
    assembly(AssemblyDefId) -> AssemblyDef, assemblies;
    require(RequireDefId) -> RequireDef, requires;
    stock(StockId) -> StockDef, stocks;
    profile(ProfileId) -> ProfileDef, profiles;
    kind(KindId) -> KindDef, kinds;
    recipe(RecipeId) -> RecipeDef, recipes;
    component(ComponentId) -> ComponentDef, components;
    scene(SceneDefId) -> SceneDef, scenes;
}

impl Defs {
    /// How a patch names a declaration of the project's own module
    /// (`module`): its name at the top level (`runner`), or the
    /// component's name and its own inside a component (`Door.hinges`).
    /// `None` for a declaration of another module, which a fix doesn't
    /// edit (grammar, "Fixes").
    pub fn patch_path(&self, id: DefId, module: ModuleId) -> Option<String> {
        let decl = self.decl(id)?;
        match decl.scope {
            Scope::Module(found) => (found == module).then(|| decl.name.clone()),
            Scope::Component(component) => {
                let outer = &self.component(component)?.decl;
                (outer.scope == Scope::Module(module))
                    .then(|| format!("{}.{}", outer.name, decl.name))
            }
        }
    }

    /// Where any declaration is.
    pub fn decl(&self, id: DefId) -> Option<&Decl> {
        match id {
            DefId::Param(id) => self.param(id).map(|d| &d.decl),
            DefId::Let(id) => self.let_def(id).map(|d| &d.decl),
            DefId::Region(id) => self.region(id).map(|d| &d.decl),
            DefId::Part(id) => self.part(id).map(|d| &d.decl),
            DefId::Joint(id) => self.joint(id).map(|d| &d.decl),
            DefId::Hardware(id) => self.hardware_def(id).map(|d| &d.decl),
            DefId::Assembly(id) => self.assembly(id).map(|d| &d.decl),
            DefId::Require(id) => self.require(id).map(|d| &d.decl),
            DefId::Stock(id) => self.stock(id).map(|d| &d.decl),
            DefId::Profile(id) => self.profile(id).map(|d| &d.decl),
            DefId::Kind(id) => self.kind(id).map(|d| &d.decl),
            DefId::Recipe(id) => self.recipe(id).map(|d| &d.decl),
            DefId::Component(id) => self.component(id).map(|d| &d.decl),
            DefId::Scene(id) => self.scene(id).map(|d| &d.decl),
        }
    }

    /// The array declaration of an item, if it is an array.
    pub fn array(&self, id: DefId) -> Option<&ArrayDef> {
        match id {
            DefId::Region(id) => self.region(id)?.array.as_ref(),
            DefId::Part(id) => self.part(id)?.array.as_ref(),
            DefId::Joint(id) => self.joint(id)?.array.as_ref(),
            DefId::Hardware(id) => self.hardware_def(id)?.array.as_ref(),
            DefId::Assembly(id) => self.assembly(id)?.array.as_ref(),
            DefId::Param(_)
            | DefId::Let(_)
            | DefId::Require(_)
            | DefId::Stock(_)
            | DefId::Profile(_)
            | DefId::Kind(_)
            | DefId::Recipe(_)
            | DefId::Component(_)
            | DefId::Scene(_) => None,
        }
    }

    /// A kind's name as a file writes it: bare for kinds of the file's own
    /// module and of the prelude, `alias.name` for a kind the file imports
    /// (under the name it imports it), and `folder.name` otherwise.
    pub fn kind_display(&self, kind: KindId, file: Option<FileId>) -> String {
        let Some(def) = self.kind(kind) else {
            return String::new();
        };
        let Scope::Module(module) = def.decl.scope else {
            return def.decl.name.clone();
        };
        let scope = file.and_then(|f| self.files.get(&f));
        if scope.is_some_and(|s| s.module == Some(module)) {
            return def.decl.name.clone();
        }
        if let Some((alias, _)) = scope.and_then(|s| s.imports.iter().find(|(_, m)| **m == module))
        {
            return format!("{alias}.{}", def.decl.name);
        }
        if self.prelude.contains(&module)
            || scope.is_none() && self.modules.first().map(|m| m.id) == Some(module)
        {
            return def.decl.name.clone();
        }
        let folder = self
            .modules
            .get(module.index())
            .map(|m| m.name.as_str())
            .unwrap_or("");
        format!("{folder}.{}", def.decl.name)
    }

    /// `true` if the declaration had a name or type error.
    pub fn is_poisoned(&self, def: DefId) -> bool {
        self.poisoned.contains(&def)
    }

    /// The project item of the module, if it is a project module.
    pub fn project(&self, module: ModuleId) -> Option<&ProjectDef> {
        self.modules.get(module.index())?.project.as_ref()
    }
}
