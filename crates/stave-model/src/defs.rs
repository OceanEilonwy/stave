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
use stave_schema::kinds::BaseField;
use stave_schema::operations::Selector;
use stave_schema::placement::AssemblyStepKind;
use stave_schema::recipes::{CuttingStep, MarkingStep, NeedKind};
use stave_schema::species::Species;
use stave_schema::words::{Grain, Measure, MotionWord, OutputUnit, StockForm};
use stave_schema::{CutOperation, Operation};
use std::collections::BTreeMap;

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
    /// A built-in function call, its arguments checked and in place.
    Builtin(Box<BuiltinCall>),
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

/// A built-in function call with each argument in its own field, as the
/// resolver checked it. A required argument that was missing or could not
/// be resolved is an [`RExprKind::Error`] expression, already reported.
#[derive(Debug, Clone, PartialEq)]
pub enum BuiltinCall {
    /// `between(a, b)`
    Between {
        /// The first part, face or region.
        a: RExpr,
        /// The second.
        b: RExpr,
    },
    /// `extent(items, axis)` or `extent(items, axis, of: part)`
    Extent {
        /// The parts measured.
        items: RExpr,
        /// The axis: a world axis, or with `of:` one of its own axes.
        axis: RExpr,
        /// The part or contact patch whose axis `axis` names.
        of: Option<RExpr>,
    },
    /// `mid(item, axis)`
    Mid {
        /// The part or region.
        item: RExpr,
        /// The axis across which the mid-plane lies.
        axis: RExpr,
    },
    /// `opening(left:, right:, bottom:, top:, front:, back:)`
    Opening(OpeningFaces),
    /// `split(region, axis)`, with `at:` or `shares:`
    Split {
        /// The region split.
        region: RExpr,
        /// The axis it is split along.
        axis: RExpr,
        /// How the pieces are sized.
        by: SplitBy,
    },
    /// `cover(region, mount, thickness:)`
    Cover {
        /// The region covered.
        region: RExpr,
        /// How the cover sits on it.
        mount: RExpr,
        /// The cover's thickness.
        thickness: RExpr,
    },
    /// `overlay(left:, right:, top:, bottom:, meeting:)`; an edge left
    /// out overlays by nothing.
    Overlay {
        /// Overlay on the left edge.
        left: Option<RExpr>,
        /// Overlay on the right edge.
        right: Option<RExpr>,
        /// Overlay on the top edge.
        top: Option<RExpr>,
        /// Overlay on the bottom edge.
        bottom: Option<RExpr>,
        /// Overlay on an edge shared with a neighbouring front.
        meeting: Option<RExpr>,
    },
    /// `inset(reveal:)`
    Inset {
        /// The gap round the front.
        reveal: RExpr,
    },
    /// `turned(profile)`
    Turned {
        /// The profile turned.
        profile: RExpr,
    },
    /// `tapered(to:, toward:, on:, square:)` (grammar, "Shaped parts").
    Tapered {
        /// The size at the narrow end.
        to: RExpr,
        /// The end the taper runs to, as placed.
        toward: RExpr,
        /// The face or faces planed, as placed.
        on: RExpr,
        /// How far it stays square from the wide end.
        square: Option<RExpr>,
    },
    /// `arched(rise:, on:, ends:)` (grammar, "Shaped parts").
    Arched {
        /// How far the arch rises into the part at its middle.
        rise: RExpr,
        /// The face it is sawn into, as placed.
        on: RExpr,
        /// How much of the face stays straight at each end.
        ends: Option<RExpr>,
    },
    /// `joined(by:)`
    Joined {
        /// The joint kind or kinds.
        by: RExpr,
    },
    /// `spliced(by:, at:, from:)`
    Spliced {
        /// The joint kind each splice is made with.
        by: RExpr,
        /// Where each splice is, from `from`.
        at: RExpr,
        /// The end the positions are measured from.
        from: RExpr,
    },
    /// `box(length:, width:, thickness:)`: an envelope.
    Box {
        /// Its length.
        length: RExpr,
        /// Its width.
        width: RExpr,
        /// Its thickness.
        thickness: RExpr,
    },
    /// `box(across:, up:, deep:)`: the project's root region.
    RegionBox {
        /// Its size across.
        across: RExpr,
        /// Its size up.
        up: RExpr,
        /// Its size deep.
        deep: RExpr,
    },
    /// `cylinder(diameter:, length:)`
    Cylinder {
        /// Its diameter.
        diameter: RExpr,
        /// Its length.
        length: RExpr,
    },
    /// `model(source, align:, sha256:)`
    Model {
        /// The model file.
        source: RExpr,
        /// The node put on the kind's origin.
        align: Option<RExpr>,
        /// The file's SHA-256, when pinned.
        sha256: Option<RExpr>,
    },
    /// `node("name")`, written only as `model(align:)`: the name itself.
    Node {
        /// The node's name.
        name: RExpr,
    },
    /// `ends(inset:)`
    Ends {
        /// How far in from each end.
        inset: RExpr,
    },
    /// `pitch(d)`
    Pitch {
        /// The distance from one to the next.
        distance: RExpr,
    },
    /// `gap(d)`
    Gap {
        /// The gap between neighbours.
        distance: RExpr,
    },
    /// `contact(a, b)`
    Contact {
        /// The first part.
        a: RExpr,
        /// The second part.
        b: RExpr,
    },
    /// `edge_to_edge(a, b)`
    EdgeToEdge {
        /// The first part.
        a: RExpr,
        /// The second part.
        b: RExpr,
    },
}

