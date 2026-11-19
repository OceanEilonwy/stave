//! What the Parts view shows about a design, worked out without any UI:
//! the parts tree (the carcass's parts, then each assembly with its motion
//! and what is inside it) and the inspector's account of the selection
//! (its kind, its stock and size, how it moves, its joints and cuts, and
//! where it is written in the text, with the snippet highlighted).

use crate::i18n::{fill_in, ntr, tr, tr_noop};
use stave_lang::Severity;
use stave_lang::units::{Angle, Length};
use stave_lang::{SourceMap, SyntaxKind};
use stave_model::Model;
use stave_model::defs::Decl;
use stave_model::mechanism::{DriveValue, Mechanism};
use stave_model::nonempty::NonEmpty;
use stave_model::solved::{
    AssemblyId, Body, CutSource, HardwareBase, HardwareId, PairIndex, PairMember, Part, PartId,
    PartOrigin, RegionId, RoleValue, Shape, SolvedDesign,
};
use stave_scene::Item;
use stave_schema::species::Srgb;

use crate::check::Checked;
use crate::focus::{self, Target};
use crate::session::{LineNumber, ProblemIndex, Selection, SourceLocation};
use crate::view::Row;

/// The grey of hardware swatches (#a8adb4).
const STEEL: Srgb = Srgb {
    red: 0xa8,
    green: 0xad,
    blue: 0xb4,
};

/// The most lines a snippet shows before it is cut short.
const SNIPPET_LINES: usize = 12;

/// The little coloured mark before a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Swatch {
    /// Its shape.
    pub shape: SwatchShape,
    /// Its colour: the wood's species, or steel.
    pub colour: Srgb,
}

/// A swatch's shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SwatchShape {
    /// A board: a rounded square.
    Board,
    /// Round or turned wood, or hardware: a circle.
    Round,
}

/// One row of the parts tree: a part or piece of hardware, or a few that
/// are the same (`rail[top], rail[bottom]`, `feet[4]`, `side_l, side_r`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeEntry {
    /// "stile[hinge]", "feet[4]", "hinges[lower, upper]".
    pub label: String,
    /// Dim text at the end: the stock (`oak_19`) or `turned`.
    pub detail: Option<String>,
    /// What it stands for; selecting the row selects the first.
    pub items: NonEmpty<Item>,
    /// Each item's own label (`rail[top]`), in the order of `items`, for
    /// showing a group's members one by one.
    pub member_labels: Vec<String>,
    /// Each item's own dim text, in the order of `items`: a joint's
    /// members on one pair say that pair's copies (`dowel_pin × 4`) where
    /// the group says the joint's (`dowel_pin × 8`).
    pub member_details: Vec<Option<String>>,
    /// Its mark.
    pub swatch: Swatch,
    /// Every full name it stands for, for searching.
    names: Vec<String>,
}

impl TreeEntry {
    /// Whether a search matches it (any part of any of its names, ignoring
    /// case).
    pub fn matches(&self, query: &str) -> bool {
        let query = query.trim().to_lowercase();
        query.is_empty()
            || self.label.to_lowercase().contains(&query)
            || self.names.iter().any(|n| n.to_lowercase().contains(&query))
            || self
                .detail
                .as_ref()
                .is_some_and(|d| d.to_lowercase().contains(&query))
    }
}

/// An assembly in the parts tree, with its motion and what is inside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssemblyNode {
    /// The assembly.
    pub id: AssemblyId,
    /// Its name relative to its parent: `door_l`, `drawer[top]`.
    pub label: String,
    /// "turns 110°", "slides out 400 mm", "mirror of door_l".
    pub detail: Option<String>,
    /// Its own parts and hardware.
    pub entries: Vec<TreeEntry>,
    /// The assemblies inside it.
    pub children: Vec<AssemblyNode>,
}

impl AssemblyNode {
    /// A copy with only the rows a search matches; `None` when nothing in
    /// it does. An assembly whose own name matches keeps everything.
    fn filtered(&self, query: &str) -> Option<AssemblyNode> {
        if self
            .label
            .to_lowercase()
            .contains(&query.trim().to_lowercase())
        {
            return Some(self.clone());
        }
        let entries: Vec<TreeEntry> = self
            .entries
            .iter()
            .filter(|e| e.matches(query))
            .cloned()
            .collect();
        let children: Vec<AssemblyNode> = self
            .children
            .iter()
            .filter_map(|c| c.filtered(query))
            .collect();
        (!entries.is_empty() || !children.is_empty()).then(|| AssemblyNode {
            id: self.id,
            label: self.label.clone(),
            detail: self.detail.clone(),
            entries,
            children,
        })
    }
}

/// The Parts view's sidebar tree.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PartsTree {
    /// The parts and hardware outside every assembly.
    pub carcass: Vec<TreeEntry>,
    /// The top-level assemblies.
    pub assemblies: Vec<AssemblyNode>,
}

impl PartsTree {
    /// The tree with only what a search matches.
    pub fn filtered(&self, query: &str) -> PartsTree {
        if query.trim().is_empty() {
            return self.clone();
        }
        PartsTree {
            carcass: self
                .carcass
                .iter()
                .filter(|e| e.matches(query))
                .cloned()
                .collect(),
            assemblies: self
                .assemblies
                .iter()
                .filter_map(|a| a.filtered(query))
                .collect(),
        }
    }

    /// Whether it has no rows.
    pub fn is_empty(&self) -> bool {
        self.carcass.is_empty() && self.assemblies.is_empty()
    }
}

/// `110°`, `12.5°`.
pub fn degrees(angle: Angle) -> String {
    let text = angle.to_string();
    match text.strip_suffix("deg") {
        Some(number) => format!("{number}°"),
        None => text,
    }
}

/// A length in millimetres: `1029.5 mm`.
pub(crate) fn mm(length: Length) -> String {
    let text = length.to_string();
    let number = text.strip_suffix("mm").unwrap_or(&text);
    format!("{number} mm")
}

/// A name with its assembly's prefix taken off: `stile[hinge]` for
/// `door_l.stile[hinge]` in `door_l`.
pub(crate) fn relative<'a>(
    design: &SolvedDesign,
    name: &'a str,
    assembly: Option<AssemblyId>,
) -> &'a str {
    assembly
        .and_then(|id| design.assembly(id))
        .and_then(|a| name.strip_prefix(a.name.as_str()))
        .and_then(|rest| rest.strip_prefix('.'))
        .unwrap_or(name)
}

/// An assembly's motion in a few words, from its mechanism's range:
/// "turns 110°", "slides out 400 mm".
fn motion_words(mechanism: &Mechanism) -> String {
    match mechanism.drive().extent() {
        DriveValue::Turn(angle) => fill_in(&tr("turns {angle}"), &[("angle", &degrees(angle))]),
        DriveValue::Travel(travel) => {
            fill_in(&tr("slides out {length}"), &[("length", &mm(travel))])
        }
    }
}

