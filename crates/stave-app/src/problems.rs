//! The problems list and a problem's details, worked out without any UI.
//!
//! The list is the parts tree showing only the parts with problems
//! (Problems.dc.html): under each part, one compact row per kind of
//! problem ([`Category`]) with how many there are and where they are
//! written (`Motion (2)  project.stave[69], project.stave[80]`). The parts
//! a problem is about are listed with it; the parts it only involves (the
//! side a door hits) carry the same row, marked "involved". Problems that
//! aren't about a part (a misspelt name, a requirement) are listed after
//! the tree.
//!
//! A problem's details give a plain-English title, the compiler-style text
//! exactly as `stave check` prints it, the ways to fix it (from its help
//! line), and, for a door or drawer that hits something, what the motion
//! card needs: where along its travel it is blocked.

use crate::i18n::{fill_in, ntr, tr};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use stave_geom::render;
use stave_lang::units::{Angle, Length};
use stave_lang::{Code, Diagnostic, Severity, place_label, render_diagnostic};
use stave_model::nonempty::NonEmpty;
use stave_model::solved::{AssemblyId, SolvedDesign};
use stave_scene::{Item, MotionRange};

use crate::check::Checked;
use crate::inspect::{self, AssemblyNode, PartsTree, TreeEntry};
use crate::session::{LineNumber, ProblemIndex, Selection, SourceLocation};
use crate::{Contact, ContactItem};

/// What kind of problem a diagnostic is, for its icon and its row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Category {
    /// A door or drawer hits something as it moves (`motion`).
    Motion,
    /// Wood passes through wood (`overlap`).
    Overlap,
    /// A joint's parts don't meet (`no-contact`), or a part nothing joins
    /// to the rest (`loose-part`).
    NoContact,
    /// A piece of hardware can't be set within its adjustment to fit
    /// where it is placed (`adjust`).
    Adjust,
    /// A piece of hardware's moving parts can't work as its kind declares
    /// them (`mechanism`).
    Mechanism,
    /// A joint can't be put together in the order its steps say (`order`).
    Order,
    /// A `require` doesn't hold (`require`, `require-range`, `range`).
    Requirement,
    /// A part's position or size isn't settled (`axis-not-set`, `ground`,
    /// ...).
    Placement,
    /// The text can't be read (`syntax`, `missing-brace`, ...).
    Syntax,
    /// A name is unknown, taken twice or unclear (`unknown-name`, ...).
    Name,
    /// A value is the wrong kind, unit or size (`type-mismatch`, ...).
    Value,
    /// A cut no recipe can make with the project's tools (`no-recipe`), a
    /// kind's cut with no name (`unnamed-cut`), a cut that leaves no wood
    /// round it (`wall`), one bigger than its part (`cut-size`), or wood
    /// joined or held against its grain (`grain`).
    Cut,
    /// A hardware model doesn't match its envelope (`model-mismatch`).
    Hardware,
    /// Something Stave can't do yet (`not-supported`), or a failure inside
    /// Stave itself (`internal`).
    NotSupported,
}

impl Category {
    /// The category of a diagnostic code.
    pub fn of(code: Code) -> Category {
        match code {
            Code::Motion => Category::Motion,
            Code::Overlap => Category::Overlap,
            Code::NoContact | Code::LoosePart => Category::NoContact,
            Code::Adjust => Category::Adjust,
            Code::Mechanism => Category::Mechanism,
            Code::Order => Category::Order,
            Code::Require | Code::RequireRange | Code::Range => Category::Requirement,
            Code::Ground
            | Code::AxisNotSet
            | Code::AxisSetTwice
            | Code::PlacementCycle
            | Code::FitUndecided
            | Code::Mirror => Category::Placement,
            Code::Syntax
            | Code::MissingValue
            | Code::ReservedWord
            | Code::UnknownUnit
            | Code::UnclosedBracket
            | Code::MissingBrace => Category::Syntax,
            Code::UnknownName
            | Code::DuplicateName
            | Code::AmbiguousName
            | Code::UnknownModule
            | Code::ImportCycle
            | Code::NotInLibrary
            | Code::ShadowsPrelude => Category::Name,
            Code::UnitMismatch
            | Code::TypeMismatch
            | Code::UnknownField
            | Code::MissingField
            | Code::OutOfRange => Category::Value,
            Code::NoRecipe | Code::UnnamedCut | Code::Wall | Code::CutSize | Code::Grain => {
                Category::Cut
            }
            Code::ModelMismatch => Category::Hardware,
            Code::NotSupported | Code::Internal => Category::NotSupported,
        }
    }

    /// Its name on a row: "Motion", "No contact".
    pub fn title(self) -> String {
        match self {
            Category::Motion => tr("Motion"),
            Category::Overlap => tr("Overlap"),
            Category::NoContact => tr("No contact"),
            Category::Adjust => tr("Adjustment"),
            Category::Mechanism => tr("Mechanism"),
            Category::Order => tr("Order"),
            Category::Requirement => tr("Requirement"),
            Category::Placement => tr("Placement"),
            Category::Syntax => tr("Syntax"),
            Category::Name => tr("Name"),
            Category::Value => tr("Value"),
            Category::Cut => tr("Cut"),
            Category::Hardware => tr("Hardware"),
            Category::NotSupported => tr("Not supported yet"),
        }
    }