/// The six planes of `opening(...)`.
#[derive(Debug, Clone, PartialEq)]
pub struct OpeningFaces {
    /// `left:`
    pub left: RExpr,
    /// `right:`
    pub right: RExpr,
    /// `bottom:`
    pub bottom: RExpr,
    /// `top:`
    pub top: RExpr,
    /// `front:`
    pub front: RExpr,
    /// `back:`
    pub back: RExpr,
}

impl OpeningFaces {
    /// The plane given for a face.
    pub fn face(&self, face: Face) -> &RExpr {
        match face {
            Face::Left => &self.left,
            Face::Right => &self.right,
            Face::Bottom => &self.bottom,
            Face::Top => &self.top,
            Face::Front => &self.front,
            Face::Back => &self.back,
        }
    }

    /// The faces in the order an opening's bounds are made: low and high
    /// across, up, then deep.
    pub const ORDER: [Face; 6] = [
        Face::Left,
        Face::Right,
        Face::Bottom,
        Face::Top,
        Face::Front,
        Face::Back,
    ];
}

/// How `split(...)` sizes its pieces.
#[derive(Debug, Clone, PartialEq)]
pub enum SplitBy {
    /// Neither `at:` nor `shares:`: equal pieces, one per array element.
    Equal,
    /// `at: part`: the two pieces either side of the part.
    At(RExpr),
    /// `shares: [n, ...]`: one relative size per piece.
    Shares(RExpr),
}

impl BuiltinCall {
    /// The function called.
    pub fn function(&self) -> Builtin {
        match self {
            BuiltinCall::Between { .. } => Builtin::Between,
            BuiltinCall::Extent { .. } => Builtin::Extent,
            BuiltinCall::Mid { .. } => Builtin::Mid,
            BuiltinCall::Opening(_) => Builtin::Opening,
            BuiltinCall::Split { .. } => Builtin::Split,
            BuiltinCall::Cover { .. } => Builtin::Cover,
            BuiltinCall::Overlay { .. } => Builtin::Overlay,
            BuiltinCall::Inset { .. } => Builtin::Inset,
            BuiltinCall::Turned { .. } => Builtin::Turned,
            BuiltinCall::Tapered { .. } => Builtin::Tapered,
            BuiltinCall::Arched { .. } => Builtin::Arched,
            BuiltinCall::Joined { .. } => Builtin::Joined,
            BuiltinCall::Spliced { .. } => Builtin::Spliced,
            BuiltinCall::Box { .. } => Builtin::Box,
            BuiltinCall::RegionBox { .. } => Builtin::RegionBox,
            BuiltinCall::Cylinder { .. } => Builtin::Cylinder,
            BuiltinCall::Model { .. } | BuiltinCall::Node { .. } => Builtin::Model,
            BuiltinCall::Ends { .. } => Builtin::Ends,
            BuiltinCall::Pitch { .. } => Builtin::Pitch,
            BuiltinCall::Gap { .. } => Builtin::Gap,
            BuiltinCall::Contact { .. } => Builtin::Contact,
            BuiltinCall::EdgeToEdge { .. } => Builtin::EdgeToEdge,
        }
    }