/// The swatch of a part.
fn part_swatch(part: &Part) -> Swatch {
    let shape = match part.shape {
        Shape::Board => SwatchShape::Board,
        Shape::Round { .. } | Shape::Turned { .. } => SwatchShape::Round,
    };
    Swatch {
        shape,
        colour: part.stock.species.colour(),
    }
}

/// The steel swatch of hardware.
fn hardware_swatch() -> Swatch {
    Swatch {
        shape: SwatchShape::Round,
        colour: STEEL,
    }
}

/// The swatch of a joint's member: its wood, or steel.
fn member_swatch(body: &Body) -> Swatch {
    match body.item.as_ref().and_then(|item| item.wood) {
        Some(species) => Swatch {
            shape: SwatchShape::Round,
            colour: species.colour(),
        },
        None => hardware_swatch(),
    }
}

/// `rail` and `top` for `rail[top]`; the whole name and `None` otherwise.
pub(crate) fn split_index(name: &str) -> (&str, Option<&str>) {
    match name.strip_suffix(']').and_then(|n| n.rsplit_once('[')) {
        Some((base, index)) if !base.contains('[') => (base, Some(index)),
        _ => (name, None),
    }
}

/// What makes rows the same: parts of one stock and size, or hardware of
/// one kind, under one base name.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Sameness {
    Part {
        base: String,
        stock: String,
        size: (Length, Length, Length),
    },
    Hardware {
        base: String,
        kind: String,
    },
    Member {
        joint: stave_model::solved::JointId,
        member: String,
    },
}

/// A row before grouping.
struct Single {
    item: Item,
    label: String,
    full: String,
    /// Its own dim text.
    detail: Option<String>,
    /// The dim text of a group it heads, when that is not its own (a
    /// joint's copies over all its pairs).
    group_detail: Option<String>,
    swatch: Swatch,
    same: Sameness,
    mirror_of: Option<PartId>,
}

fn group(singles: Vec<Single>) -> Vec<TreeEntry> {
    let mut groups: Vec<(Sameness, Vec<Single>)> = Vec::new();
    for single in singles {
        let joins = groups.last().is_some_and(|(same, members)| {
            *same == single.same
                || members.first().is_some_and(|first| {
                    single.mirror_of.is_some()
                        && Some(first.item) == single.mirror_of.map(Item::Part)
                })
        });
        match groups.last_mut() {
            Some((_, members)) if joins => members.push(single),
            _ => groups.push((single.same.clone(), vec![single])),
        }
    }
    groups
        .into_iter()
        .filter_map(|(_, members)| {
            let mut members = members.into_iter();
            let first = members.next()?;
            let mut items = NonEmpty::one(first.item);
            let mut names = vec![first.full];
            let mut labels = vec![first.label];
            let mut details = vec![first.detail.clone()];
            for member in members {
                items.push(member.item);
                names.push(member.full);
                labels.push(member.label);
                details.push(member.detail);
            }
            let label = group_label(&labels);
            let detail = if items.len() == 1 {
                first.detail
            } else {
                first.group_detail.or(first.detail)
            };
            Some(TreeEntry {
                label,
                member_labels: labels,
                member_details: details,
                detail,
                items,
                swatch: first.swatch,
                names,
            })
        })
        .collect()
}

/// `stile[hinge]`; `feet[4]` for numbered members or more than two;
/// `hinges[lower, upper]` for two named ones; `side_l, side_r` otherwise.
pub(crate) fn group_label(labels: &[String]) -> String {
    let [first, ..] = labels else {
        return String::new();
    };
    if labels.len() == 1 {
        return first.clone();
    }
    let split: Vec<(&str, Option<&str>)> = labels.iter().map(|l| split_index(l)).collect();
    let base = split.first().map(|(b, _)| *b);
    let indices: Option<Vec<&str>> = split
        .iter()
        .map(|(b, index)| if Some(*b) == base { *index } else { None })
        .collect();
    match (base, indices) {
        (Some(base), Some(indices)) => {
            if indices.len() > 2 || indices.iter().all(|i| i.parse::<u32>().is_ok()) {
                format!("{base}[{}]", indices.len())
            } else {
                format!("{base}[{}]", indices.join(", "))
            }
        }
        _ => labels.join(", "),
    }
}

/// The parts and pieces of hardware a selection stands for in the 3D
/// view: the selected item, or everything inside a selected assembly (at
/// any depth), parts first, in design order. Nothing for a selection that
/// is not an item (a joint, a place in the text, a problem).
pub fn items_in(design: &SolvedDesign, selection: &Selection) -> Vec<Item> {
    match selection {
        Selection::Assembly(id) => design
            .parts()
            .iter()
            .map(|part| Item::Part(part.id))
            .chain(
                design
                    .hardware()
                    .iter()
                    .map(|piece| Item::Hardware(piece.id)),
            )
            .filter(|item| crate::problems::inside(design, *item, *id))
            .collect(),
        other => other.item().into_iter().collect(),
    }
}

