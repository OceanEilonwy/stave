//! The graphical editor's contract (`docs/editing.md`): the values that
//! pass between pointing at the design, the menu worked out from the
//! declarations in scope, the check of a candidate and the card that shows
//! its verdict.
//!
//! ```text
//! pointer ──▶ Picked ──signature matching──▶ [Candidate] ──patch──▶ scratch copy ──check──▶ Verdict
//!                                                                                 └─ Refused: Problem + [Fix]
//! grip ──▶ Grip: a plain number to move (Movable), or what an expression follows (Follows)
//! ```
//!
//! These are types only: nothing here points, matches, checks or writes.
//! The lanes that build the editor implement against them, and
//! [`clothing_cabinet_fixture`] gives a plausible hand-built example to
//! build and test against until the real pipeline produces them.
//!
//! Every gesture becomes an ordinary patch of the `.stave` grammar, so a
//! [`Fix`] is a parsed [`Patch`], and diagnostics are the checker's own
//! [`Diagnostic`]s. Ids are the resolved model's ([`stave_model::ids`]).

use stave_lang::ast::{
    AssemblyDef, DesignItem, Expr, Ident, ItemKind, Name, PatchOpKind, Span, StringPart, TypeName,
};
use stave_lang::placement::{Axis, Face};
use stave_lang::units::{Angle, Length, Rational};
use stave_lang::{Code, Diagnostic, Label, SourceMap};
use stave_model::ids::{AssemblyId, ComponentId, KindId, PartId, RegionId};
use stave_model::nonempty::NonEmpty;

pub use stave_lang::ast::Patch;

/// Why a contract value can't be made: each names the invariant it would
/// break.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ContractError {
    /// Two facing parts are two different parts.
    #[error("a part can't face itself")]
    SamePart,
    /// A refusal lists errors, never warnings.
    #[error("a refusal lists errors only")]
    NotAnError,
    /// A candidate that fits has warnings at most.
    #[error("a candidate that fits has no errors")]
    ErrorInFits,
    /// A refusal has at least one error.
    #[error("a refusal needs at least one error")]
    NoProblems,
    /// A fix changes something.
    #[error("a fix needs at least one operation")]
    EmptyPatch,
    /// A grip's value lies within its range, its range runs low to high,
    /// and its stops lie within the range.
    #[error("a grip's value and stops must lie within its range")]
    OutsideRange,
    /// A grip's step is more than zero.
    #[error("a grip's step must be more than zero")]
    StepNotPositive,
}

// ---------------------------------------------------------------------------
// The selection
// ---------------------------------------------------------------------------

/// What the user points at, as typed values: the same types a component's
/// place params take (`Region`, `Anchor` with its `Part`, `Parts`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Picked {
    /// A click in a space: the smallest region that holds the point.
    Region(RegionId),
    /// A click on a face of a part: an `Anchor` and its `Part`.
    Face(PartFace),
    /// A click in the gap between two parallel faces that face each other:
    /// `Parts`, the two.
    Between(FacingPair),
    /// A group selection: `Parts`, every part of the group.
    Group(NonEmpty<PartId>),
}

impl Picked {
    /// The card's heading for this selection.
    pub fn heading(&self) -> Heading {
        match self {
            Picked::Region(_) => Heading::InThisSpace,
            Picked::Face(_) => Heading::OnThisFace,
            Picked::Between(_) => Heading::BetweenTheseParts,
            Picked::Group(_) => Heading::WithTheseParts,
        }
    }
}

/// A face of a part: the `Anchor` a click on a face gives, with the `Part`
/// it belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PartFace {
    /// The part clicked.
    pub part: PartId,
    /// Which of its faces.
    pub face: Face,
}

/// Two different parts that face each other, in the order they were
/// pointed at (the first goes to a kind's first role).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FacingPair {
    first: PartId,
    second: PartId,
}

impl FacingPair {
    /// The pair, or [`ContractError::SamePart`] when both are one part.
    pub fn new(first: PartId, second: PartId) -> Result<Self, ContractError> {
        if first == second {
            Err(ContractError::SamePart)
        } else {
            Ok(Self { first, second })
        }
    }