    /// The two parts of a question about how two parts meet
    /// (`edge_to_edge(a, b)`), or `None` for a call that asks none.
    pub fn meeting_question(&self) -> Option<(&RExpr, &RExpr)> {
        match self {
            BuiltinCall::EdgeToEdge { a, b } => Some((a, b)),
            BuiltinCall::Between { .. }
            | BuiltinCall::Extent { .. }
            | BuiltinCall::Mid { .. }
            | BuiltinCall::Opening(_)
            | BuiltinCall::Split { .. }
            | BuiltinCall::Cover { .. }
            | BuiltinCall::Overlay { .. }
            | BuiltinCall::Inset { .. }
            | BuiltinCall::Turned { .. }
            | BuiltinCall::Tapered { .. }
            | BuiltinCall::Arched { .. }
            | BuiltinCall::Joined { .. }
            | BuiltinCall::Spliced { .. }
            | BuiltinCall::Box { .. }
            | BuiltinCall::RegionBox { .. }
            | BuiltinCall::Cylinder { .. }
            | BuiltinCall::Model { .. }
            | BuiltinCall::Node { .. }
            | BuiltinCall::Ends { .. }
            | BuiltinCall::Pitch { .. }
            | BuiltinCall::Gap { .. }
            | BuiltinCall::Contact { .. } => None,
        }
    }

    /// Every argument given, in the order the function's parameters are
    /// listed, for walking the expressions inside a call.
    pub fn arguments(&self) -> impl Iterator<Item = &RExpr> {
        let slots: [Option<&RExpr>; 6] = match self {
            BuiltinCall::Between { a, b }
            | BuiltinCall::Contact { a, b }
            | BuiltinCall::EdgeToEdge { a, b } => [Some(a), Some(b), None, None, None, None],
            BuiltinCall::Extent { items, axis, of } => {
                [Some(items), Some(axis), of.as_ref(), None, None, None]
            }
            BuiltinCall::Mid { item, axis } => [Some(item), Some(axis), None, None, None, None],
            BuiltinCall::Opening(faces) => [
                Some(&faces.left),
                Some(&faces.right),
                Some(&faces.bottom),
                Some(&faces.top),
                Some(&faces.front),
                Some(&faces.back),
            ],
            BuiltinCall::Split { region, axis, by } => {
                let by = match by {
                    SplitBy::Equal => None,
                    SplitBy::At(expr) | SplitBy::Shares(expr) => Some(expr),
                };
                [Some(region), Some(axis), by, None, None, None]
            }
            BuiltinCall::Cover {
                region,
                mount,
                thickness,
            } => [Some(region), Some(mount), Some(thickness), None, None, None],
            BuiltinCall::Overlay {
                left,
                right,
                top,
                bottom,
                meeting,
            } => [
                left.as_ref(),
                right.as_ref(),
                top.as_ref(),
                bottom.as_ref(),
                meeting.as_ref(),
                None,
            ],
            BuiltinCall::Inset { reveal: one }
            | BuiltinCall::Turned { profile: one }
            | BuiltinCall::Joined { by: one }
            | BuiltinCall::Node { name: one }
            | BuiltinCall::Ends { inset: one }
            | BuiltinCall::Pitch { distance: one }
            | BuiltinCall::Gap { distance: one } => [Some(one), None, None, None, None, None],
            BuiltinCall::Spliced { by, at, from } => {
                [Some(by), Some(at), Some(from), None, None, None]
            }
            BuiltinCall::Tapered {
                to,
                toward,
                on,
                square,
            } => [
                Some(to),
                Some(toward),
                Some(on),
                square.as_ref(),
                None,
                None,
            ],
            BuiltinCall::Arched { rise, on, ends } => {
                [Some(rise), Some(on), ends.as_ref(), None, None, None]
            }
            BuiltinCall::Box {
                length,
                width,
                thickness,
            } => [Some(length), Some(width), Some(thickness), None, None, None],
            BuiltinCall::RegionBox { across, up, deep } => {
                [Some(across), Some(up), Some(deep), None, None, None]
            }
            BuiltinCall::Cylinder { diameter, length } => {
                [Some(diameter), Some(length), None, None, None, None]
            }
            BuiltinCall::Model {
                source,
                align,
                sha256,
            } => [
                Some(source),
                align.as_ref(),
                sha256.as_ref(),
                None,
                None,
                None,
            ],
        };
        slots.into_iter().flatten()
    }
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
    /// Its cross-section: a board's thickness or a dowel's diameter, as
    /// its form needs; `None` when that size is missing or misspelt,
    /// which has been reported.
    pub section: Option<StockSectionDef>,
    /// `max_width:`
    pub max_width: Option<RExpr>,
    /// `max_length:`
    pub max_length: Option<RExpr>,
    /// `sold:`: the thickness the boards are bought at (solid only).
    pub sold: Option<RExpr>,
    /// `kerf:`, overriding the project's.
    pub kerf: Option<RExpr>,
    /// `rough_length:`, overriding the project's (solid only).
    pub rough_length: Option<RExpr>,
    /// `rough_width:`, overriding the project's (solid only).
    pub rough_width: Option<RExpr>,
    /// `max_held_width:`: the widest a board of it may be held fast across
    /// its grain (solid only; grammar, "Grain and wood movement").
    pub max_held_width: Option<RExpr>,
}