/// The parts tree of a design.
pub fn parts_tree(design: &SolvedDesign) -> PartsTree {
    let singles_in = |assembly: Option<AssemblyId>| -> Vec<Single> {
        let parts = design
            .parts()
            .iter()
            .filter(|p| p.assembly == assembly)
            .map(|part| {
                let label = relative(design, &part.name, part.assembly).to_owned();
                let detail = match part.shape {
                    Shape::Turned { .. } => tr("turned"),
                    Shape::Board | Shape::Round { .. } => part.stock.name.clone(),
                };
                let mirror_of = match part.origin {
                    PartOrigin::Mirror { source } => Some(source),
                    PartOrigin::Declared
                    | PartOrigin::MirroredAssembly { .. }
                    | PartOrigin::Made { .. }
                    | PartOrigin::Piece { .. }
                    | PartOrigin::Boards { .. } => None,
                };
                Single {
                    item: Item::Part(part.id),
                    same: Sameness::Part {
                        base: split_index(&label).0.to_owned(),
                        stock: part.stock.name.clone(),
                        size: (part.cut.length, part.cut.width, part.cut.thickness),
                    },
                    label,
                    full: part.name.clone(),
                    detail: Some(detail),
                    group_detail: None,
                    swatch: part_swatch(part),
                    mirror_of,
                }
            });
        let hardware = design
            .hardware()
            .iter()
            .filter(|h| h.assembly == assembly)
            .map(|hardware| {
                let label = relative(design, &hardware.name, hardware.assembly).to_owned();
                Single {
                    item: Item::Hardware(hardware.id),
                    same: Sameness::Hardware {
                        base: split_index(&label).0.to_owned(),
                        kind: hardware.kind.clone(),
                    },
                    label,
                    full: hardware.name.clone(),
                    detail: None,
                    group_detail: None,
                    swatch: hardware_swatch(),
                    mirror_of: None,
                }
            });
        // Each joint pair's members (a dowel's pins), with the part that
        // carries them: a row for every pair, grouped by joint, saying how
        // many copies the joint places in all.
        let members = design
            .pair_members()
            .filter(move |(_, body)| {
                body.carried_by()
                    .and_then(|part| design.part(part))
                    .is_some_and(|part| part.assembly == assembly)
            })
            .filter_map(|(member, body)| {
                let joint = design.joint(member.joint)?;
                let base = format!(
                    "{}'s {}",
                    relative(design, &joint.name, assembly),
                    body.name
                );
                let label = if joint.pairs.len() > 1 {
                    format!("{base}[{}]", member.pair)
                } else {
                    base
                };
                let total: u32 = design
                    .pair_members()
                    .filter(|(other, found)| other.joint == member.joint && found.name == body.name)
                    .map(|(_, found)| found.copies())
                    .sum();
                let what = body
                    .item
                    .as_ref()
                    .map_or_else(|| body.name.replace('_', " "), |item| item.kind.clone());
                Some(Single {
                    item: Item::Member(member),
                    same: Sameness::Member {
                        joint: member.joint,
                        member: body.name.clone(),
                    },
                    full: design
                        .pair_member_name(member)
                        .unwrap_or_else(|| label.clone()),
                    label,
                    detail: Some(format!("{what} × {}", body.copies())),
                    group_detail: Some(format!("{what} × {total}")),
                    swatch: member_swatch(body),
                    mirror_of: None,
                })
            });
        parts.chain(hardware).chain(members).collect()
    };
    fn node(
        design: &SolvedDesign,
        id: AssemblyId,
        depth: usize,
        singles_in: &dyn Fn(Option<AssemblyId>) -> Vec<Single>,
    ) -> Option<AssemblyNode> {
        let assembly = design.assembly(id)?;
        let parent_name = assembly
            .parent
            .and_then(|p| design.assembly(p))
            .map(|p| p.name.as_str());
        let label = parent_name
            .and_then(|p| assembly.name.strip_prefix(p))
            .and_then(|rest| rest.strip_prefix('.'))
            .unwrap_or(&assembly.name)
            .to_owned();
        let detail = match (assembly.mirror_of, &assembly.mechanism) {
            (Some(source), _) => design
                .assemblies()
                .get(source.index())
                .map(|s| fill_in(&tr("mirror of {name}"), &[("name", &s.name)])),
            (None, Some(motion)) => Some(motion_words(motion)),
            (None, None) => Some(assembly.component.clone()),
        };
        // Assemblies nest only as deep as there are assemblies.
        let children = if depth > design.assemblies().len() {
            Vec::new()
        } else {
            design
                .assemblies()
                .iter()
                .filter(|a| a.parent == Some(id))
                .filter_map(|a| node(design, a.id, depth.saturating_add(1), singles_in))
                .collect()
        };
        Some(AssemblyNode {
            id,
            label,
            detail,
            entries: group(singles_in(Some(id))),
            children,
        })
    }
    PartsTree {
        carcass: group(singles_in(None)),
        assemblies: design
            .assemblies()
            .iter()
            .filter(|a| a.parent.is_none())
            .filter_map(|a| node(design, a.id, 0, &singles_in))
            .collect(),
    }
}

// ---------------------------------------------------------------------------
// The inspector
// ---------------------------------------------------------------------------

/// How a word of a snippet is coloured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenClass {
    /// Names and punctuation.
    Plain,
    /// `assembly`, `part`, `import`.
    Keyword,
    /// A capitalised name: `FramePanelDoor`.
    Type,
    /// A name called like a function: `overlay(`.
    Function,
    /// `18mm`, `4`.
    Number,
    /// A string.
    Text,
    /// A comment.
    Comment,
}

/// A run of text of one colour.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    /// How it is coloured.
    pub class: TokenClass,
    /// The text.
    pub text: String,
}

/// A declaration as written, highlighted, with where it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snippet {
    /// Where it starts.
    pub location: SourceLocation,
    /// `doors.stave:9`.
    pub label: String,
    /// Its lines, each a run of tokens.
    pub lines: Vec<Vec<Token>>,
    /// `true` when the declaration is longer than what is shown.
    pub cut_short: bool,
}

/// What a link in the inspector stands for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkTarget {
    /// Something that can be selected: a part, hardware, an assembly.
    Select(Selection),
    /// A space (a region): it can be lit, not selected.
    Space(RegionId),
}

/// A name in a value that stands for another thing of the design: the
/// inspector draws it as a link that selects it, and lights it in the 3D
/// view while the pointer is on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameLink {
    /// The name as shown: `rail[top]`, `side_l`, `bay[doors]`.
    pub name: String,
    /// What it stands for.
    pub target: LinkTarget,
}

/// Which section of the inspector a [`Connection`] goes in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConnectionSection {
    /// "Joined to": its joints, the hardware on it, what it turns on.
    Joined,
    /// "Cuts": the cuts hardware makes in it.
    Cuts,
    /// The way into focus mode for a joint, a member or hardware.
    HowItWorks,
}

/// A row of the inspector's joints and cuts list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Connection {
    /// Its mark.
    pub swatch: Swatch,
    /// The bold start: `cope_and_stick`, `2 bores`.
    pub lead: String,
    /// The rest in words: "with rail[top], rail[bottom]".
    pub rest: String,
    /// The things it joins this to, each a link (empty for a cut or a
    /// way into focus mode).
    pub partners: Vec<NameLink>,
    /// A few words beside the partners: "back face" for hardware on a
    /// face; empty otherwise.
    pub note: String,
    /// The section it goes in.
    pub section: ConnectionSection,
    /// Focus mode on it, when it can be shown.
    pub target: Option<Target>,
}

/// A label and value row of the inspector, whose value may name other
/// things of the design.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    /// What it is: "Stock".
    pub label: &'static str,
    /// Its value in words.
    pub value: String,
    /// The things its value names, each a link; empty for a plain value.
    pub links: Vec<NameLink>,
}

impl From<Row> for Field {
    fn from(row: Row) -> Field {
        Field {
            label: row.label,
            value: row.value,
            links: Vec::new(),
        }
    }
}

/// A check that fails about the selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailingCheck {
    /// Error or warning.
    pub severity: Severity,
    /// The problem's message.
    pub message: String,
    /// The problem, to select it.
    pub problem: ProblemIndex,
}