    /// A plain-English title for a problem of this kind, when nothing more
    /// specific can be said.
    fn plain_title(self) -> String {
        match self {
            Category::Motion => tr("A door or drawer hits something as it moves"),
            Category::Overlap => tr("Wood passes through wood"),
            Category::NoContact => tr("The joint's parts don't meet"),
            Category::Adjust => tr("The hardware can't be adjusted to fit"),
            Category::Mechanism => tr("The hardware's moving parts can't work as written"),
            Category::Order => tr("The joint can't be put together in this order"),
            Category::Requirement => tr("A requirement doesn't hold"),
            Category::Placement => tr("Where a part goes isn't settled"),
            Category::Syntax => tr("Stave can't read the text here"),
            Category::Name => tr("A name isn't right"),
            Category::Value => tr("A value is the wrong kind or size"),
            Category::Cut => tr("A cut can't be made with your tools"),
            Category::Hardware => tr("The hardware doesn't match its size"),
            Category::NotSupported => tr("Stave can't do this yet"),
        }
    }
}

/// Which problems the list shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Filter {
    /// Every problem.
    #[default]
    All,
    /// Errors only.
    Errors,
    /// Warnings only.
    Warnings,
}

impl Filter {
    /// Whether a problem of this severity is shown.
    pub fn shows(self, severity: Severity) -> bool {
        match self {
            Filter::All => true,
            Filter::Errors => severity == Severity::Error,
            Filter::Warnings => severity == Severity::Warning,
        }
    }
}

/// How many problems each filter shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Counts {
    /// Every problem.
    pub all: usize,
    /// Errors.
    pub errors: usize,
    /// Warnings.
    pub warnings: usize,
}

/// A place a problem is written: `project.stave[69]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    /// `project.stave[69]`: the file relative to the project folder.
    pub label: String,
    /// The file and line, for "Show in text".
    pub location: SourceLocation,
}

/// One row of the list: the problems of one kind about one part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProblemRow {
    /// Their kind.
    pub category: Category,
    /// The most serious of them.
    pub severity: Severity,
    /// Which problems, in `stave check` order; choosing the row selects
    /// the first.
    pub problems: NonEmpty<ProblemIndex>,
    /// Where they are written, each place once, in order.
    pub places: Vec<Place>,
    /// Every message, one per line, for a tooltip.
    pub messages: String,
}

impl ProblemRow {
    /// How many problems it stands for.
    pub fn count(&self) -> usize {
        self.problems.len()
    }

    /// "Motion", or "Overlap (2)" when it stands for more than one.
    pub fn title(&self) -> String {
        match self.count() {
            1 => self.category.title(),
            count => fill_in(
                &tr("{category} ({count})"),
                &[("category", &self.category.title()), ("count", &count)],
            ),
        }
    }

    /// "project.stave[69], project.stave[80]".
    pub fn places_text(&self) -> String {
        self.places
            .iter()
            .map(|place| place.label.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Whether the selection is one of its problems.
    pub fn holds(&self, selection: Option<&Selection>) -> bool {
        match selection {
            Some(Selection::Problem(index)) => self.problems.iter().any(|p| p == index),
            Some(_) | None => false,
        }
    }

    fn add(
        &mut self,
        index: ProblemIndex,
        severity: Severity,
        place: Option<Place>,
        message: &str,
    ) {
        if self.problems.iter().any(|p| *p == index) {
            return;
        }
        self.problems.push(index);
        self.severity = self.severity.min(severity);
        if let Some(place) = place
            && !self.places.contains(&place)
        {
            self.places.push(place);
        }
        self.messages.push('\n');
        self.messages.push_str(message);
    }
}

/// Adds a problem to the row of its kind, or starts one.
fn add_to_rows(
    rows: &mut Vec<ProblemRow>,
    category: Category,
    index: ProblemIndex,
    severity: Severity,
    place: Option<Place>,
    message: &str,
) {
    match rows.iter_mut().find(|row| row.category == category) {
        Some(row) => row.add(index, severity, place, message),
        None => rows.push(ProblemRow {
            category,
            severity,
            problems: NonEmpty::one(index),
            places: place.into_iter().collect(),
            messages: message.to_owned(),
        }),
    }
}

/// A part (or a group of the same parts) with problems.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProblemEntry {
    /// The parts tree's row: its label, swatch and items.
    pub entry: TreeEntry,
    /// How many of the problems are about it (not only involving it): the
    /// count on its badge. 0 when it is only involved.
    pub about: usize,
    /// The most serious problem it is about, for the badge's colour.
    pub severity: Option<Severity>,
    /// Its problems, one row per kind.
    pub rows: Vec<ProblemRow>,
}

impl ProblemEntry {
    /// Whether it is only involved in its problems (the side a door
    /// hits): it shows "involved" instead of a count.
    pub fn involved(&self) -> bool {
        self.about == 0
    }
}

/// An assembly holding parts with problems.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProblemAssembly {
    /// The assembly.
    pub id: AssemblyId,
    /// Its name: `door_l`.
    pub label: String,
    /// "turns 110°", as in the parts tree.
    pub detail: Option<String>,
    /// Its parts with problems.
    pub entries: Vec<ProblemEntry>,
    /// The assemblies inside it holding parts with problems.
    pub children: Vec<ProblemAssembly>,
}