impl StockDef {
    /// `width:`, for a board sold at a fixed width.
    pub fn fixed_width(&self) -> Option<&RExpr> {
        match &self.section {
            Some(StockSectionDef::Board { width, .. }) => width.as_ref(),
            Some(StockSectionDef::Dowel { .. }) | None => None,
        }
    }
}

/// A stock's cross-section, by its form: a board (`solid`, `plywood`,
/// `mdf`) has a thickness and perhaps a fixed width, a dowel a diameter.
#[derive(Debug, Clone, PartialEq)]
pub enum StockSectionDef {
    /// A board or sheet.
    Board {
        /// `thickness:`
        thickness: RExpr,
        /// `width:`, when it is sold at a fixed width.
        width: Option<RExpr>,
    },
    /// Round stock.
    Dowel {
        /// `diameter:`
        diameter: RExpr,
    },
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
    /// `edges:`: its chamfers and roundovers, in the order written
    /// (grammar, "Edge profiles").
    pub edges: Vec<EdgeProfileDef>,
    /// `banding:`: the edge banding on its edges (grammar, "Edge
    /// banding").
    pub banding: Option<BandingDef>,
    /// The `place { }` terms in order.
    pub place: Vec<PlaceTerm>,
    /// The `place { }` block, when written.
    pub place_span: Option<Span>,
    /// The body's braces.
    pub span: Span,
}

/// One profile of a part's `edges:`: `chamfer(3mm, on: ...)` or
/// `roundover(6mm, on: ...)` (grammar, "Edge profiles").
#[derive(Debug, Clone, PartialEq)]
pub struct EdgeProfileDef {
    /// [`CutOperation::Chamfer`] or [`CutOperation::Roundover`].
    pub operation: CutOperation,
    /// The chamfer's size or the roundover's radius.
    pub size: RExpr,
    /// The edges it shapes, each once, in the order written (`around`
    /// written out).
    pub edges: Vec<EdgeDef>,
    /// The whole call.
    pub span: Span,
}

/// A part's `banding: edge_band(1mm, on: [front, back])` (grammar, "Edge
/// banding").
#[derive(Debug, Clone, PartialEq)]
pub struct BandingDef {
    /// The band's thickness.
    pub thickness: RExpr,
    /// The faces it is glued to, as they face once the part is placed,
    /// each once, in the order written, with where each was named.
    pub faces: Vec<(Face, Span)>,
    /// The whole call.
    pub span: Span,
}