    /// The part pointed at first.
    pub fn first(self) -> PartId {
        self.first
    }

    /// The other part.
    pub fn second(self) -> PartId {
        self.second
    }
}

/// The card's heading, from the type of the selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Heading {
    /// A `Region`.
    InThisSpace,
    /// An `Anchor` and its `Part`.
    OnThisFace,
    /// Two facing parts.
    BetweenTheseParts,
    /// A group of parts.
    WithTheseParts,
}

impl Heading {
    /// The words shown: "In this space".
    pub fn text(self) -> &'static str {
        match self {
            Heading::InThisSpace => "In this space",
            Heading::OnThisFace => "On this face",
            Heading::BetweenTheseParts => "Between these parts",
            Heading::WithTheseParts => "With these parts",
        }
    }
}

// ---------------------------------------------------------------------------
// Candidates
// ---------------------------------------------------------------------------

/// One entry of the menu for a selection: a declaration in scope whose
/// place params the selection can fill.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    /// Which declaration it instantiates.
    pub declaration: Declaration,
    /// How the menu shows it.
    pub entry: MenuEntry,
    /// Its place params (`Region`, `Anchor`, `Part`, `Parts`) in
    /// declaration order, each bound by pointing or still to point at. For
    /// a kind, its roles.
    pub place: Vec<PlaceParam>,
    /// Its setting params in declaration order, filled in the card.
    pub settings: Vec<SettingParam>,
    /// Whether it applies here: its own `require` lines, checked in a
    /// scratch copy with its defaults.
    pub applicability: Applicability,
}

impl Candidate {
    /// The place params still to point at.
    pub fn unbound_place(&self) -> impl Iterator<Item = &PlaceParam> {
        self.place.iter().filter(|param| !param.slot.is_bound())
    }

    /// The setting params the user must still choose in the card.
    pub fn unset_settings(&self) -> impl Iterator<Item = &SettingParam> {
        self.settings
            .iter()
            .filter(|param| matches!(param.value, SettingValue::Unset))
    }

    /// `true` when every place param is bound and every setting has a
    /// value, so the candidate can be tried.
    pub fn is_complete(&self) -> bool {
        self.unbound_place().next().is_none() && self.unset_settings().next().is_none()
    }
}

/// The declaration a candidate instantiates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Declaration {
    /// A `component`: a pick writes an `assembly`.
    Component(ComponentId),
    /// A `kind`: a pick writes a `joint` or a `hardware`.
    Kind(KindId),
}

/// How a candidate appears in the menu, drawn from its declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuEntry {
    /// The declaration's name: `Drawer`.
    pub label: Ident,
    /// The first sentence of its `///` doc comment, when it has one.
    pub hint: Option<String>,
    /// The module it comes from.
    pub group: MenuGroup,
}

/// The menu group a candidate is listed in, in this order: the project's
/// own declarations, then imported modules by name, then `std`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuGroup {
    /// Declared in the project's own module.
    Project,
    /// From a module the project imports, by the name it is used through
    /// (`casework`). A standard module that is imported, such as
    /// `std:casework`, is listed here too.
    Module(Ident),
    /// From a standard module of the prelude, in scope without an import
    /// (`joinery`, `hardware`), by its name: a [`Name`], since `hardware`
    /// is a reserved word.
    Standard(Name),
}

/// A place param and what fills it. The variant is the param's type, so a
/// binding always has the type its param needs.
#[derive(Debug, Clone, PartialEq)]
pub struct PlaceParam {
    /// The param's (or role's) name: `opening`, `hinge_side`. A role's
    /// name may be any word (a slide's `in`), so this is a [`Name`].
    pub name: Name,
    /// Its type, and its value once pointed at.
    pub slot: PlaceSlot,
}

/// A place param's type and, once bound, its value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaceSlot {
    /// A `Region` param.
    Region(Option<RegionId>),
    /// An `Anchor` param, bound to a face of a part.
    Anchor(Option<PartFace>),
    /// A `Part` param.
    Part(Option<PartId>),
    /// A `Parts` param.
    Parts(Option<NonEmpty<PartId>>),
}