/// What the inspector shows about the selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inspection {
    /// "PART · IN door_l": the words the inspection's kind and place are
    /// keyed by, in English.
    pub eyebrow: String,
    /// What kind of thing it is, as the window shows it: "Part", "Joint
    /// member" (translated).
    pub kind: String,
    /// The assembly it sits in: "door_l".
    pub within: Option<String>,
    /// "stile[hinge]".
    pub title: String,
    /// The doc comment: "The upright on the hinge side of a door".
    pub description: Option<String>,
    /// Its mark: the wood's colour, or steel.
    pub swatch: Option<Swatch>,
    /// Stock, finished size, grain, how it moves.
    pub rows: Vec<Field>,
    /// The heading of [`Inspection::connections`]: "Joints and cuts".
    pub connections_title: &'static str,
    /// Its joints and cuts, or what it can be focused on.
    pub connections: Vec<Connection>,
    /// The innermost assembly that moves around it, for the "Open it"
    /// slider.
    pub moves_with: Option<AssemblyId>,
    /// The checks that fail about it, in `stave check` order (empty when
    /// every check about it holds).
    pub checks: Vec<FailingCheck>,
    /// Where it is written.
    pub source: Option<Snippet>,
}

/// Doc comments as one paragraph.
fn docs(decl: &Decl) -> Option<String> {
    let text = decl.docs.join(" ");
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    (!text.is_empty()).then_some(text)
}

/// The innermost assembly around `assembly` (itself included) that moves.
fn moving(design: &SolvedDesign, mut assembly: Option<AssemblyId>) -> Option<AssemblyId> {
    let mut steps = 0;
    while let Some(id) = assembly {
        let found = design.assembly(id)?;
        if found.mechanism.is_some() {
            return Some(id);
        }
        steps += 1;
        if steps > design.assemblies().len() {
            return None;
        }
        assembly = found.parent;
    }
    None
}

/// "with door_l, turns 110°".
fn moves_row(design: &SolvedDesign, moving: Option<AssemblyId>) -> Option<Row> {
    let assembly = design.assemblies().get(moving?.index())?;
    let motion = assembly.mechanism.as_ref()?;
    Some(Row {
        label: tr_noop("Moves"),
        value: fill_in(
            &tr("with {assembly}, {motion}"),
            &[
                ("assembly", &assembly.name),
                ("motion", &motion_words(motion)),
            ],
        ),
    })
}

/// The window's words for an eyebrow's kind: "Part" for "PART".
fn kind_title(kind: &str) -> String {
    match kind {
        "PART" => tr("Part"),
        "PIECE OF A SPLICED PART" => tr("Piece of a spliced part"),
        "MIRROR IMAGE" => tr("Mirror image"),
        "MADE PART" => tr("Made part"),
        "HARDWARE" => tr("Hardware"),
        "ASSEMBLY" => tr("Assembly"),
        "JOINT MEMBER" => tr("Joint member"),
        "JOINT" => tr("Joint"),
        other => other.to_owned(),
    }
}

/// The name of the assembly something sits in.
fn within(design: &SolvedDesign, assembly: Option<AssemblyId>) -> Option<String> {
    assembly
        .and_then(|id| design.assembly(id))
        .map(|found| found.name.clone())
}

/// "PART · IN door_l".
fn eyebrow(design: &SolvedDesign, kind: &str, assembly: Option<AssemblyId>) -> String {
    match assembly.and_then(|id| design.assembly(id)) {
        Some(found) => format!("{kind} · IN {}", found.name),
        None => kind.to_owned(),
    }
}

/// Where a declaration is written, in the project's own files: the item's
/// own declaration when it is there, otherwise the innermost assembly
/// around it declared there (a part of a door from a component library is
/// shown by the door's declaration).
fn source_decl<'m>(
    model: &'m Model,
    own: Option<&'m Decl>,
    mut assembly: Option<stave_model::ids::AssemblyId>,
) -> Option<&'m Decl> {
    let in_root = |decl: &Decl| {
        model
            .defs
            .files
            .get(&decl.file)
            .is_some_and(|scope| scope.module == Some(model.root))
    };
    if let Some(decl) = own.filter(|d| in_root(d)) {
        return Some(decl);
    }
    let mut steps = 0;
    while let Some(id) = assembly {
        let found = model.assemblies.get(id.index()).filter(|a| a.id == id)?;
        if let Some(decl) = model.defs.assembly(found.def).map(|d| &d.decl)
            && in_root(decl)
        {
            return Some(decl);
        }
        steps += 1;
        if steps > model.assemblies.len() {
            break;
        }
        assembly = found.parent;
    }
    own
}

/// A declaration's text, highlighted.
pub fn snippet(sources: &SourceMap, folder: &std::path::Path, decl: &Decl) -> Option<Snippet> {
    let file = sources.get(decl.file)?;
    let line = LineNumber::from_usize(file.line_column(decl.span.start()).line)?;
    let location = SourceLocation {
        path: file.path().to_owned(),
        line,
    };
    let start = usize::try_from(decl.span.start()).ok()?;
    let end = usize::try_from(decl.span.end()).ok()?;
    let text = file.text();
    // Start at the beginning of the line, so indentation lines up.
    let line_start = text.get(..start)?.rfind('\n').map_or(0, |i| i + 1);
    let tokens = highlight(text.get(line_start..end)?);
    let mut lines: Vec<Vec<Token>> = vec![Vec::new()];
    for token in tokens {
        let mut pieces = token.text.split('\n').peekable();
        while let Some(piece) = pieces.next() {
            if !piece.is_empty()
                && let Some(line) = lines.last_mut()
            {
                line.push(Token {
                    class: token.class,
                    text: piece.to_owned(),
                });
            }
            if pieces.peek().is_some() {
                lines.push(Vec::new());
            }
        }
    }
    let cut_short = lines.len() > SNIPPET_LINES;
    lines.truncate(SNIPPET_LINES);
    Some(Snippet {
        label: location.display(folder),
        location,
        lines,
        cut_short,
    })
}