/// The problems list: the parts tree showing only parts with problems,
/// then the problems that aren't about a part.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProblemsTree {
    /// How many problems each filter shows.
    pub counts: Counts,
    /// The carcass's parts with problems.
    pub carcass: Vec<ProblemEntry>,
    /// Assemblies holding parts with problems.
    pub assemblies: Vec<ProblemAssembly>,
    /// Problems not about a part, one row per kind.
    pub elsewhere: Vec<ProblemRow>,
}

impl ProblemsTree {
    /// Whether the filter shows no problem at all.
    pub fn is_empty(&self) -> bool {
        self.carcass.is_empty() && self.assemblies.is_empty() && self.elsewhere.is_empty()
    }
}

/// Where a diagnostic is written.
fn place_of(checked: &Checked, diagnostic: &Diagnostic) -> Option<Place> {
    let label = diagnostic.primary()?;
    let file = checked.opened.sources.get(label.file)?;
    let line = LineNumber::from_usize(file.line_column(label.span.start()).line)?;
    let path = Path::new(file.path());
    let shown = path.strip_prefix(&checked.folder).unwrap_or(path);
    Some(Place {
        label: format!("{}[{line}]", shown.display()),
        location: SourceLocation {
            path: file.path().to_owned(),
            line,
        },
    })
}

fn item_of(contact_item: ContactItem) -> Item {
    crate::view::contact_item(contact_item)
}

/// Whether an item is inside an assembly (at any depth).
pub(crate) fn inside(design: &SolvedDesign, item: Item, assembly: AssemblyId) -> bool {
    let mut current = match item {
        Item::Part(id) => design.part(id).and_then(|p| p.assembly),
        Item::Hardware(id) => design.hardware_piece(id).and_then(|h| h.assembly),
        Item::Member(member) => design
            .pair_member(member)
            .and_then(|body| design.part(body.carried_by()?))
            .and_then(|p| p.assembly),
    };
    let mut steps = 0;
    while let Some(id) = current {
        if id == assembly {
            return true;
        }
        steps += 1;
        if steps > design.assemblies().len() {
            return false;
        }
        current = design.assembly(id).and_then(|a| a.parent);
    }
    false
}

/// The items a problem is about, and the ones it only involves. A motion
/// is about what moves; an overlap is about every part in it; the other
/// checks are about their first item (the part that doesn't meet, the
/// drawer side that doesn't fit).
fn about_and_involved(
    design: &SolvedDesign,
    category: Category,
    contact: &Contact,
) -> (Vec<Item>, Vec<Item>) {
    let items: Vec<Item> = contact.items.iter().copied().map(item_of).collect();
    let first_only = |items: &[Item]| -> (Vec<Item>, Vec<Item>) {
        match items.split_first() {
            Some((first, rest)) => (vec![*first], rest.to_vec()),
            None => (Vec::new(), Vec::new()),
        }
    };
    let (about, involved): (Vec<Item>, Vec<Item>) = match category {
        Category::Motion => match contact.motion {
            Some(motion) => items
                .iter()
                .partition(|item| inside(design, **item, motion.assembly)),
            None => first_only(&items),
        },
        Category::Overlap => (items, Vec::new()),
        Category::NoContact
        | Category::Adjust
        | Category::Mechanism
        | Category::Order
        | Category::Requirement
        | Category::Placement
        | Category::Syntax
        | Category::Name
        | Category::Value
        | Category::Cut
        | Category::Hardware
        | Category::NotSupported => first_only(&items),
    };
    if about.is_empty() {
        // Nothing moves inside it: treat everything as what it is about.
        (involved, Vec::new())
    } else {
        (about, involved)
    }
}

// ---------------------------------------------------------------------------
// Marks: the dots on the model tree's rows and the open files' tabs
// ---------------------------------------------------------------------------

/// How a row of the model tree, or a file, is marked for its problems.
/// Ordered worst first, so the smaller of two marks is the one to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Mark {
    /// At least one error is about it: a red dot.
    Error,
    /// At least one warning is about it, and no error: an amber dot.
    Warning,
    /// A problem involves it without being about it (the side a door
    /// hits): a red ring.
    Involved,
}

impl Mark {
    /// The mark a problem of this severity gives what it is about; notes
    /// mark nothing.
    pub fn of(severity: Severity) -> Option<Mark> {
        match severity {
            Severity::Error => Some(Mark::Error),
            Severity::Warning => Some(Mark::Warning),
            Severity::Note => None,
        }
    }