/// An edge of a part, named by the two faces that meet there, as they
/// face once the part is placed. The two faces are never on one axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EdgeDef {
    /// The two faces, as written (the face `around` names first).
    pub faces: [Face; 2],
    /// The `edge(...)` or `around(...)` it was written with.
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
    /// Its roles, in order; the first takes the positional argument.
    pub roles: Vec<RoleDef>,
    /// Its params, in order.
    pub params: Vec<ParamDef>,
    /// Its requires, checked per pair.
    pub requires: Vec<RequireDef>,
    /// The members with a body (`member arm { ... }`), in source order;
    /// a [`BodyId`] indexes this list. Empty for a fastener or tool kind.
    pub bodies: Vec<BodyDef>,
    /// What it is a kind of, with the entries only that base has.
    pub body: KindBody,
}

/// What a kind is a kind of, with the entries only that base has: a joint
/// kind has joint entries and no fields, a slide, hinge or pull kind has
/// its mount and cuts, and only a mounted kind has a mechanism.
#[derive(Debug, Clone, PartialEq)]
pub enum KindBody {
    /// A `joint` kind.
    Joint(JointKindDef),
    /// A `slide`, `hinge` or `pull` kind.
    Mounted(Box<MountedKindDef>),
    /// A `fastener` kind.
    Fastener(KindFields),
    /// A `tool` kind.
    Tool(KindFields),
}

/// A base whose kinds are mounted on a part: `slide`, `hinge` or `pull`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MountedBase {
    /// A drawer slide.
    Slide,
    /// A hinge.
    Hinge,
    /// A knob or pull.
    Pull,
    /// A shelf support.
    Support,
    /// Any other mounted fitting.
    Fitting,
}

impl MountedBase {
    /// The mounted base `base` is, or `None` for a joint, fastener or tool.
    pub fn new(base: BaseKind) -> Option<MountedBase> {
        match base {
            BaseKind::Slide => Some(MountedBase::Slide),
            BaseKind::Hinge => Some(MountedBase::Hinge),
            BaseKind::Pull => Some(MountedBase::Pull),
            BaseKind::Support => Some(MountedBase::Support),
            BaseKind::Fitting => Some(MountedBase::Fitting),
            BaseKind::Joint | BaseKind::Fastener | BaseKind::Tool => None,
        }
    }

    /// The base kind it is.
    pub fn base(self) -> BaseKind {
        match self {
            MountedBase::Slide => BaseKind::Slide,
            MountedBase::Hinge => BaseKind::Hinge,
            MountedBase::Pull => BaseKind::Pull,
            MountedBase::Support => BaseKind::Support,
            MountedBase::Fitting => BaseKind::Fitting,
        }
    }
}

/// The entries of a slide, hinge or pull kind.
#[derive(Debug, Clone, PartialEq)]
pub struct MountedKindDef {
    /// Which of the three it is.
    pub base: MountedBase,
    /// Its fields.
    pub fields: KindFields,
    /// How the bodies move, when the kind says.
    pub mechanism: Option<MechanismDef>,
    /// Where it sits and what it cuts; `None` when one of those entries
    /// is missing or did not lower, which has been reported.
    pub hardware: Option<HardwareKindDef>,
}

/// The fields of a hardware, fastener or tool kind.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct KindFields {
    /// Its fields, by name: the entries its base fixes and, for a mounted
    /// kind, any catalogue data.
    pub settings: Vec<Setting>,
    /// Where each entry its base fixes is given, found once when the kind
    /// is resolved; see [`KindDef::base_entry`].
    pub base_entries: BTreeMap<BaseField, BaseEntry>,
}

/// Where a kind gives one of the entries its base fixes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BaseEntry {
    /// A param of the kind, by its index in [`KindDef::params`]: a kind
    /// value may set it.
    Param(usize),
    /// A field of the kind, by its index in [`KindDef::fields`].
    Field(usize),
}

impl KindDef {
    /// What it is a kind of.
    pub fn base(&self) -> BaseKind {
        match &self.body {
            KindBody::Joint(_) => BaseKind::Joint,
            KindBody::Mounted(mounted) => mounted.base.base(),
            KindBody::Fastener(_) => BaseKind::Fastener,
            KindBody::Tool(_) => BaseKind::Tool,
        }
    }