/// Colours `.stave` source text by its tokens. The text is parsed on its
/// own, so a fragment works too; anything the parser drops stays plain.
pub fn highlight(text: &str) -> Vec<Token> {
    // A file id of its own: the parse needs one, and nothing else reads it.
    let Ok(file) = SourceMap::new().add("snippet.stave", text) else {
        return vec![Token {
            class: TokenClass::Plain,
            text: text.to_owned(),
        }];
    };
    let parse = stave_lang::parse_file(file, text);
    let tokens: Vec<(SyntaxKind, String)> = parse
        .syntax()
        .descendants_with_tokens()
        .filter_map(|element| element.into_token())
        .map(|token| (token.kind(), token.text().to_owned()))
        .collect();
    let mut out: Vec<Token> = Vec::with_capacity(tokens.len());
    let mut covered = 0usize;
    for (index, (kind, token)) in tokens.iter().enumerate() {
        let next_significant = tokens
            .iter()
            .skip(index.saturating_add(1))
            .find(|(k, _)| !k.is_trivia())
            .map(|(k, _)| *k);
        let class = match kind {
            SyntaxKind::LINE_COMMENT | SyntaxKind::BLOCK_COMMENT | SyntaxKind::DOC => {
                TokenClass::Comment
            }
            SyntaxKind::QUANTITY | SyntaxKind::INT_NUMBER | SyntaxKind::DECIMAL_NUMBER => {
                TokenClass::Number
            }
            SyntaxKind::STRING_QUOTE | SyntaxKind::STRING_TEXT => TokenClass::Text,
            SyntaxKind::IDENT if next_significant == Some(SyntaxKind::L_PAREN) => {
                TokenClass::Function
            }
            SyntaxKind::IDENT if token.starts_with(|c: char| c.is_ascii_uppercase()) => {
                TokenClass::Type
            }
            kind if kind.is_keyword() => TokenClass::Keyword,
            _ => TokenClass::Plain,
        };
        covered = covered.saturating_add(token.len());
        match out.last_mut() {
            Some(last) if last.class == class => last.text.push_str(token),
            _ => out.push(Token {
                class,
                text: token.clone(),
            }),
        }
    }
    // Whatever the tree didn't keep (it is lossless, so normally nothing).
    if let Some(rest) = text.get(covered..)
        && !rest.is_empty()
    {
        out.push(Token {
            class: TokenClass::Plain,
            text: rest.to_owned(),
        });
    }
    out
}

/// The inspector's account of a selection, from the shown result.
pub fn inspect(checked: &Checked, selection: &Selection) -> Option<Inspection> {
    let design = checked.design()?;
    match selection {
        Selection::Part(id) => inspect_part(checked, design, *id),
        Selection::Hardware(id) => inspect_hardware(checked, design, *id),
        Selection::Assembly(id) => inspect_assembly(checked, design, *id),
        Selection::Joint { joint, pair } => inspect_joint(checked, design, *joint, *pair),
        Selection::Member(member) => inspect_member(checked, design, *member),
        Selection::Location(_) | Selection::Problem(_) => None,
    }
}

fn source_of(
    checked: &Checked,
    own: impl FnOnce(&Model) -> Option<&Decl>,
    assembly: Option<AssemblyId>,
) -> Option<Snippet> {
    let model = checked.opened.model.as_ref()?;
    let decl = source_decl(model, own(model), assembly)?;
    snippet(&checked.opened.sources, &checked.folder, decl)
}

/// Cuts the hardware on a part makes, of one kind and size.
struct HardwareCut {
    /// The kind's name for the cut ("bore", "hole"), or what it is.
    what: String,
    /// Its size: "35 mm across, 12.8 mm deep".
    value: String,
    /// How many.
    count: usize,
    /// The hardware making them.
    hardware: Vec<stave_model::solved::HardwareId>,
}

/// The declaration behind a part.
fn part_decl(model: &Model, id: PartId) -> Option<&Decl> {
    let instance = model
        .parts
        .get(id.index())
        .filter(|p| p.id == id)
        .or_else(|| model.parts.iter().find(|p| p.id == id))?;
    model.defs.part(instance.def).map(|d| &d.decl)
}

/// The declaration behind a piece of hardware.
fn hardware_decl(model: &Model, id: stave_model::solved::HardwareId) -> Option<&Decl> {
    let instance = model
        .hardware
        .get(id.index())
        .filter(|h| h.id == id)
        .or_else(|| model.hardware.iter().find(|h| h.id == id))?;
    model.defs.hardware_def(instance.def).map(|d| &d.decl)
}

/// A link to a part, named relative to `assembly`.
fn part_link(design: &SolvedDesign, id: PartId, assembly: Option<AssemblyId>) -> NameLink {
    let name = design.part(id).map_or_else(String::new, |p| {
        relative(design, &p.name, assembly).to_owned()
    });
    NameLink {
        name,
        target: LinkTarget::Select(Selection::Part(id)),
    }
}

/// A link to a piece of hardware, named relative to `assembly`.
fn hardware_link(design: &SolvedDesign, id: HardwareId, assembly: Option<AssemblyId>) -> NameLink {
    let name = design
        .hardware()
        .get(id.index())
        .map_or_else(String::new, |h| {
            relative(design, &h.name, assembly).to_owned()
        });
    NameLink {
        name,
        target: LinkTarget::Select(Selection::Hardware(id)),
    }
}

/// A link to a space of the design, by its name; `None` when the design
/// has no space of that name.
pub fn space_link(design: &SolvedDesign, name: &str) -> Option<NameLink> {
    let region = design.regions().iter().find(|r| r.name == name)?;
    Some(NameLink {
        name: name.to_owned(),
        target: LinkTarget::Space(region.id),
    })
}

/// A link to whatever of the design is called `name`: a part, hardware,
/// an assembly or a space; `None` for a name the design doesn't have.
pub fn name_link(design: &SolvedDesign, name: &str) -> Option<NameLink> {
    match crate::link::selection_named(design, name) {
        Some(selection) => Some(NameLink {
            name: name.to_owned(),
            target: LinkTarget::Select(selection),
        }),
        None => space_link(design, name),
    }
}

/// The links in a place param's value as written (`bay[doors]`,
/// `[side_l, side_r]`, `side_l.right`): each name the design has, once,
/// in order. A face (`side_l.right`) links to its part.
pub fn place_links(design: &SolvedDesign, text: &str) -> Vec<NameLink> {
    let inner = text
        .trim()
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .unwrap_or(text.trim());
    let mut links: Vec<NameLink> = Vec::new();
    for name in inner.split(',').map(str::trim).filter(|n| !n.is_empty()) {
        let found = name_link(design, name).or_else(|| {
            // A face: the part before its last `.`.
            let (part, _) = name.rsplit_once('.')?;
            let mut link = name_link(design, part)?;
            link.name = name.to_owned();
            Some(link)
        });
        if let Some(link) = found
            && !links.contains(&link)
        {
            links.push(link);
        }
    }
    links
}

/// "with rail[top], rail[bottom]"; empty for none.
fn with_names(links: &[NameLink]) -> String {
    if links.is_empty() {
        return String::new();
    }
    let names: Vec<&str> = links.iter().map(|link| link.name.as_str()).collect();
    fill_in(&tr("with {names}"), &[("names", &names.join(", "))])
}