    /// The worse of two marks.
    pub fn worse(self, other: Mark) -> Mark {
        self.min(other)
    }
}

/// Every mark of one check's result: by part or piece of hardware, by
/// assembly (the worst inside it, at any depth) and by file (the worst
/// problem written in it).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Marks {
    items: BTreeMap<Item, Mark>,
    assemblies: BTreeMap<AssemblyId, Mark>,
    files: BTreeMap<String, Mark>,
}

impl Marks {
    /// A part's or a piece of hardware's mark.
    pub fn item(&self, item: Item) -> Option<Mark> {
        self.items.get(&item).copied()
    }

    /// An assembly's mark: the worst of everything inside it.
    pub fn assembly(&self, assembly: AssemblyId) -> Option<Mark> {
        self.assemblies.get(&assembly).copied()
    }

    /// A file's mark, by its path as the source map names it.
    pub fn file(&self, path: &str) -> Option<Mark> {
        self.files.get(path).copied()
    }

    /// A tree row's mark: the worst of the items it stands for.
    pub fn entry(&self, entry: &TreeEntry) -> Option<Mark> {
        entry
            .items
            .iter()
            .filter_map(|item| self.item(*item))
            .reduce(Mark::worse)
    }

    fn add<K: Ord>(map: &mut BTreeMap<K, Mark>, key: K, mark: Mark) {
        map.entry(key)
            .and_modify(|old| *old = old.worse(mark))
            .or_insert(mark);
    }
}

/// The marks of a check's result. A problem marks what it is about with
/// its severity, what it only involves with [`Mark::Involved`], every
/// assembly round those, and the file it is written in. Items are those
/// of the result's own design, so they match its ids.
pub fn marks(checked: &Checked) -> Marks {
    let mut marks = Marks::default();
    for (problem, diagnostic) in checked.problems.iter().zip(&checked.opened.diagnostics) {
        let Some(severity_mark) = Mark::of(problem.severity) else {
            continue;
        };
        if let Some(path) = diagnostic
            .primary()
            .and_then(|label| checked.opened.sources.get(label.file))
            .map(|file| file.path().to_owned())
        {
            Marks::add(&mut marks.files, path, severity_mark);
        }
        let (Some(design), Some(contact)) = (checked.design(), &problem.contact) else {
            continue;
        };
        let (about, involved) = about_and_involved(design, Category::of(diagnostic.code), contact);
        for (items, mark) in [(&about, severity_mark), (&involved, Mark::Involved)] {
            for item in items {
                Marks::add(&mut marks.items, *item, mark);
                for assembly in design.assemblies() {
                    if inside(design, *item, assembly.id) {
                        Marks::add(&mut marks.assemblies, assembly.id, mark);
                    }
                }
            }
        }
    }
    marks
}

/// A part of the tree being filled: an entry and the problems found so far.
struct Filling {
    rows: Vec<ProblemRow>,
    about: BTreeSet<ProblemIndex>,
    severity: Option<Severity>,
}

/// Builds the problems list for a check's result. The tree is the
/// result's own design's parts tree, so it matches the problems' ids;
/// when the design didn't solve, every problem is listed after the tree.
pub fn tree(checked: &Checked, filter: Filter) -> ProblemsTree {
    let mut counts = Counts::default();
    for problem in &checked.problems {
        counts.all += 1;
        match problem.severity {
            Severity::Error => counts.errors += 1,
            Severity::Warning => counts.warnings += 1,
            Severity::Note => {}
        }
    }
    let parts_tree = checked
        .design()
        .map(inspect::parts_tree)
        .unwrap_or_default();
    let entries = flatten(&parts_tree);
    let mut filling: Vec<Filling> = entries
        .iter()
        .map(|_| Filling {
            rows: Vec::new(),
            about: BTreeSet::new(),
            severity: None,
        })
        .collect();
    let mut elsewhere = Vec::new();
    for (position, (problem, diagnostic)) in checked
        .problems
        .iter()
        .zip(&checked.opened.diagnostics)
        .enumerate()
    {
        if !filter.shows(problem.severity) {
            continue;
        }
        let index = ProblemIndex(position);
        let category = Category::of(diagnostic.code);
        let place = place_of(checked, diagnostic);
        let (about, involved) = match (checked.design(), &problem.contact) {
            (Some(design), Some(contact)) => about_and_involved(design, category, contact),
            _ => (Vec::new(), Vec::new()),
        };
        let mut listed = false;
        for (items, is_about) in [(&about, true), (&involved, false)] {
            for item in items {
                let Some(found) = entries
                    .iter()
                    .position(|entry| entry.items.iter().any(|i| i == item))
                else {
                    continue;
                };
                let Some(slot) = filling.get_mut(found) else {
                    continue;
                };
                add_to_rows(
                    &mut slot.rows,
                    category,
                    index,
                    problem.severity,
                    place.clone(),
                    &problem.message,
                );
                if is_about {
                    slot.about.insert(index);
                    slot.severity = Some(
                        slot.severity
                            .map_or(problem.severity, |s| s.min(problem.severity)),
                    );
                }
                listed = true;
            }
        }
        if !listed {
            add_to_rows(
                &mut elsewhere,
                category,
                index,
                problem.severity,
                place,
                &problem.message,
            );
        }
    }
    let mut filled = entries.into_iter().zip(filling).map(|(entry, filling)| {
        (!filling.rows.is_empty()).then(|| ProblemEntry {
            entry: entry.clone(),
            about: filling.about.len(),
            severity: filling.severity,
            rows: filling.rows,
        })
    });
    // `flatten` lists the carcass, then each assembly depth first, in
    // the same order `rebuild` takes them.
    let carcass = parts_tree
        .carcass
        .iter()
        .filter_map(|_| filled.next().flatten())
        .collect();
    let assemblies = parts_tree
        .assemblies
        .iter()
        .filter_map(|node| rebuild(node, &mut filled))
        .collect();
    ProblemsTree {
        counts,
        carcass,
        assemblies,
        elsewhere,
    }
}

