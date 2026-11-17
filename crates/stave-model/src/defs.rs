//! Resolved definitions: every item of every module with its names
//! resolved and its expressions typed, before anything is instantiated.
//!
//! Definitions are generic: a component's parts are defined once here and
//! instantiated once per assembly by [`crate::instance`]. Expressions refer
//! to items through [`Symbol`]s; which concrete item a symbol means is
//! decided by the instance it is evaluated in.

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
    AssemblyDefId, ComponentId, HardwareDefId, JointDefId, KindId, LetDefId, ParamDefId, PartDefId,
    ProfileId, RecipeId, RegionDefId, RequireDefId, SceneDefId, StockId,
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
    Role(KindId, usize),
    /// A param of the kind being defined.
    KindParam(KindId, usize),
    /// A part the kind makes.
    MadePart(KindId, usize),
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
    /// `bounds`
    Bounds,
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
    /// `true` when `locked`: smart edits must not change it.
    pub locked: bool,
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
    /// The named, documented members of a hardware kind (`member cup`),
    /// in source order.
    pub members: Vec<MemberDef>,
}

/// `member cup` in a hardware kind: a named piece, with the doc comments
/// written above it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberDef {
    /// Which member.
    pub name: stave_schema::kinds::MemberName,
    /// Where its name is written.
    pub span: Span,
    /// Its doc comments, one line each.
    pub docs: Vec<String>,
}

impl KindDef {
    /// The index of a param by name.
    pub fn param(&self, name: &str) -> Option<usize> {
        self.params.iter().position(|p| p.decl.name == name)
    }

    /// The index of a role by name.
    pub fn role(&self, name: &str) -> Option<usize> {
        self.roles.iter().position(|r| r.name == name)
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
    pub cut_first: Vec<usize>,
    /// `adds:`
    pub adds: Vec<ExtendDef>,
    /// `uses:`
    pub uses: Vec<UseDef>,
    /// `cut selector { }` entries.
    pub cuts: Vec<CutDef>,
    /// `makes part` entries.
    pub makes: Vec<MadePartDef>,
    /// `note:`
    pub note: Option<Text>,
}

/// Who an assembly step or selector refers to: a role or a made part.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JointMember {
    /// A role of the kind.
    Role(usize),
    /// A part the kind makes.
    Made(usize),
}

/// One step of `insert:`.
#[derive(Debug, Clone, PartialEq)]
pub struct StepDef {
    /// `push`, `rotate` or `drive`.
    pub kind: AssemblyStepKind,
    /// The part that moves.
    pub part: JointMember,
    /// `along:` or `about:`: an axis of the moving part.
    pub axis: PartAxis,
    /// `by:` (a length for push, an angle for rotate).
    pub by: Option<RExpr>,
    /// `angle:` of a push.
    pub angle: Option<RExpr>,
    /// The whole step.
    pub span: Span,
}

/// `extend(role, by: d)`
#[derive(Debug, Clone, PartialEq)]
pub struct ExtendDef {
    /// The role extended.
    pub role: usize,
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
        of: usize,
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