/// The hardware mounted on a part, one row per kind and face
/// ("euro_hinge ×2, back face → hinges[lower], hinges[upper]"), then for
/// hinges the parts they hang it on ("hinged on → side_l").
fn hardware_on(design: &SolvedDesign, part: &Part) -> Vec<Connection> {
    let mut groups: Vec<(String, String, Vec<HardwareId>)> = Vec::new();
    let mut hinged_on: Vec<NameLink> = Vec::new();
    let mut first_hinge = None;
    for hardware in design.hardware().iter().filter(|h| h.piece == part.id) {
        let kind = hardware
            .kind
            .rsplit('.')
            .next()
            .unwrap_or(&hardware.kind)
            .to_owned();
        let face = fill_in(&tr("{face} face"), &[("face", &hardware.mount_face.word())]);
        match groups.iter_mut().find(|(k, f, _)| *k == kind && *f == face) {
            Some((_, _, ids)) => ids.push(hardware.id),
            None => groups.push((kind, face, vec![hardware.id])),
        }
        if hardware.base == HardwareBase::Hinge {
            first_hinge.get_or_insert(hardware.id);
            for (_, value) in &hardware.roles {
                if let RoleValue::Part(other) = value
                    && *other != part.id
                    && design.part(*other).is_some()
                {
                    let link = part_link(design, *other, part.assembly);
                    if !hinged_on.contains(&link) {
                        hinged_on.push(link);
                    }
                }
            }
        }
    }
    let mut rows: Vec<Connection> = groups
        .into_iter()
        .map(|(kind, face, ids)| {
            let partners: Vec<NameLink> = ids
                .iter()
                .map(|id| hardware_link(design, *id, part.assembly))
                .collect();
            Connection {
                swatch: hardware_swatch(),
                lead: if ids.len() == 1 {
                    kind
                } else {
                    format!("{kind} ×{}", ids.len())
                },
                rest: fill_in(
                    &tr("on its {face}: {names}"),
                    &[("face", &face), ("names", &with_names(&partners))],
                ),
                partners,
                note: face,
                section: ConnectionSection::Joined,
                target: ids.first().map(|id| Target::Hardware(*id)),
            }
        })
        .collect();
    if !hinged_on.is_empty() {
        rows.push(Connection {
            swatch: hardware_swatch(),
            lead: tr("hinged on"),
            rest: with_names(&hinged_on),
            partners: hinged_on,
            note: String::new(),
            section: ConnectionSection::Joined,
            target: first_hinge.map(Target::Hardware),
        });
    }
    rows
}

/// The checks that fail about an item (errors and warnings), in `stave
/// check` order.
fn failing_checks(checked: &Checked, item: Item) -> Vec<FailingCheck> {
    checked
        .problems
        .iter()
        .enumerate()
        .filter(|(_, problem)| matches!(problem.severity, Severity::Error | Severity::Warning))
        .filter(|(_, problem)| {
            problem.contact.as_ref().is_some_and(|contact| {
                contact
                    .items
                    .iter()
                    .any(|found| crate::view::contact_item(*found) == item)
            })
        })
        .map(|(index, problem)| FailingCheck {
            severity: problem.severity,
            message: problem.message.clone(),
            problem: ProblemIndex(index),
        })
        .collect()
}

fn inspect_part(checked: &Checked, design: &SolvedDesign, id: PartId) -> Option<Inspection> {
    let part = design.part(id)?;
    let kind = match part.origin {
        _ if part.pieces.is_some() => "PIECE OF A SPLICED PART",
        PartOrigin::Piece { .. } => "PIECE OF A SPLICED PART",
        PartOrigin::Declared => "PART",
        PartOrigin::Mirror { .. } | PartOrigin::MirroredAssembly { .. } => "MIRROR IMAGE",
        PartOrigin::Made { .. } | PartOrigin::Boards { .. } => "MADE PART",
    };
    let model = checked.opened.model.as_ref();
    // A piece of a spliced part is declared as its part.
    let declared = match part.origin {
        PartOrigin::Piece { part: first, .. } => first,
        PartOrigin::Declared
        | PartOrigin::Mirror { .. }
        | PartOrigin::MirroredAssembly { .. }
        | PartOrigin::Made { .. }
        | PartOrigin::Boards { .. } => id,
    };
    let description = model.and_then(|m| part_decl(m, declared)).and_then(docs);
    let thickness = match part.shape {
        Shape::Round { diameter } => fill_in(&tr("{length} across"), &[("length", &mm(diameter))]),
        Shape::Board | Shape::Turned { .. } => part
            .stock
            .board_thickness()
            .map_or_else(|| mm(part.cut.thickness), mm),
    };
    let mut rows = vec![
        Row {
            label: tr_noop("Stock"),
            value: format!(
                "{} · {}, {thickness}",
                part.stock.name,
                part.stock.species.title()
            ),
        },
        Row {
            label: tr_noop("Finished size"),
            value: format!(
                "{} × {} × {}",
                mm(part.cut.length).trim_end_matches(" mm"),
                mm(part.cut.width).trim_end_matches(" mm"),
                mm(part.cut.thickness)
            ),
        },
        Row {
            label: tr_noop("Grain"),
            value: fill_in(
                &tr("along the {direction}"),
                &[("direction", &part.grain.word())],
            ),
        },
    ];
    if let Shape::Turned { profile, .. } = &part.shape {
        rows.push(Row {
            label: tr_noop("Shape"),
            value: fill_in(&tr("turned to {profile}"), &[("profile", &profile)]),
        });
    }
    let mut rows: Vec<Field> = rows.into_iter().map(Field::from).collect();
    if let PartOrigin::Mirror { source } | PartOrigin::MirroredAssembly { source } = part.origin
        && let Some(source) = design.part(source)
    {
        rows.push(Field {
            label: tr_noop("Mirror of"),
            value: source.name.clone(),
            links: vec![part_link(design, source.id, part.assembly)],
        });
    }
    let moves_with = moving(design, part.assembly);
    rows.extend(moves_row(design, moves_with).map(Field::from));
    if let Some(note) = &part.note {
        rows.push(Field {
            label: tr_noop("Note"),
            value: note.clone(),
            links: Vec::new(),
        });
    }
    let mut connections = Vec::new();
    // One row per joint: its kind and the other parts in it.
    for joint in design.joints().iter().filter(|j| {
        j.pairs
            .iter()
            .any(|p| p.roles.iter().any(|r| r.part == id) || p.made_parts.contains(&id))
    }) {
        let mut others: Vec<NameLink> = Vec::new();
        let mut first_pair = None;
        for (index, pair) in joint.indexed_pairs() {
            if !(pair.roles.iter().any(|r| r.part == id) || pair.made_parts.contains(&id)) {
                continue;
            }
            first_pair.get_or_insert(index);
            for role in &pair.roles {
                if role.part != id && design.part(role.part).is_some() {
                    let link = part_link(design, role.part, part.assembly);
                    if !others.contains(&link) {
                        others.push(link);
                    }
                }
            }
        }
        connections.push(Connection {
            swatch: part_swatch(part),
            lead: joint
                .kind
                .rsplit('.')
                .next()
                .unwrap_or(&joint.kind)
                .to_owned(),
            rest: with_names(&others),
            partners: others,
            note: String::new(),
            section: ConnectionSection::Joined,
            target: first_pair.map(|pair| Target::Joint {
                joint: joint.id,
                pair,
            }),
        });
    }
    connections.extend(hardware_on(design, part));
    // The hardware's cuts, grouped by hardware kind and cut.
    let mut cuts: Vec<HardwareCut> = Vec::new();
    for cut in &part.cuts {
        let CutSource::Hardware { hardware } = cut.source else {
            continue;
        };
        if design.hardware_piece(hardware).is_none() {
            continue;
        }
        let row = focus::cut_row(&cut.kind);
        // The name the kind gives the cut (`cut bore: ...`), or else what
        // the cut is.
        let what = match &cut.name {
            Some(name) => name.as_str().replace('_', " "),
            None => row.label.to_lowercase(),
        };

        match cuts
            .iter_mut()
            .find(|cut| cut.what == what && cut.value == row.value)
        {
            Some(cut) => {
                cut.count = cut.count.saturating_add(1);
                if !cut.hardware.contains(&hardware) {
                    cut.hardware.push(hardware);
                }
            }
            None => cuts.push(HardwareCut {
                what,
                value: row.value,
                count: 1,
                hardware: vec![hardware],
            }),
        }
    }
    for cut in cuts {
        let lead = if cut.count == 1 {
            format!("1 {}", cut.what)
        } else {
            format!("{} {}s", cut.count, cut.what)
        };
        connections.push(Connection {
            swatch: hardware_swatch(),
            lead,
            rest: format!("· {}", cut.value),
            partners: Vec::new(),
            note: cut.value,
            section: ConnectionSection::Cuts,
            target: cut.hardware.first().map(|h| Target::Hardware(*h)),
        });
    }
    let source = source_of(checked, |m| part_decl(m, declared), part.assembly);
    Some(Inspection {
        eyebrow: eyebrow(design, kind, part.assembly),
        kind: kind_title(kind),
        within: within(design, part.assembly),
        title: relative(design, &part.name, part.assembly).to_owned(),
        description,
        swatch: Some(part_swatch(part)),
        checks: failing_checks(checked, Item::Part(id)),
        rows,
        connections_title: tr_noop("Joints and cuts"),
        connections,
        moves_with,
        source,
    })
}