/// Every entry of the parts tree: the carcass, then each assembly's own
/// entries before its children's, depth first.
fn flatten(tree: &PartsTree) -> Vec<&TreeEntry> {
    fn walk<'t>(node: &'t AssemblyNode, out: &mut Vec<&'t TreeEntry>) {
        out.extend(node.entries.iter());
        for child in &node.children {
            walk(child, out);
        }
    }
    let mut out: Vec<&TreeEntry> = tree.carcass.iter().collect();
    for node in &tree.assemblies {
        walk(node, &mut out);
    }
    out
}

/// An assembly with only its entries that have problems, taking them from
/// `filled` in [`flatten`]'s order; `None` when nothing in it has any.
fn rebuild(
    node: &AssemblyNode,
    filled: &mut impl Iterator<Item = Option<ProblemEntry>>,
) -> Option<ProblemAssembly> {
    let entries: Vec<ProblemEntry> = node
        .entries
        .iter()
        .filter_map(|_| filled.next().flatten())
        .collect();
    let children: Vec<ProblemAssembly> = node
        .children
        .iter()
        .filter_map(|child| rebuild(child, filled))
        .collect();
    (!entries.is_empty() || !children.is_empty()).then(|| ProblemAssembly {
        id: node.id,
        label: node.label.clone(),
        detail: node.detail.clone(),
        entries,
        children,
    })
}

/// One way to fix a problem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fix {
    /// What to do: "Leave at least 1mm between door and wall".
    pub text: String,
    /// Where to do it in the text, for "Show in text".
    pub location: Option<SourceLocation>,
}

/// Where a door or drawer that hits something is blocked, for the motion
/// card ("Swing door to see where it hits").
#[derive(Debug, Clone, PartialEq)]
pub struct MotionCard {
    /// The assembly that moves.
    pub assembly: AssemblyId,
    /// Its name: `door`.
    pub name: String,
    /// How far it moves, where it first touches and where the shared
    /// wood is measured, exact, in the unit of its motion.
    pub travel: MotionTravel,
    /// "closed 0°".
    pub start: String,
    /// "blocked from 0.6°".
    pub blocked: String,
    /// "opens to 110°".
    pub end: String,
}

/// The amounts on a motion card, exact. The variant carries the unit, so a
/// door's angles can't be read as a drawer's lengths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotionTravel {
    /// A door that swings.
    Swing {
        /// Fully open.
        full: Angle,
        /// Where it first touches.
        contact: Angle,
        /// Where the shared wood is measured, a little past the contact.
        shown: Angle,
    },
    /// A drawer that slides.
    Slide {
        /// Fully out.
        full: Length,
        /// Where it first touches.
        contact: Length,
        /// Where the shared wood is measured, a little past the contact.
        shown: Length,
    },
}

impl MotionCard {
    /// `true` for a door, whose render amounts are radians; `false` for a
    /// drawer, whose render amounts are millimetres.
    pub fn turns(&self) -> bool {
        match self.travel {
            MotionTravel::Swing { .. } => true,
            MotionTravel::Slide { .. } => false,
        }
    }

    /// Fully open, as a render amount (radians or millimetres).
    pub fn full(&self) -> f32 {
        match self.travel {
            MotionTravel::Swing { full, .. } => render::radians(full),
            MotionTravel::Slide { full, .. } => render::mm(full),
        }
    }

    /// Where it first touches, as a render amount (radians or
    /// millimetres).
    pub fn contact(&self) -> f32 {
        match self.travel {
            MotionTravel::Swing { contact, .. } => render::radians(contact),
            MotionTravel::Slide { contact, .. } => render::mm(contact),
        }
    }

    /// Where the shared wood is measured, as a render amount (radians or
    /// millimetres).
    pub fn shown(&self) -> f32 {
        match self.travel {
            MotionTravel::Swing { shown, .. } => render::radians(shown),
            MotionTravel::Slide { shown, .. } => render::mm(shown),
        }
    }

    /// The card's lead: "Swing door", "Slide drawer[top]".
    pub fn lead(&self) -> String {
        let template = if self.turns() {
            tr("Swing {name}")
        } else {
            tr("Slide {name}")
        };
        fill_in(&template, &[("name", &self.name)])
    }