impl PlaceSlot {
    /// `true` once a value fills it.
    pub fn is_bound(&self) -> bool {
        match self {
            PlaceSlot::Region(value) => value.is_some(),
            PlaceSlot::Anchor(value) => value.is_some(),
            PlaceSlot::Part(value) => value.is_some(),
            PlaceSlot::Parts(value) => value.is_some(),
        }
    }
}

/// A setting param (`Stock`, `Mount`, a kind type, a length or another
/// quantity) and where its value comes from.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingParam {
    /// The param's name: `front_stock`.
    pub name: Ident,
    /// Its type.
    pub ty: TypeName,
    /// Its value, or that it has none yet.
    pub value: SettingValue,
}

/// Where a setting's value comes from.
#[derive(Debug, Clone, PartialEq)]
pub enum SettingValue {
    /// The param's own default applies, so nothing is written for it.
    Default,
    /// Copied from the project's most recent assembly of the same
    /// component, as written there.
    Copied {
        /// The assembly it was read from.
        from: AssemblyId,
        /// The expression written there.
        value: Expr,
    },
    /// Chosen by the user in the card.
    Chosen(Expr),
    /// No default and nothing to copy: the user must choose before the
    /// candidate is tried.
    Unset,
}

/// Whether a candidate applies to the selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Applicability {
    /// Not checked yet: signature matching lists it, and the scratch-copy
    /// check hasn't run.
    NotChecked,
    /// Its `require` lines hold with its defaults.
    Applies,
    /// Listed greyed, with the checker's messages saying why.
    Greyed(NonEmpty<Diagnostic>),
}

// ---------------------------------------------------------------------------
// Verdicts and fixes
// ---------------------------------------------------------------------------

/// What the check of a candidate says. There are exactly three verdicts;
/// a candidate that isn't complete is not tried, so it has none.
#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    /// The check has not finished (on a moving pointer: the fast tier found
    /// no error and the slow tier hasn't run).
    Checking,
    /// It checks with no error; Add is enabled.
    Fits(Fits),
    /// It would cause errors; nothing is written.
    Refused(Refusal),
}

/// A candidate that fits, with the warnings it has (which never block).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fits {
    warnings: Vec<Diagnostic>,
}

impl Fits {
    /// A fit with these warnings, or [`ContractError::ErrorInFits`] when
    /// one of them is an error.
    pub fn new(warnings: Vec<Diagnostic>) -> Result<Self, ContractError> {
        if warnings.iter().any(Diagnostic::is_error) {
            Err(ContractError::ErrorInFits)
        } else {
            Ok(Self { warnings })
        }
    }

    /// The warnings, shown on the card.
    pub fn warnings(&self) -> &[Diagnostic] {
        &self.warnings
    }
}

/// Why a candidate is refused: at least one error it would cause, each
/// with the checker's fixes.
#[derive(Debug, Clone, PartialEq)]
pub struct Refusal {
    problems: NonEmpty<Problem>,
}

impl Refusal {
    /// A refusal, or [`ContractError::NoProblems`] when there is nothing
    /// to refuse for.
    pub fn new(problems: Vec<Problem>) -> Result<Self, ContractError> {
        NonEmpty::new(problems)
            .map(|problems| Self { problems })
            .ok_or(ContractError::NoProblems)
    }

    /// The errors, in the checker's order.
    pub fn problems(&self) -> &NonEmpty<Problem> {
        &self.problems
    }
}

/// One error a candidate would cause, and the patches that fix it.
#[derive(Debug, Clone, PartialEq)]
pub struct Problem {
    diagnostic: Diagnostic,
    fixes: Vec<Fix>,
}

impl Problem {
    /// The problem, or [`ContractError::NotAnError`] for a warning.
    pub fn new(diagnostic: Diagnostic, fixes: Vec<Fix>) -> Result<Self, ContractError> {
        if diagnostic.is_error() {
            Ok(Self { diagnostic, fixes })
        } else {
            Err(ContractError::NotAnError)
        }
    }

    /// The error as `stave check` words it; its help line is always there.
    pub fn diagnostic(&self) -> &Diagnostic {
        &self.diagnostic
    }