fn inspect_hardware(
    checked: &Checked,
    design: &SolvedDesign,
    id: stave_model::solved::HardwareId,
) -> Option<Inspection> {
    let hardware = design.hardware_piece(id)?;
    let model = checked.opened.model.as_ref();
    let description = model
        .and_then(|m| m.defs.kind(hardware.kind_id))
        .and_then(|k| docs(&k.decl));
    let piece = design.part(hardware.piece).map_or("?", |p| p.name.as_str());
    let what = match hardware.base {
        HardwareBase::Hinge => tr("hinge"),
        HardwareBase::Slide => tr("drawer slide"),
        HardwareBase::Pull => tr("pull"),
    };
    let mut rows = vec![
        Field::from(Row {
            label: tr_noop("Kind"),
            value: format!("{} ({what})", hardware.kind),
        }),
        Field {
            label: tr_noop("Mounted on"),
            value: fill_in(
                &tr("{piece}, {face} face"),
                &[("piece", &piece), ("face", &hardware.mount_face.word())],
            ),
            links: vec![part_link(design, hardware.piece, None)],
        },
    ];
    let moves_with = moving(design, hardware.assembly);
    rows.extend(moves_row(design, moves_with).map(Field::from));
    let connections = vec![Connection {
        swatch: hardware_swatch(),
        lead: tr("See how it works"),
        rest: String::new(),
        partners: Vec::new(),
        note: String::new(),
        section: ConnectionSection::HowItWorks,
        target: Some(Target::Hardware(id)),
    }];
    let source = source_of(checked, |m| hardware_decl(m, id), hardware.assembly);
    Some(Inspection {
        eyebrow: eyebrow(design, "HARDWARE", hardware.assembly),
        kind: kind_title("HARDWARE"),
        within: within(design, hardware.assembly),
        title: relative(design, &hardware.name, hardware.assembly).to_owned(),
        description,
        swatch: Some(hardware_swatch()),
        checks: failing_checks(checked, Item::Hardware(id)),
        rows,
        connections_title: tr_noop("How it works"),
        connections,
        moves_with,
        source,
    })
}

fn inspect_assembly(
    checked: &Checked,
    design: &SolvedDesign,
    id: AssemblyId,
) -> Option<Inspection> {
    let assembly = design.assembly(id)?;
    let model = checked.opened.model.as_ref();
    let instance = model.and_then(|m| m.assemblies.get(id.index()).filter(|a| a.id == id));
    let description = model
        .zip(instance)
        .and_then(|(m, a)| m.defs.component(a.component))
        .and_then(|c| docs(&c.decl));
    let mut rows = vec![Row {
        label: tr_noop("Component"),
        value: assembly.component.clone(),
    }];
    if let Some(motion) = &assembly.mechanism {
        rows.push(Row {
            label: tr_noop("Moves"),
            value: motion_words(motion),
        });
    }
    let mut rows: Vec<Field> = rows.into_iter().map(Field::from).collect();
    if let Some(source) = assembly.mirror_of.and_then(|s| design.assembly(s)) {
        rows.push(Field {
            label: tr_noop("Mirror of"),
            value: source.name.clone(),
            links: vec![NameLink {
                name: source.name.clone(),
                target: LinkTarget::Select(Selection::Assembly(source.id)),
            }],
        });
    }
    rows.push(Field::from(Row {
        label: tr_noop("Inside"),
        value: fill_in(
            &ntr(
                "{parts} part, {hardware} of hardware",
                "{parts} parts, {hardware} of hardware",
                assembly.parts.len(),
            ),
            &[
                ("parts", &assembly.parts.len()),
                ("hardware", &assembly.hardware.len()),
            ],
        ),
    }));
    let moves_with = moving(design, Some(id));
    let source = source_of(
        checked,
        |model| {
            let instance = model.assemblies.get(id.index()).filter(|a| a.id == id)?;
            model.defs.assembly(instance.def).map(|d| &d.decl)
        },
        assembly.parent,
    );
    Some(Inspection {
        eyebrow: eyebrow(design, "ASSEMBLY", assembly.parent),
        kind: kind_title("ASSEMBLY"),
        within: within(design, assembly.parent),
        title: relative(design, &assembly.name, assembly.parent).to_owned(),
        description,
        swatch: None,
        checks: Vec::new(),
        rows,
        connections_title: tr_noop("Joints and cuts"),
        connections: Vec::new(),
        moves_with,
        source,
    })
}