    /// A render amount (radians for a door, millimetres for a drawer, as
    /// the scrubber gives) as the card writes it: `90.1°`, `312 mm`.
    pub fn amount_text(&self, amount: f32) -> String {
        if self.turns() {
            format!("{}°", tenths(f64::from(amount).to_degrees()))
        } else {
            format!("{} mm", tenths(f64::from(amount)))
        }
    }

    /// An exact amount on this card as the card writes it.
    fn exact_text(amount: MotionAmount) -> String {
        match amount {
            MotionAmount::Angle(angle) => {
                format!("{}°", tenths(angle.to_radians_f64().to_degrees()))
            }
            MotionAmount::Length(length) => format!("{} mm", tenths(length.to_mm_f64())),
        }
    }
}

/// One exact amount of a motion card, for its words.
#[derive(Debug, Clone, Copy)]
enum MotionAmount {
    Angle(Angle),
    Length(Length),
}

/// A number to the nearest tenth, without a trailing `.0`.
fn tenths(value: f64) -> String {
    let rounded = (value * 10.0).round() / 10.0;
    if rounded.fract() == 0.0 {
        format!("{rounded:.0}")
    } else {
        format!("{rounded:.1}")
    }
}

/// What the inspector shows about a selected problem.
#[derive(Debug, Clone, PartialEq)]
pub struct Details {
    /// "Error · Motion".
    pub eyebrow: String,
    /// How serious it is.
    pub severity: Severity,
    /// Its kind.
    pub category: Category,
    /// A plain-English title: "door hits wall as it opens".
    pub title: String,
    /// The message and its notes, in plain words.
    pub explanation: String,
    /// The problem exactly as `stave check` prints it.
    pub text: String,
    /// Ways to fix it, from its help line.
    pub fixes: Vec<Fix>,
    /// Where it is written.
    pub location: Option<SourceLocation>,
    /// For a door or drawer that hits something: the motion card.
    pub motion: Option<MotionCard>,
}

/// The details of one of a result's problems.
pub fn details(checked: &Checked, index: ProblemIndex) -> Option<Details> {
    let problem = checked.problems.get(index.0)?;
    let diagnostic = checked.opened.diagnostics.get(index.0)?;
    let category = Category::of(diagnostic.code);
    let place = place_of(checked, diagnostic);
    let location = place.map(|place| place.location);
    let motion = problem
        .contact
        .as_ref()
        .and_then(|contact| motion_card(checked, contact));
    let title = specific_title(checked, category, problem.contact.as_ref())
        .unwrap_or_else(|| category.plain_title());
    let mut explanation = sentence(&diagnostic.message);
    for note in &diagnostic.notes {
        explanation.push(' ');
        explanation.push_str(&sentence(note));
    }
    let fixes = diagnostic
        .help
        .as_deref()
        .map(|help| {
            ways(help)
                .into_iter()
                .map(|text| Fix {
                    text,
                    location: location.clone(),
                })
                .collect()
        })
        .unwrap_or_default();
    Some(Details {
        eyebrow: fill_in(
            &tr("{severity} · {category}"),
            &[
                (
                    "severity",
                    &capitalised(&severity_word(diagnostic.severity)),
                ),
                ("category", &category.title()),
            ],
        ),
        severity: diagnostic.severity,
        category,
        title,
        explanation,
        text: relative_paths(
            render_diagnostic(&checked.opened.sources, diagnostic).trim_end(),
            &checked.folder,
        ),
        fixes,
        location,
        motion,
    })
}

/// Whether a place in a [`Report`] is where the problem is, or a place
/// that explains it (the kind's rule a requirement comes from).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    /// Where the problem is: `-->` in `stave check`'s text, underlined
    /// with `^`.
    Primary,
    /// A place that explains it: `:::`, underlined with `-`.
    Secondary,
}

/// One place a [`Report`] points at, with its line of source and the part
/// of it the problem is about, as `stave check` prints them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportPlace {
    /// Where the problem is, or a place that explains it.
    pub role: Role,
    /// `carcass.stave:102:22`: the project's own files relative to its
    /// folder, the standard library's as `std/...`.
    pub shown: String,
    /// The place, for opening in the editor.
    pub location: SourceLocation,
    /// The line's text before the underlined part (tabs as spaces).
    pub before: String,
    /// The underlined part of the line.
    pub marked: String,
    /// The rest of the line.
    pub after: String,
    /// What `stave check` writes after the underline (`the kind's rule`);
    /// may be empty.
    pub message: String,
}

/// A problem in full, as `stave check` prints it, as data for a viewer:
/// the heading, every place with its line of source, the notes and the
/// help line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// How serious it is.
    pub severity: Severity,
    /// Its code: `require`.
    pub code: Code,
    /// The message.
    pub message: String,
    /// Where it is (first) and the places that explain it.
    pub places: Vec<ReportPlace>,
    /// The extra facts (`= note:`).
    pub notes: Vec<String>,
    /// What to do about it (`= help:`).
    pub help: Option<String>,
}