    /// Its fixes, the one the help recommends first; empty when the help
    /// asks for a judgement.
    pub fn fixes(&self) -> &[Fix] {
        &self.fixes
    }
}

/// A fix: a patch that removes a diagnostic, which a tool applies as it is
/// (grammar, "Fixes"). Choosing one adds it to the candidate, which is
/// tried again.
#[derive(Debug, Clone, PartialEq)]
pub struct Fix {
    patch: Patch,
}

impl Fix {
    /// The fix, or [`ContractError::EmptyPatch`] for a patch with no
    /// operations.
    pub fn new(patch: Patch) -> Result<Self, ContractError> {
        if patch.ops.is_empty() {
            Err(ContractError::EmptyPatch)
        } else {
            Ok(Self { patch })
        }
    }

    /// The patch.
    pub fn patch(&self) -> &Patch {
        &self.patch
    }

    /// The label on the fix's button: the patch's string ("Use 18mm").
    pub fn label(&self) -> String {
        self.patch
            .summary
            .parts
            .iter()
            .filter_map(|part| match part {
                StringPart::Text { text, .. } => Some(text.as_str()),
                StringPart::Interp { .. } => None,
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Grips
// ---------------------------------------------------------------------------

/// A handle on a size in the 3D view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grip {
    /// What a drag writes to.
    pub target: GripTarget,
    /// The world axis it drags along.
    pub axis: Axis,
    /// A number to move, or what the size follows.
    pub handle: Handle,
}

/// What a grip's drag sets, as the path of a `set` patch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GripTarget {
    /// A param's default: `set overall_width = 950mm`.
    Param(Ident),
    /// A component param given in an assembly:
    /// `set apron_front.setback = 5mm`.
    AssemblyArgument {
        /// The assembly.
        assembly: AssemblyId,
        /// The component param.
        param: Ident,
    },
    /// The boundary between two neighbouring pieces of a split with
    /// `shares:`: `set drawer_bay.shares = [140, 200, 200]`. The drag moves
    /// the two shares and keeps their sum.
    Shares {
        /// The region declaration the split makes: `drawer_bay`.
        region: Ident,
        /// The index of the first of the two pieces, in label order.
        lower: usize,
    },
}

/// What a grip does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Handle {
    /// The size is a plain number: the grip moves it.
    Moves(Movable),
    /// The size is an expression: the grip shows what it follows and has
    /// no handle of its own.
    Follows(Follows),
}

/// A plain number a grip moves, of one of the types a drag writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Movable {
    /// A length: an envelope size, a gap.
    Length(GripRange<Length>),
    /// An angle.
    Angle(GripRange<Angle>),
    /// A unitless number: a share, a ratio or a count.
    Number(GripRange<Rational>),
}

/// The value a grip moves, the range it may move in and its stops. The
/// value and every stop lie within the range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GripRange<T> {
    value: T,
    low: T,
    high: T,
    stops: Stops<T>,
}

impl<T: Positive> GripRange<T> {
    /// The range, or [`ContractError::OutsideRange`] when `low > high` or
    /// the value or a stop is outside it, or
    /// [`ContractError::StepNotPositive`] for a step of zero or less.
    pub fn new(value: T, low: T, high: T, stops: Stops<T>) -> Result<Self, ContractError> {
        let inside = |x: &T| low <= *x && *x <= high;
        if !inside(&value) {
            return Err(ContractError::OutsideRange);
        }
        match &stops {
            Stops::List(list) => {
                if !list.iter().all(inside) {
                    return Err(ContractError::OutsideRange);
                }
            }
            Stops::Step(step) => {
                if !step.is_positive() {
                    return Err(ContractError::StepNotPositive);
                }
            }
        }
        Ok(Self {
            value,
            low,
            high,
            stops,
        })
    }

    /// The value now.
    pub fn value(&self) -> T {
        self.value
    }

    /// The lowest value it may take: the low end of the param's `within`.
    pub fn low(&self) -> T {
        self.low
    }

    /// The highest value it may take.
    pub fn high(&self) -> T {
        self.high
    }

    /// Where it snaps.
    pub fn stops(&self) -> &Stops<T> {
        &self.stops
    }
}