    /// The joint entries, for a joint kind.
    pub fn joint(&self) -> Option<&JointKindDef> {
        match &self.body {
            KindBody::Joint(joint) => Some(joint),
            KindBody::Mounted(_) | KindBody::Fastener(_) | KindBody::Tool(_) => None,
        }
    }

    /// The entries of a slide, hinge or pull kind.
    pub fn mounted(&self) -> Option<&MountedKindDef> {
        match &self.body {
            KindBody::Mounted(mounted) => Some(mounted.as_ref()),
            KindBody::Joint(_) | KindBody::Fastener(_) | KindBody::Tool(_) => None,
        }
    }

    /// Where it sits and what it cuts, for a slide, hinge or pull kind
    /// whose entries lowered.
    pub fn hardware(&self) -> Option<&HardwareKindDef> {
        self.mounted()?.hardware.as_ref()
    }

    /// How its bodies move, for a slide, hinge or pull kind that says.
    pub fn mechanism(&self) -> Option<&MechanismDef> {
        self.mounted()?.mechanism.as_ref()
    }

    /// Its fields; a joint kind has none.
    pub fn kind_fields(&self) -> Option<&KindFields> {
        match &self.body {
            KindBody::Joint(_) => None,
            KindBody::Mounted(mounted) => Some(&mounted.fields),
            KindBody::Fastener(fields) | KindBody::Tool(fields) => Some(fields),
        }
    }

    /// Its fields, by name; empty for a joint kind.
    pub fn fields(&self) -> &[Setting] {
        self.kind_fields().map_or(&[], |fields| &fields.settings)
    }

    /// Where the kind gives a base entry (`diameter`, `species`, `model`,
    /// ...), or `None` when it gives none.
    pub fn base_entry(&self, field: BaseField) -> Option<BaseEntry> {
        self.kind_fields()?.base_entries.get(&field).copied()
    }

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
    /// `face: face(piece, toward: on)`: of the two faces across the
    /// piece's thickness, the one nearer the middle of another role's
    /// part.
    Toward {
        /// The role whose part the hardware is mounted on.
        part: RoleIndex,
        /// The `Part` role it faces.
        other: RoleIndex,
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
    /// `end(p, nearer: q)`: of the two ends of `p`'s length, the one
    /// nearer the middle of `q`.
    EndNearer {
        /// The part whose end it is.
        part: RoleIndex,
        /// The part or region whose middle decides.
        other: RoleIndex,
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
    /// `row(crossing(role, member), pitch:, up:, ...)`: the same cut made
    /// again beside each of the member's, in a hardware kind only.
    pub row: Option<RowDef>,
    /// The whole entry.
    pub span: Span,
}

/// `row(crossing(role, member), pitch: d, up: n, down: m)` (grammar,
/// "Rows of holes"): a hardware cut made again `n` times a pitch apart
/// toward each direction it names, beside the member's own, with nothing
/// in the copies: the spare holes that let an adjustable shelf move.
#[derive(Debug, Clone, PartialEq)]
pub struct RowDef {
    /// `pitch:`, from one copy's centre to the next: a length over the
    /// kind's params and roles.
    pub pitch: RExpr,
    /// How many copies toward each direction, in source order; each
    /// direction at most once.
    pub copies: Vec<RowCopies>,
    /// The whole `row(...)`.
    pub span: Span,
}

/// One direction of a [`RowDef`]: `up: holes_above`.
#[derive(Debug, Clone, PartialEq)]
pub struct RowCopies {
    /// The way the copies run from the member's own cut.
    pub toward: Direction,
    /// How many copies: a whole count over the kind's params.
    pub count: RExpr,
    /// The argument, for messages.
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
    /// `cylinder(diameter:, depth:, at:, axis:, tilt:, toward:)`
    Cylinder {
        /// Its diameter.
        diameter: RExpr,
        /// Its extent along its axis (along the slant, when it has one).
        extent: RangeDef,
        /// The point its axis runs through.
        at: Box<KindPoint>,
        /// The frame direction its axis runs along.
        axis: FrameAxis,
        /// How it is slanted, with `tilt:` and `toward:`.
        tilt: Option<CylinderTilt>,
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

/// A slanted cylinder's `tilt:` and `toward:` (grammar, "Shapes"): its
/// axis turned about `at` from the direction it runs along toward another.
#[derive(Debug, Clone, PartialEq)]
pub struct CylinderTilt {
    /// The angle it is turned by.
    pub angle: RExpr,
    /// The frame direction it leans toward; never the one it runs along.
    pub toward: FrameAxis,
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
        self.fields().iter().find(|f| f.name == name)
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
    pub target: stave_schema::operations::StepTarget,
    /// `on:` of `gauge`.
    pub on: Option<stave_schema::operations::Feature>,
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
    pub feature: stave_schema::operations::Feature,
    /// `to:`: a measurement or a feature.
    pub to: Option<stave_schema::operations::StepTarget>,
    /// `passes:`, a whole number of at least one.
    pub passes: Option<std::num::NonZeroU32>,
    /// `guide:`
    pub guide: Option<stave_schema::operations::Feature>,
    /// `diameter:` of `bore`: the measurement that names the bit.
    pub diameter: Option<stave_schema::operations::Measurement>,
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
    /// `kerf:` (grammar, "Cut list").
    pub kerf: Option<RExpr>,
    /// `rough_length:` (grammar, "Cut list").
    pub rough_length: Option<RExpr>,
    /// `rough_width:` (grammar, "Cut list").
    pub rough_width: Option<RExpr>,
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
    pub(crate) poisoned: std::collections::BTreeSet<DefId>,
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

/// Why a later stage may not read a declaration's body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unusable {
    /// It had a name or type error, already reported: what depends on it
    /// is skipped without further errors, and its body may hold
    /// [`RExprKind::Error`] expressions.
    Poisoned,
    /// No declaration has this id: a bug in Stave.
    Missing,
}

macro_rules! getters {
    ($( $method:ident, $sound:ident($id:ty) -> $def:ty, $field:ident, $variant:ident; )*) => {
        impl Defs {
            $(
                #[doc = concat!(
                    "The definition behind a [`", stringify!($id), "`], poisoned or not. ",
                    "A stage that evaluates it uses [`Defs::", stringify!($sound), "`]."
                )]
                pub fn $method(&self, id: $id) -> Option<&$def> {
                    self.$field.get(id.index())
                }

                #[doc = concat!(
                    "The definition behind a [`", stringify!($id), "`], only when it ",
                    "resolved without an error: a later stage that evaluates it can't ",
                    "be handed a poisoned one."
                )]
                pub fn $sound(&self, id: $id) -> Result<&$def, Unusable> {
                    if self.poisoned.contains(&DefId::$variant(id)) {
                        return Err(Unusable::Poisoned);
                    }
                    self.$field.get(id.index()).ok_or(Unusable::Missing)
                }
            )*
        }
    };
}