impl Report {
    /// The heading's start: `error[require]`.
    pub fn heading(&self) -> String {
        format!("{}[{}]", self.severity, self.code)
    }
}

/// One of a result's problems in full.
pub fn report(checked: &Checked, index: ProblemIndex) -> Option<Report> {
    checked
        .opened
        .diagnostics
        .get(index.0)
        .map(|diagnostic| report_of(checked, diagnostic))
}

/// Every one of a result's problems in full, in `stave check` order.
pub fn reports(checked: &Checked) -> Vec<Report> {
    checked
        .opened
        .diagnostics
        .iter()
        .map(|diagnostic| report_of(checked, diagnostic))
        .collect()
}

fn report_of(checked: &Checked, diagnostic: &Diagnostic) -> Report {
    let places = diagnostic
        .labels
        .iter()
        .enumerate()
        .filter_map(|(index, label)| {
            let placement = place_label(&checked.opened.sources, label)?;
            let line = LineNumber::from_usize(placement.start.line)?;
            let path = Path::new(placement.file.path());
            let shown = path.strip_prefix(&checked.folder).unwrap_or(path);
            let text = placement.line_text().replace('\t', " ");
            let skip = placement.start.column.saturating_sub(1);
            let before: String = text.chars().take(skip).collect();
            let marked: String = text.chars().skip(skip).take(placement.width).collect();
            let after: String = text
                .chars()
                .skip(skip.saturating_add(placement.width))
                .collect();
            Some(ReportPlace {
                role: if index == 0 {
                    Role::Primary
                } else {
                    Role::Secondary
                },
                shown: format!(
                    "{}:{}:{}",
                    shown.display(),
                    placement.start.line,
                    placement.start.column
                ),
                location: SourceLocation {
                    path: placement.file.path().to_owned(),
                    line,
                },
                before,
                marked,
                after,
                message: label.message.clone(),
            })
        })
        .collect();
    Report {
        severity: diagnostic.severity,
        code: diagnostic.code,
        message: diagnostic.message.clone(),
        places,
        notes: diagnostic.notes.clone(),
        help: diagnostic.help.clone(),
    }
}

/// Text with a capital first letter, unless it starts with a name as
/// written (`door.panel`, `side_l`), which keeps its case.
fn capitalised(text: &str) -> String {
    let first_word = text.split_whitespace().next().unwrap_or("");
    if first_word.contains(['.', '_', '[', '(']) {
        return text.to_owned();
    }
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

/// A severity in the window's words: "error", "warning", "note".
pub fn severity_word(severity: Severity) -> String {
    match severity {
        Severity::Error => tr("error"),
        Severity::Warning => tr("warning"),
        Severity::Note => tr("note"),
    }
}

/// The rendered text with the project folder's own files named relative
/// to it (`project.stave:82:10`), as `stave check .` prints them from
/// inside the folder.
fn relative_paths(text: &str, folder: &Path) -> String {
    let prefix = format!("{}/", folder.display());
    text.replace(&format!(" {prefix}"), " ")
}

/// A message as a sentence: a capital first letter (unless it starts with
/// a name) and a full stop.
fn sentence(text: &str) -> String {
    let mut out = capitalised(text.trim());
    if !out.is_empty() && !out.ends_with(['.', '!', '?']) {
        out.push('.');
    }
    out
}

/// A help line split into its alternatives ("..., or ..."), each a
/// sentence without its full stop.
fn ways(help: &str) -> Vec<String> {
    help.split(", or ")
        .flat_map(|part| part.split("; or "))
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| capitalised(part.trim_end_matches('.')))
        .collect()
}

/// "door hits wall as it opens": for a motion, its assembly and what it
/// hits.
fn specific_title(
    checked: &Checked,
    category: Category,
    contact: Option<&Contact>,
) -> Option<String> {
    let contact = contact?;
    let design = checked.design()?;
    match category {
        Category::Motion => {
            let motion = contact.motion?;
            let name = &design.assembly(motion.assembly)?.name;
            let mut hits: Vec<String> = Vec::new();
            for item in contact.items.iter().copied().map(item_of) {
                if inside(design, item, motion.assembly) {
                    continue;
                }
                let found = match item {
                    Item::Part(id) => design.part(id).map(|p| p.name.clone()),
                    Item::Hardware(id) => design.hardware_piece(id).map(|h| h.name.clone()),
                    Item::Member(member) => design.pair_member_name(member),
                };
                if let Some(found) = found
                    && !hits.contains(&found)
                {
                    hits.push(found);
                }
            }
            if hits.is_empty() {
                return None;
            }
            let turns = checked
                .scene
                .as_ref()
                .and_then(|graph| graph.motion_range(motion.assembly))
                .is_none_or(|range| range.turns());
            let template = if turns {
                tr("{name} hits {things} as it opens")
            } else {
                tr("{name} hits {things} as it slides out")
            };
            Some(fill_in(
                &template,
                &[("name", &name), ("things", &hits.join(&tr(" and ")))],
            ))
        }
        Category::Overlap
        | Category::NoContact
        | Category::Adjust
        | Category::Mechanism
        | Category::Order
        | Category::Requirement
        | Category::Placement
        | Category::Syntax
        | Category::Name
        | Category::Value
        | Category::Cut
        | Category::Hardware
        | Category::NotSupported => None,
    }
}