/// Where a grip snaps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stops<T> {
    /// Only these values: a `within` list.
    List(NonEmpty<T>),
    /// Every multiple of a step: the project's `resolution:` for a length.
    Step(T),
}

/// A value a grip can move: ordered, with a sign.
pub trait Positive: Copy + Ord {
    /// `true` when more than zero.
    fn is_positive(&self) -> bool;
}

impl Positive for Length {
    fn is_positive(&self) -> bool {
        *self > Length::ZERO
    }
}

impl Positive for Angle {
    fn is_positive(&self) -> bool {
        self.microdeg() > 0
    }
}

impl Positive for Rational {
    fn is_positive(&self) -> bool {
        self.numerator() > 0 && self.denominator() > 0
            || self.numerator() < 0 && self.denominator() < 0
    }
}

/// What a size written as an expression follows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Follows {
    /// The params and items it follows, nearest first: "follows
    /// `cabinet_width`".
    pub names: NonEmpty<Ident>,
    /// The plain params among them that a drag may change by single-param
    /// inversion (grammar, "Smart edits", rule 2), in the order offered.
    pub plain: Vec<Ident>,
}

// ---------------------------------------------------------------------------
// The fixture
// ---------------------------------------------------------------------------

/// A hand-built example of the contract for the clothing cabinet
/// (`examples/clothing_cabinet`): what the editor would hold after a click
/// in the space behind the left door.
#[derive(Debug, Clone, PartialEq)]
pub struct Fixture {
    /// A click in `door_bay[left]`.
    pub picked: Picked,
    /// The menu: `Drawer`, ready to try, and `FramePanelDoor`, still
    /// waiting for its hinge side to be pointed at.
    pub candidates: Vec<Candidate>,
    /// The patch the `Drawer` pick writes, which the verdict is for.
    pub tried: Patch,
    /// Its verdict: refused, because the new drawer's front runs into
    /// `door_l`, with one fix.
    pub verdict: Verdict,
    /// Grips on the result: the cabinet's width, which a drag moves, and
    /// the drawer's box height, which follows the opening.
    pub grips: Vec<Grip>,
}

/// The text the fixture parses its patches and expressions from. Its
/// diagnostic points into this text.
const FIXTURE_TEXT: &str = r#"stave 1

patch "Add a drawer" {
  add assembly drawer_2 : casework.Drawer {
    opening:      door_bay[left]
    mount:        overlay(left: 18mm, right: 18mm, top: 8mm, bottom: 8mm, meeting: -reveal / 2)
    front_stock:  oak_19
    box_stock:    maple_12
    bottom_stock: ply_6
    slide:        side_mount_slide(length: slide_length)
  }
}

patch "Remove door_l and door_r" {
  remove assembly door_r
  remove assembly door_l
}

patch "Settings copied from door_l" {
  set door_l.mount = overlay(left: 18mm, right: 18mm, top: -reveal, bottom: 8mm, meeting: -reveal / 2)
  set door_l.frame = oak_19
  set door_l.panel_stock = oak_6
}
"#;

