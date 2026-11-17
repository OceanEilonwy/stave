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

use std::collections::BTreeSet;
use std::path::Path;

use stave_lang::{Code, Diagnostic, Severity, render_diagnostic};
use stave_model::solved::{AssemblyId, SolvedDesign};
use stave_scene::Item;

use crate::check::Checked;
use crate::inspect::{self, AssemblyNode, NonEmpty, PartsTree, TreeEntry};
use crate::session::{LineNumber, ProblemIndex, Selection, SourceLocation};
use crate::{Contact, ContactItem};

/// What kind of problem a diagnostic is, for its icon and its row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Category {
    /// A door or drawer hits something as it moves (`motion`).
    Motion,
    /// Wood passes through wood (`overlap`).
    Overlap,
    /// A joint's parts don't meet (`no-contact`).
    NoContact,
    /// A drawer's slides don't fit their gap (`slide-gap`).
    SlideGap,
    /// A door covers its side by more or less than its hinges allow
    /// (`hinge-overlay`).
    HingeOverlay,
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
    /// A cut no recipe can make with the project's tools (`no-recipe`).
    Cut,
    /// A hardware model doesn't match its envelope (`model-mismatch`).
    Hardware,
    /// Something Stave can't do yet (`not-supported`).
    NotSupported,
}

impl Category {
    /// The category of a diagnostic code.
    pub fn of(code: Code) -> Category {
        match code {
            Code::Motion => Category::Motion,
            Code::Overlap => Category::Overlap,
            Code::NoContact => Category::NoContact,
            Code::SlideGap => Category::SlideGap,
            Code::HingeOverlay => Category::HingeOverlay,
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
            Code::NoRecipe => Category::Cut,
            Code::ModelMismatch => Category::Hardware,
            Code::NotSupported => Category::NotSupported,
        }
    }

    /// Its name on a row: "Motion", "No contact".
    pub fn title(self) -> &'static str {
        match self {
            Category::Motion => "Motion",
            Category::Overlap => "Overlap",
            Category::NoContact => "No contact",
            Category::SlideGap => "Slide gap",
            Category::HingeOverlay => "Hinge overlay",
            Category::Order => "Order",
            Category::Requirement => "Requirement",
            Category::Placement => "Placement",
            Category::Syntax => "Syntax",
            Category::Name => "Name",
            Category::Value => "Value",
            Category::Cut => "Cut",
            Category::Hardware => "Hardware",
            Category::NotSupported => "Not supported yet",
        }
    }

    /// A plain-English title for a problem of this kind, when nothing more
    /// specific can be said.
    fn plain_title(self) -> &'static str {
        match self {
            Category::Motion => "A door or drawer hits something as it moves",
            Category::Overlap => "Wood passes through wood",
            Category::NoContact => "The joint's parts don't meet",
            Category::SlideGap => "The drawer doesn't fit its slides",
            Category::HingeOverlay => "The door covers the side by the wrong amount",
            Category::Order => "The joint can't be put together in this order",
            Category::Requirement => "A requirement doesn't hold",
            Category::Placement => "Where a part goes isn't settled",
            Category::Syntax => "Stave can't read the text here",
            Category::Name => "A name isn't right",
            Category::Value => "A value is the wrong kind or size",
            Category::Cut => "A cut can't be made with your tools",
            Category::Hardware => "The hardware doesn't match its size",
            Category::NotSupported => "Stave can't do this yet",
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
            1 => self.category.title().to_owned(),
            count => format!("{} ({count})", self.category.title()),
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
    match contact_item {
        ContactItem::Part(id) => Item::Part(id),
        ContactItem::Hardware(id) => Item::Hardware(id),
    }
}

/// Whether an item is inside an assembly (at any depth).
fn inside(design: &SolvedDesign, item: Item, assembly: AssemblyId) -> bool {
    let mut current = match item {
        Item::Part(id) => design.part(id).and_then(|p| p.assembly),
        Item::Hardware(id) => design.hardware.get(id.index()).and_then(|h| h.assembly),
    };
    let mut steps = 0;
    while let Some(id) = current {
        if id == assembly {
            return true;
        }
        steps += 1;
        if steps > design.assemblies.len() {
            return false;
        }
        current = design.assemblies.get(id.index()).and_then(|a| a.parent);
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
        Category::Overlap => (items.clone(), Vec::new()),
        Category::NoContact
        | Category::SlideGap
        | Category::HingeOverlay
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
    /// `true` for a door (degrees), `false` for a drawer (millimetres).
    pub turns: bool,
    /// Fully open: radians or millimetres.
    pub full: f32,
    /// Where it first touches: radians or millimetres.
    pub contact: f32,
    /// Where the shared wood is measured, a little past the contact.
    pub shown: f32,
    /// "closed 0°".
    pub start: String,
    /// "blocked from 0.6°".
    pub blocked: String,
    /// "opens to 110°".
    pub end: String,
}

impl MotionCard {
    /// The card's lead: "Swing door", "Slide drawer[top]".
    pub fn lead(&self) -> String {
        format!(
            "{} {}",
            if self.turns { "Swing" } else { "Slide" },
            self.name
        )
    }

    /// An amount (radians or millimetres) as the card writes it: `90.1°`,
    /// `312 mm`.
    pub fn amount_text(&self, amount: f32) -> String {
        if self.turns {
            format!("{}°", tenths(f64::from(amount).to_degrees()))
        } else {
            format!("{} mm", tenths(f64::from(amount)))
        }
    }
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
    /// "ERROR · MOTION".
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
        .unwrap_or_else(|| category.plain_title().to_owned());
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
        eyebrow: format!(
            "{} · {}",
            diagnostic.severity.word().to_uppercase(),
            category.title().to_uppercase()
        ),
        severity: diagnostic.severity,
        category,
        title,
        explanation,
        text: render_diagnostic(&checked.opened.sources, diagnostic)
            .trim_end()
            .to_owned(),
        fixes,
        location,
        motion,
    })
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
            let name = &design.assemblies.get(motion.assembly.index())?.name;
            let mut hits: Vec<String> = Vec::new();
            for item in contact.items.iter().copied().map(item_of) {
                if inside(design, item, motion.assembly) {
                    continue;
                }
                let found = match item {
                    Item::Part(id) => design.part(id).map(|p| p.name.clone()),
                    Item::Hardware(id) => design.hardware.get(id.index()).map(|h| h.name.clone()),
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
                .is_none_or(|range| range.turns);
            Some(format!(
                "{name} hits {} as it {}",
                hits.join(" and "),
                if turns { "opens" } else { "slides out" }
            ))
        }
        Category::Overlap
        | Category::NoContact
        | Category::SlideGap
        | Category::HingeOverlay
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
    let name = design.assemblies.get(motion.assembly.index())?.name.clone();
    let mut card = MotionCard {
        assembly: motion.assembly,
        name,
        turns: range.turns,
        full: range.full,
        contact: graph.amount(motion.assembly, motion.at)?,
        shown: graph.amount(motion.assembly, motion.shown)?,
        start: String::new(),
        blocked: String::new(),
        end: String::new(),
    };
    card.start = format!("closed {}", card.amount_text(0.0));
    card.blocked = format!("blocked from {}", card.amount_text(card.contact));
    card.end = format!(
        "{} {}",
        if card.turns { "opens to" } else { "slides out" },
        card.amount_text(card.full)
    );
    Some(card)
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