fn motion_card(checked: &Checked, contact: &Contact) -> Option<MotionCard> {
    let motion = contact.motion?;
    let (design, graph) = checked.drawable()?;
    let range = graph.motion_range(motion.assembly)?;
    let name = design.assembly(motion.assembly)?.name.clone();
    let (travel, closed, contact_at, full) = match range {
        MotionRange::Swing { full } => {
            let contact = range.angle(motion.at)?;
            let shown = range.angle(motion.shown)?;
            (
                MotionTravel::Swing {
                    full,
                    contact,
                    shown,
                },
                MotionAmount::Angle(Angle::ZERO),
                MotionAmount::Angle(contact),
                MotionAmount::Angle(full),
            )
        }
        MotionRange::Slide { full } => {
            let contact = range.travel(motion.at)?;
            let shown = range.travel(motion.shown)?;
            (
                MotionTravel::Slide {
                    full,
                    contact,
                    shown,
                },
                MotionAmount::Length(Length::ZERO),
                MotionAmount::Length(contact),
                MotionAmount::Length(full),
            )
        }
    };
    let end = match travel {
        MotionTravel::Swing { .. } => tr("opens to {amount}"),
        MotionTravel::Slide { .. } => tr("slides out {amount}"),
    };
    Some(MotionCard {
        assembly: motion.assembly,
        name,
        travel,
        start: fill_in(
            &tr("closed {amount}"),
            &[("amount", &MotionCard::exact_text(closed))],
        ),
        blocked: fill_in(
            &tr("blocked from {amount}"),
            &[("amount", &MotionCard::exact_text(contact_at))],
        ),
        end: fill_in(&end, &[("amount", &MotionCard::exact_text(full))]),
    })
}

// ---------------------------------------------------------------------------
// Pins: the problems marked where they happen in the 3D view
// ---------------------------------------------------------------------------

/// The most problems pinned in the 3D view at once.
pub const PIN_LIMIT: usize = 12;

/// A problem pinned where it happens in the 3D view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pinned {
    /// Which problem.
    pub index: ProblemIndex,
    /// Its mark: an error's or a warning's.
    pub mark: Mark,
}

/// The problems that get a pin in the 3D view, in `stave check` order:
/// those the design knows the place of, errors and warnings (notes get
/// none), the first `limit` of them. While a problem is posed only its own
/// pin shows.
pub fn pins(
    problems: &[crate::view::Problem],
    posed: Option<ProblemIndex>,
    limit: usize,
) -> Vec<Pinned> {
    problems
        .iter()
        .enumerate()
        .filter(|(index, _)| posed.is_none_or(|posed| posed.0 == *index))
        .filter(|(_, problem)| problem.contact.is_some())
        .filter_map(|(index, problem)| {
            Some(Pinned {
                index: ProblemIndex(index),
                mark: Mark::of(problem.severity)?,
            })
        })
        .take(limit)
        .collect()
}

/// The words on the chip at the view's edge that names the first pinned
/// problem whose place is off screen, and how many more are:
/// "Off screen: error · The door hits the side · 2 more".
pub fn off_screen_text(mark: Mark, message: &str, more: usize) -> String {
    let words = match mark {
        Mark::Warning => tr("warning"),
        Mark::Error | Mark::Involved => tr("error"),
    };
    if more == 0 {
        fill_in(
            &tr("Off screen: {words} · {message}"),
            &[("words", &words), ("message", &message)],
        )
    } else {
        fill_in(
            &ntr(
                "Off screen: {words} · {message} · {more} more",
                "Off screen: {words} · {message} · {more} more",
                more,
            ),
            &[("words", &words), ("message", &message), ("more", &more)],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_code_has_a_category_and_a_title() {
        for code in Code::ALL {
            let category = Category::of(*code);
            assert!(!category.title().is_empty());
            assert!(!category.plain_title().is_empty());
        }
    }

    #[test]
    fn help_lines_split_into_ways() {
        assert_eq!(
            ways(
                "open door no further than 90.1deg (a smaller opening_angle), or move side_l out of its way"
            ),
            vec![
                "Open door no further than 90.1deg (a smaller opening_angle)".to_owned(),
                "Move side_l out of its way".to_owned()
            ]
        );
        assert_eq!(
            ways("leave at least 1mm between door and wall"),
            vec!["Leave at least 1mm between door and wall".to_owned()]
        );
    }

    #[test]
    fn numbers_show_to_a_tenth() {
        assert_eq!(tenths(90.06), "90.1");
        assert_eq!(tenths(110.0), "110");
        assert_eq!(sentence("door.panel hits wall"), "door.panel hits wall.");
        assert_eq!(
            sentence("the hinges open to 110deg"),
            "The hinges open to 110deg."
        );
    }
}