/// Builds the [`Fixture`]. Its ids are illustrative indexes, not those of
/// the opened example; its names, expressions and patches are the
/// cabinet's. `None` only if the fixture's own text stops parsing.
pub fn clothing_cabinet_fixture() -> Option<Fixture> {
    let mut sources = SourceMap::new();
    let file = sources
        .add("clothing_cabinet_fixture.stave", FIXTURE_TEXT)
        .ok()?;
    let parsed = stave_lang::parse_patch_file(file, FIXTURE_TEXT);
    if parsed.has_errors() {
        return None;
    }
    let mut patches = parsed.ast?.patches.into_iter();
    let (add_drawer, remove_doors, door_settings) =
        (patches.next()?, patches.next()?, patches.next()?);

    let name = |text: &str| Ident::new(text, Span::default()).ok();
    let word = |text: &str| Name::new(text, Span::default()).ok();
    let set_value = |patch: &Patch, index: usize| match &patch.ops.get(index)?.kind {
        PatchOpKind::Set { value, .. } => Some(value.clone()),
        _ => None,
    };
    let assembly_value = |field: &str| -> Option<Expr> {
        let PatchOpKind::Add { item, .. } = &add_drawer.ops.first()?.kind else {
            return None;
        };
        let ItemKind::Design(DesignItem::Assembly(assembly)) = &item.kind else {
            return None;
        };
        let AssemblyDef::Instance { body, .. } = &assembly.def else {
            return None;
        };
        body.field(field).map(|f| f.value.clone())
    };

    let door_bay_left = RegionId::from_index(7)?;
    let drawer_top = AssemblyId::from_index(1)?;
    let door_l = AssemblyId::from_index(2)?;
    let casework = name("casework")?;
    let setting = |param: &str, ty: TypeName, value: SettingValue| -> Option<SettingParam> {
        Some(SettingParam {
            name: name(param)?,
            ty,
            value,
        })
    };
    let copied = |from: AssemblyId, value: Option<Expr>| -> Option<SettingValue> {
        Some(SettingValue::Copied {
            from,
            value: value?,
        })
    };

    let drawer = Candidate {
        declaration: Declaration::Component(ComponentId::from_index(1)?),
        entry: MenuEntry {
            label: name("Drawer")?,
            hint: Some(
                "A drawer on side-mount slides: an applied front on a box with a grooved-in bottom."
                    .to_owned(),
            ),
            group: MenuGroup::Module(casework.clone()),
        },
        place: vec![PlaceParam {
            name: word("opening")?,
            slot: PlaceSlot::Region(Some(door_bay_left)),
        }],
        settings: vec![
            setting(
                "mount",
                TypeName::Mount,
                copied(drawer_top, assembly_value("mount"))?,
            )?,
            setting(
                "front_stock",
                TypeName::Stock,
                copied(drawer_top, assembly_value("front_stock"))?,
            )?,
            setting(
                "box_stock",
                TypeName::Stock,
                copied(drawer_top, assembly_value("box_stock"))?,
            )?,
            setting(
                "bottom_stock",
                TypeName::Stock,
                copied(drawer_top, assembly_value("bottom_stock"))?,
            )?,
            setting(
                "slide",
                TypeName::SlideKind,
                copied(drawer_top, assembly_value("slide"))?,
            )?,
            setting("pull_kind", TypeName::PullKind, SettingValue::Default)?,
            setting("bottom_gap", TypeName::Length, SettingValue::Default)?,
            setting("top_gap", TypeName::Length, SettingValue::Default)?,
            setting("box_height", TypeName::Length, SettingValue::Default)?,
        ],
        applicability: Applicability::Applies,
    };

    let door = Candidate {
        declaration: Declaration::Component(ComponentId::from_index(0)?),
        entry: MenuEntry {
            label: name("FramePanelDoor")?,
            hint: Some(
                "A frame-and-panel door: two stiles, two rails and a floating panel.".to_owned(),
            ),
            group: MenuGroup::Module(casework),
        },
        place: vec![
            PlaceParam {
                name: word("opening")?,
                slot: PlaceSlot::Region(Some(door_bay_left)),
            },
            PlaceParam {
                name: word("hinge_side")?,
                slot: PlaceSlot::Part(None),
            },
        ],
        settings: vec![
            setting(
                "mount",
                TypeName::Mount,
                copied(door_l, set_value(&door_settings, 0))?,
            )?,
            setting(
                "frame",
                TypeName::Stock,
                copied(door_l, set_value(&door_settings, 1))?,
            )?,
            setting(
                "panel_stock",
                TypeName::Stock,
                copied(door_l, set_value(&door_settings, 2))?,
            )?,
            setting("stile_width", TypeName::Length, SettingValue::Default)?,
            setting("rail_width", TypeName::Length, SettingValue::Default)?,
            setting("joinery", TypeName::JointKind, SettingValue::Default)?,
            setting("hinge_kind", TypeName::HingeKind, SettingValue::Default)?,
            setting("knob_kind", TypeName::PullKind, SettingValue::Default)?,
        ],
        applicability: Applicability::Applies,
    };

    let at = FIXTURE_TEXT.find("drawer_2")?;
    let span = Span::new(
        u32::try_from(at).ok()?,
        u32::try_from(at + "drawer_2".len()).ok()?,
    );
    let overlap = Diagnostic::error(
        Code::Overlap,
        "drawer_2 and door_l overlap",
        Label::new(file, span),
    )
    .with_help("door_l covers the front of door_bay[left]; put the drawer in another space, or remove the doors");
    let verdict = Verdict::Refused(
        Refusal::new(vec![
            Problem::new(overlap, vec![Fix::new(remove_doors).ok()?]).ok()?,
        ])
        .ok()?,
    );

    let millimetres = Length::from_mm;
    let width = GripRange::new(
        millimetres(900),
        millimetres(600),
        millimetres(1200),
        Stops::Step(millimetres(1)),
    )
    .ok()?;
    let grips = vec![
        Grip {
            target: GripTarget::Param(name("cabinet_width")?),
            axis: Axis::Across,
            handle: Handle::Moves(Movable::Length(width)),
        },
        Grip {
            target: GripTarget::AssemblyArgument {
                assembly: drawer_top,
                param: name("box_height")?,
            },
            axis: Axis::Up,
            handle: Handle::Follows(Follows {
                names: NonEmpty::new(vec![
                    name("drawer_bay")?,
                    name("drawer_zone")?,
                    name("bottom_gap")?,
                    name("top_gap")?,
                ])?,
                plain: vec![name("drawer_zone")?],
            }),
        },
    ];

    Some(Fixture {
        picked: Picked::Region(door_bay_left),
        candidates: vec![drawer, door],
        tried: add_drawer,
        verdict,
        grips,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cabinet_fixture_is_a_plausible_menu_and_verdict() {
        let fixture = clothing_cabinet_fixture().expect("the fixture's text parses");
        assert_eq!(fixture.picked.heading().text(), "In this space");

        let [drawer, door] = fixture.candidates.as_slice() else {
            panic!("two candidates");
        };
        assert_eq!(drawer.entry.label.as_str(), "Drawer");
        assert!(drawer.is_complete());
        assert_eq!(door.entry.label.as_str(), "FramePanelDoor");
        let unbound: Vec<&str> = door.unbound_place().map(|p| p.name.as_str()).collect();
        assert_eq!(unbound, ["hinge_side"]);
        assert_eq!(door.unset_settings().count(), 0);
        assert!(!door.is_complete());

        assert_eq!(fixture.tried.ops.len(), 1);
        let Verdict::Refused(refusal) = &fixture.verdict else {
            panic!("refused");
        };
        let problem = refusal.problems().first();
        assert_eq!(problem.diagnostic().code, Code::Overlap);
        assert!(problem.diagnostic().help.is_some());
        let [fix] = problem.fixes() else {
            panic!("one fix");
        };
        assert_eq!(fix.label(), "Remove door_l and door_r");
        assert_eq!(fix.patch().ops.len(), 2);

        let [width, box_height] = fixture.grips.as_slice() else {
            panic!("two grips");
        };
        let Handle::Moves(Movable::Length(range)) = &width.handle else {
            panic!("the width moves");
        };
        assert_eq!(range.value(), Length::from_mm(900));
        let Handle::Follows(follows) = &box_height.handle else {
            panic!("the box height follows");
        };
        assert_eq!(follows.names.first().as_str(), "drawer_bay");
    }

    #[test]
    fn contract_values_keep_their_invariants() {
        let part = |index| PartId::from_index(index).unwrap();
        assert_eq!(
            FacingPair::new(part(3), part(3)),
            Err(ContractError::SamePart)
        );
        assert!(FacingPair::new(part(3), part(4)).is_ok());
        assert_eq!(Refusal::new(Vec::new()), Err(ContractError::NoProblems));
        let mm = Length::from_mm;
        assert_eq!(
            GripRange::new(mm(5), mm(10), mm(20), Stops::Step(mm(1))),
            Err(ContractError::OutsideRange)
        );
        assert_eq!(
            GripRange::new(mm(15), mm(10), mm(20), Stops::Step(mm(0))),
            Err(ContractError::StepNotPositive)
        );
        let share = |n| Rational::from_int(n);
        assert!(GripRange::new(share(1), share(1), share(3), Stops::Step(share(1))).is_ok());
    }
}