getters! {
    param, sound_param(ParamDefId) -> ParamDef, params, Param;
    let_def, sound_let_def(LetDefId) -> LetDef, lets, Let;
    region, sound_region(RegionDefId) -> RegionDef, regions, Region;
    part, sound_part(PartDefId) -> PartDef, parts, Part;
    joint, sound_joint(JointDefId) -> JointDef, joints, Joint;
    hardware_def, sound_hardware_def(HardwareDefId) -> HardwareDef, hardware, Hardware;
    assembly, sound_assembly(AssemblyDefId) -> AssemblyDef, assemblies, Assembly;
    require, sound_require(RequireDefId) -> RequireDef, requires, Require;
    stock, sound_stock(StockId) -> StockDef, stocks, Stock;
    profile, sound_profile(ProfileId) -> ProfileDef, profiles, Profile;
    kind, sound_kind(KindId) -> KindDef, kinds, Kind;
    recipe, sound_recipe(RecipeId) -> RecipeDef, recipes, Recipe;
    component, sound_component(ComponentId) -> ComponentDef, components, Component;
    scene, sound_scene(SceneDefId) -> SceneDef, scenes, Scene;
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

    /// `true` if any declaration had a name or type error.
    pub fn any_poisoned(&self) -> bool {
        !self.poisoned.is_empty()
    }

    /// The project item of the module, if it is a project module.
    pub fn project(&self, module: ModuleId) -> Option<&ProjectDef> {
        self.modules.get(module.index())?.project.as_ref()
    }
}