/// The declaration of a joint, or of the part a splice joins the pieces
/// of (its `lengths:` makes the splice).
fn joint_or_splice_decl<'m>(
    model: &'m Model,
    joint: &stave_model::solved::Joint,
) -> Option<&'m Decl> {
    match joint.origin {
        stave_model::solved::JointOrigin::Splice { part, .. } => part_decl(model, part),
        stave_model::solved::JointOrigin::Declared => joint_decl(model, joint.id),
    }
}

/// The declaration of a joint.
fn joint_decl(model: &Model, id: stave_model::solved::JointId) -> Option<&Decl> {
    let instance = model.joints.get(id.index()).filter(|j| j.id == id)?;
    model.defs.joint(instance.def).map(|d| &d.decl)
}

/// A joint pair's member (a dowel's pins): which joint placed it and
/// where that joint is written, what it is and how many copies.
fn inspect_member(
    checked: &Checked,
    design: &SolvedDesign,
    member: PairMember,
) -> Option<Inspection> {
    let joint = design.joint(member.joint)?;
    let body = design.pair_member(member)?;
    let model = checked.opened.model.as_ref();
    // The member's own doc comment in its kind (`/// The pins: ...`).
    let description = model
        .and_then(|m| m.defs.kind(joint.kind_id))
        .and_then(|kind| kind.bodies.get(member.body.index()))
        .and_then(|def| {
            let text = def.docs.join(" ");
            let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
            (!text.is_empty()).then_some(text)
        });
    let target = Target::Joint {
        joint: member.joint,
        pair: member.pair,
    };
    let mut rows = vec![
        Row {
            label: tr_noop("Placed by"),
            value: format!("{} ({})", joint.name, joint.kind),
        },
        Row {
            label: tr_noop("Pair"),
            value: focus::title(design, target),
        },
    ];
    rows.extend(body.item.as_ref().map(|item| Row {
        label: tr_noop("Fastener"),
        value: crate::view::item_text(item),
    }));
    rows.push(Row {
        label: tr_noop("Copies"),
        value: body.copies().to_string(),
    });
    rows.push(Row {
        label: tr_noop("Each"),
        value: crate::view::copy_text(body),
    });
    let mut rows: Vec<Field> = rows.into_iter().map(Field::from).collect();
    let carrier = body.carried_by().and_then(|part| design.part(part));
    if let Some(part) = carrier {
        rows.push(Field {
            label: tr_noop("Goes in with"),
            value: part.name.clone(),
            links: vec![part_link(design, part.id, None)],
        });
    }
    let assembly = carrier.and_then(|part| part.assembly);
    let moves_with = moving(design, assembly);
    rows.extend(moves_row(design, moves_with).map(Field::from));
    let source = source_of(checked, |m| joint_or_splice_decl(m, joint), joint.assembly);
    Some(Inspection {
        eyebrow: eyebrow(design, "JOINT MEMBER", assembly),
        kind: kind_title("JOINT MEMBER"),
        within: within(design, assembly),
        title: format!(
            "{}'s {}",
            relative(design, &joint.name, joint.assembly),
            body.name
        ),
        description,
        swatch: Some(member_swatch(body)),
        checks: failing_checks(checked, Item::Member(member)),
        rows,
        connections_title: tr_noop("How it goes together"),
        connections: vec![Connection {
            swatch: member_swatch(body),
            lead: tr("See how it goes together"),
            rest: String::new(),
            partners: Vec::new(),
            note: String::new(),
            section: ConnectionSection::HowItWorks,
            target: Some(target),
        }],
        moves_with,
        source,
    })
}

fn inspect_joint(
    checked: &Checked,
    design: &SolvedDesign,
    id: stave_model::solved::JointId,
    pair: PairIndex,
) -> Option<Inspection> {
    let joint = design.joint(id)?;
    let model = checked.opened.model.as_ref();
    let description = model
        .and_then(|m| m.defs.kind(joint.kind_id))
        .and_then(|k| docs(&k.decl));
    let target = Target::Joint { joint: id, pair };
    let rows = vec![
        Row {
            label: tr_noop("Kind"),
            value: joint.kind.clone(),
        },
        Row {
            label: tr_noop("Pair"),
            value: focus::title(design, target),
        },
    ];
    let source = source_of(
        checked,
        |model| joint_or_splice_decl(model, joint),
        joint.assembly,
    );
    Some(Inspection {
        eyebrow: eyebrow(design, "JOINT", joint.assembly),
        kind: kind_title("JOINT"),
        within: within(design, joint.assembly),
        title: relative(design, &joint.name, joint.assembly).to_owned(),
        description,
        swatch: None,
        checks: Vec::new(),
        rows: rows.into_iter().map(Field::from).collect(),
        connections_title: tr_noop("How it goes together"),
        connections: vec![Connection {
            swatch: hardware_swatch(),
            lead: tr("See how it goes together"),
            rest: String::new(),
            partners: Vec::new(),
            note: String::new(),
            section: ConnectionSection::HowItWorks,
            target: Some(target),
        }],
        moves_with: None,
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_are_labelled_compactly() {
        let labels =
            |l: &[&str]| group_label(&l.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>());
        assert_eq!(labels(&["stile[hinge]"]), "stile[hinge]");
        assert_eq!(
            labels(&["feet[0]", "feet[1]", "feet[2]", "feet[3]"]),
            "feet[4]"
        );
        assert_eq!(
            labels(&["hinges[lower]", "hinges[upper]"]),
            "hinges[lower, upper]"
        );
        assert_eq!(labels(&["side_l", "side_r"]), "side_l, side_r");
    }

    #[test]
    fn snippets_are_coloured_by_token() {
        let tokens = highlight(
            "assembly door_l : casework.FramePanelDoor {\n  mount: overlay(left: 18mm)\n}",
        );
        let class_of = |text: &str| {
            tokens
                .iter()
                .find(|t| t.text.contains(text))
                .map(|t| t.class)
        };
        assert_eq!(class_of("assembly"), Some(TokenClass::Keyword));
        assert_eq!(class_of("FramePanelDoor"), Some(TokenClass::Type));
        assert_eq!(class_of("overlay"), Some(TokenClass::Function));
        assert_eq!(class_of("18mm"), Some(TokenClass::Number));
        let text: String = tokens.iter().map(|t| t.text.as_str()).collect();
        assert!(text.starts_with("assembly door_l"));
        assert!(text.ends_with('}'));
    }

    #[test]
    fn angles_read_in_degrees() {
        assert_eq!(degrees(Angle::from_deg(110)), "110°");
    }
}
