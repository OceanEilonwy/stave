//! The Parts panel's joint chips, worked out without any UI: for every
//! part, the joints it is in grouped by kind with a count (`dowel 8`,
//! `dado 2`) and the hardware fitted to it; for the selected part, what it
//! is joined to (the joint kind in words, whether it is glued, the partner
//! parts as things to select, the hardware on it, and the part its
//! assembly turns or slides on); for each assembly, a heading note ("110°
//! on side_l", "12 parts · 6 joints"); which assemblies fold into one row;
//! and what the panel shows at a given width.
//!
//! Everything comes from the solved design: `joints[].pairs[].roles`,
//! each joint's kind and `glue:`, the hardware's `piece`, `mount_face` and
//! roles, and the assemblies' mechanisms. No joint or hardware kind is
//! named here: a chip's words are the kind's own name, and its
//! [`Glyph`] is chosen from what the joint does to the wood (the cuts its
//! kind makes, or the pieces it lays out) or from the hardware's base.
//!
//! **What a count counts.** A chip counts what the part has of that kind:
//! for a joint that lays out pieces (a dowel's pins, a screw joint's
//! screws), the pieces in every pair the part is in (side_l's two dowel
//! joints of four pins each make `dowel 8`); for any other joint, the
//! pairs the part is in (`dado 2`: it meets the divider and the shelf);
//! for hardware, the pieces fitted to it. A row standing for several parts
//! (`rail[bottom, top]`) adds them up. A folded assembly counts each pair
//! once.

use crate::i18n::{fill_in, ntr, tr};
use std::collections::{BTreeMap, BTreeSet};

use stave_model::mechanism::DriveValue;
use stave_model::nonempty::NonEmpty;
use stave_model::solved::{
    AssemblyId, CutKind, CutSource, Hardware, HardwareBase, Joint, PartId, RoleValue, SolvedDesign,
};
use stave_scene::Item;

use crate::focus::Target;
use crate::inspect::{
    self, AssemblyNode, PartsTree, TreeEntry, group_label, relative, split_index,
};
use crate::session::Selection;

/// The small picture on a chip or a joined-to row. Joints get theirs from
/// what they do to the wood, hardware from its base, so a kind a project
/// defines itself gets a fitting one too.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Glyph {
    /// The joint lays out pieces between the parts: pins, screws.
    Pins,
    /// A tenon (square or round) in a mortise or a hole.
    Tenon,
    /// A tenon and a groove together: a frame's joint.
    Frame,
    /// Dovetails.
    Dovetail,
    /// A slot or groove through the middle of a face.
    Slot,
    /// A channel the other part sits in: a dado, a rabbet.
    Channel,
    /// A round hole the other part fits.
    Hole,
    /// A marked-out shape.
    Shaped,
    /// The parts just meet: no cut.
    Butt,
    /// A hinge.
    Hinge,
    /// A drawer slide.
    Slide,
    /// A knob or pull.
    Pull,
    /// A mirror image of another assembly.
    Mirror,
    /// What an assembly turns on.
    Turn,
    /// What an assembly slides in.
    Travel,
}

/// How an assembly moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Motion {
    /// It turns (a door).
    Turn,
    /// It slides (a drawer).
    Travel,
}

/// What a chip's count is of.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Counted {
    /// Joined places (pairs).
    Joints,
    /// The pieces a joint lays out, by the member's name (`pin`).
    Pieces(String),
    /// Pieces of hardware.
    Fitted,
    /// Nothing: the chip only says something (a mirror image).
    Nothing,
}

/// One chip: a joint kind (or hardware kind) with how many.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chip {
    /// Its picture.
    pub glyph: Glyph,
    /// The kind in words: `dowel`, `groove, no glue`, `euro hinge`,
    /// `mirror of door_l`.
    pub kind: String,
    /// How many.
    pub count: u32,
    /// What the count is of.
    pub counted: Counted,
}

impl Chip {
    /// Whether the chip shows its count: only when there is more than one.
    pub fn shows_count(&self) -> bool {
        self.count > 1 && self.counted != Counted::Nothing
    }

    /// Its tooltip: `dowel: 8 pins`, `dado: 2 joints`, `euro hinge × 2`.
    pub fn tooltip(&self) -> String {
        let count = self.count;
        match &self.counted {
            Counted::Joints => fill_in(
                &ntr("{kind}: {count} joint", "{kind}: {count} joints", count),
                &[("kind", &self.kind), ("count", &count)],
            ),
            Counted::Pieces(name) => fill_in(
                &ntr("{kind}: {count} {name}", "{kind}: {count} {name}s", count),
                &[
                    ("kind", &self.kind),
                    ("count", &count),
                    ("name", &name.replace('_', " ")),
                ],
            ),
            Counted::Fitted if count > 1 => fill_in(
                &tr("{kind} × {count}"),
                &[("kind", &self.kind), ("count", &count)],
            ),
            Counted::Fitted | Counted::Nothing => self.kind.clone(),
        }
    }

    fn adds_to(&self, other: &Chip) -> bool {
        self.glyph == other.glyph
            && self.kind == other.kind
            && std::mem::discriminant(&self.counted) == std::mem::discriminant(&other.counted)
    }
}

/// Adds `chip` to `chips`, into a chip of the same kind when there is one.
fn add_chip(chips: &mut Vec<Chip>, chip: Chip) {
    match chips.iter_mut().find(|c| c.adds_to(&chip)) {
        Some(found) => found.count = found.count.saturating_add(chip.count),
        None => chips.push(chip),
    }
}

/// Chips folded into one badge for a narrow panel: how many kinds, and
/// each chip's tooltip on its own line.
pub fn badge(chips: &[Chip]) -> Option<(usize, String)> {
    (!chips.is_empty()).then(|| {
        let lines: Vec<String> = chips.iter().map(Chip::tooltip).collect();
        (chips.len(), lines.join("\n"))
    })
}

/// Another part (or piece of hardware, or assembly, or space) a joined-to
/// row names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Partner {
    /// Its name relative to the selected part's assembly; several alike
    /// together: `rail[bottom, top]`, `hinges[lower, upper]`.
    pub label: String,
    /// What clicking it selects (the first of them); `None` for a space,
    /// which can't be selected.
    pub selection: Option<Selection>,
    /// Everything it stands for, to light in the 3D view and mark in the
    /// list.
    pub items: Vec<Item>,
}

/// One "joined to" row under the selected part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinedTo {
    /// Its picture.
    pub glyph: Glyph,
    /// `cope and stick`, `groove, no glue`, `euro hinge ×2, back face`,
    /// `turns on`.
    pub words: String,
    /// What it is joined to.
    pub partners: Vec<Partner>,
    /// Focus mode on it, when it can be shown.
    pub focus: Option<Target>,
    /// The selections it stands for (its joint's pairs and their pieces,
    /// or its hardware), so choosing one of them marks this row.
    pub stands_for: Vec<Selection>,
}

/// An assembly's (or the carcass's) heading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heading {
    /// "Carcass", or the assembly's name as written: `door_l`. The window
    /// may draw it in capitals; the words stay as written.
    pub title: String,
    /// The grey note: "110° on side_l", or "12 parts · 6 joints".
    pub note: String,
    /// How it moves, for the icon that stays when the note drops.
    pub motion: Option<Motion>,
    /// Its motion in a sentence, for the icon's tooltip: "turns 110° on
    /// side_l".
    pub motion_words: Option<String>,
}

/// What the Parts panel shows about joints, for every part and assembly
/// of one design. Built once per check; selecting only looks things up.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PartJoints {
    chips: BTreeMap<PartId, Vec<Chip>>,
    joined: BTreeMap<PartId, Vec<JoinedTo>>,
    /// The part a piece of hardware or joint piece belongs with.
    homes: BTreeMap<Item, PartId>,
    headings: BTreeMap<Option<AssemblyId>, Heading>,
    folded_chips: BTreeMap<AssemblyId, Vec<Chip>>,
    folds: BTreeMap<AssemblyId, Fold>,
}

/// How an assembly folds.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Fold {
    /// Folded unless opened: a mirror image, or one of a set.
    by_default: bool,
    /// Folded rows next to each other with the same key share one row.
    key: Option<(String, String)>,
    /// "mirror of door_l".
    mirror_of: Option<String>,
}

/// One block of the panel's list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block<'t> {
    /// A heading (`None`: the carcass) and its part rows.
    Heading {
        /// The assembly, or `None` for the carcass.
        assembly: Option<AssemblyId>,
        /// Its part rows.
        entries: Vec<&'t TreeEntry>,
    },
    /// Folded assemblies as one row.
    Folded {
        /// The assemblies it stands for; selecting it selects the first.
        assemblies: NonEmpty<AssemblyId>,
        /// `door_r`, `drawer[bottom, top]`.
        label: String,
        /// "mirror of door_l".
        detail: Option<String>,
    },
}

/// The words for a kind's name: `cope and stick` for
/// `casework.cope_and_stick`.
fn kind_words(kind: &str) -> String {
    kind.rsplit('.').next().unwrap_or(kind).replace('_', " ")
}

/// A joint's kind in words, with `no glue` when it isn't glued.
fn joint_words(joint: &Joint) -> String {
    let words = kind_words(&joint.kind);
    if joint.glue {
        words
    } else {
        fill_in(&tr("{kind}, no glue"), &[("kind", &words)])
    }
}

fn hardware_glyph(hardware: &Hardware) -> Glyph {
    match hardware.base {
        HardwareBase::Hinge => Glyph::Hinge,
        HardwareBase::Slide => Glyph::Slide,
        HardwareBase::Pull => Glyph::Pull,
        HardwareBase::Support | HardwareBase::Fitting => Glyph::Pins,
    }
}

/// A joint's glyph, from the pieces it lays out or the cuts it makes.
fn joint_glyph<'d>(joint: &Joint, cuts: impl Iterator<Item = &'d CutKind>) -> Glyph {
    if joint.pairs.iter().any(|pair| !pair.members.is_empty()) {
        return Glyph::Pins;
    }
    let (mut tenon, mut slot, mut channel, mut hole, mut dovetail, mut shaped) =
        (false, false, false, false, false, false);
    for cut in cuts {
        match cut {
            CutKind::Pocket { .. } | CutKind::Tenon { .. } | CutKind::Spigot { .. } => tenon = true,
            CutKind::Slot { .. } => slot = true,
            CutKind::Notch { .. } => channel = true,
            CutKind::Drill { .. } | CutKind::Round { .. } => hole = true,
            CutKind::DovetailSocket { .. } | CutKind::DovetailTail { .. } => dovetail = true,
            CutKind::Outline { .. }
            | CutKind::Diagonal
            | CutKind::Chamfer { .. }
            | CutKind::Roundover { .. } => shaped = true,
        }
    }
    if dovetail {
        Glyph::Dovetail
    } else if tenon && slot {
        Glyph::Frame
    } else if tenon {
        Glyph::Tenon
    } else if slot {
        Glyph::Slot
    } else if channel {
        Glyph::Channel
    } else if hole {
        Glyph::Hole
    } else if shaped {
        Glyph::Shaped
    } else {
        Glyph::Butt
    }
}

/// Whether `assembly` is `outer` or inside it.
fn inside(design: &SolvedDesign, mut assembly: Option<AssemblyId>, outer: AssemblyId) -> bool {
    for _ in 0..=design.assemblies().len() {
        match assembly {
            Some(found) if found == outer => return true,
            Some(found) => assembly = design.assembly(found).and_then(|a| a.parent),
            None => return false,
        }
    }
    false
}

/// Names grouped the way the parts tree groups them: `rail[bottom, top]`.
fn grouped(labels: Vec<(String, Item)>) -> Vec<Partner> {
    let mut groups: Vec<(String, Vec<(String, Item)>)> = Vec::new();
    for (label, item) in labels {
        let base = split_index(&label).0.to_owned();
        match groups.iter_mut().find(|(b, _)| *b == base) {
            Some((_, members)) => {
                if !members.iter().any(|(l, _)| *l == label) {
                    members.push((label, item));
                }
            }
            None => groups.push((base, vec![(label, item)])),
        }
    }
    groups
        .into_iter()
        .map(|(_, members)| {
            let names: Vec<String> = members.iter().map(|(l, _)| l.clone()).collect();
            let items: Vec<Item> = members.iter().map(|(_, i)| *i).collect();
            Partner {
                label: group_label(&names),
                selection: items.first().copied().map(Selection::of_item),
                items,
            }
        })
        .collect()
}

impl PartJoints {
    /// Works it all out for a design.
    pub fn new(design: &SolvedDesign) -> PartJoints {
        // The cuts each joint makes, by joint index.
        let mut cuts: BTreeMap<usize, Vec<&CutKind>> = BTreeMap::new();
        for part in design.parts() {
            for cut in &part.cuts {
                if let CutSource::Joint { joint, .. } = cut.source {
                    cuts.entry(joint.index()).or_default().push(&cut.kind);
                }
            }
        }
        let glyphs: Vec<Glyph> = design
            .joints()
            .iter()
            .map(|joint| {
                joint_glyph(
                    joint,
                    cuts.get(&joint.id.index()).into_iter().flatten().copied(),
                )
            })
            .collect();
        let mut found = PartJoints::default();
        found.part_chips(design, &glyphs);
        for part in design.parts() {
            let rows = joined_to(design, &glyphs, part.id);
            found.joined.insert(part.id, rows);
        }
        for hardware in design.hardware() {
            found
                .homes
                .insert(Item::Hardware(hardware.id), hardware.piece);
        }
        for (member, body) in design.pair_members() {
            let home = body.carried_by().or_else(|| {
                design
                    .joints()
                    .get(member.joint.index())?
                    .pair(member.pair)?
                    .roles
                    .first()
                    .map(|role| role.part)
            });
            if let Some(home) = home {
                found.homes.insert(Item::Member(member), home);
            }
        }
        found.headings.insert(None, carcass_heading(design));
        for assembly in design.assemblies() {
            found
                .headings
                .insert(Some(assembly.id), assembly_heading(design, assembly.id));
            found
                .folded_chips
                .insert(assembly.id, folded_chips(design, &glyphs, assembly.id));
            let mirror_of = assembly
                .mirror_of
                .and_then(|source| design.assembly(source))
                .map(|source| fill_in(&tr("mirror of {name}"), &[("name", &source.name)]));
            let (base, index) = split_index(&assembly.name);
            found.folds.insert(
                assembly.id,
                Fold {
                    by_default: mirror_of.is_some() || index.is_some(),
                    key: (mirror_of.is_none() && index.is_some())
                        .then(|| (assembly.component.clone(), base.to_owned())),
                    mirror_of,
                },
            );
        }
        found
    }

    fn part_chips(&mut self, design: &SolvedDesign, glyphs: &[Glyph]) {
        for (joint, glyph) in design.joints().iter().zip(glyphs) {
            let words = joint_words(joint);
            for pair in &joint.pairs {
                let pieces: u32 = pair.members.iter().map(|body| body.copies()).sum();
                let counted = match pair.members.first() {
                    Some(body) => Counted::Pieces(body.name.clone()),
                    None => Counted::Joints,
                };
                let count = if pair.members.is_empty() { 1 } else { pieces };
                let mut parts: Vec<PartId> = pair.roles.iter().map(|r| r.part).collect();
                parts.extend(pair.made_parts.iter().copied());
                let mut seen = BTreeSet::new();
                for part in parts {
                    if !seen.insert(part) {
                        continue;
                    }
                    add_chip(
                        self.chips.entry(part).or_default(),
                        Chip {
                            glyph: *glyph,
                            kind: words.clone(),
                            count,
                            counted: counted.clone(),
                        },
                    );
                }
            }
        }
        for hardware in design.hardware() {
            add_chip(
                self.chips.entry(hardware.piece).or_default(),
                Chip {
                    glyph: hardware_glyph(hardware),
                    kind: kind_words(&hardware.kind),
                    count: 1,
                    counted: Counted::Fitted,
                },
            );
        }
    }

    /// The chips of a row standing for `items` (its parts' chips added
    /// up); hardware and joint pieces have none.
    pub fn chips(&self, items: &[Item]) -> Vec<Chip> {
        let mut chips = Vec::new();
        for item in items {
            if let Item::Part(part) = item {
                for chip in self.chips.get(part).into_iter().flatten() {
                    add_chip(&mut chips, chip.clone());
                }
            }
        }
        chips
    }

    /// The chips of a folded row standing for `assemblies`.
    pub fn folded_chips(&self, assemblies: &[AssemblyId]) -> Vec<Chip> {
        let mut chips = Vec::new();
        for assembly in assemblies {
            match self.folds.get(assembly).and_then(|f| f.mirror_of.clone()) {
                // A mirror image's joints are its source's: it says so.
                Some(mirror_of) => add_chip(
                    &mut chips,
                    Chip {
                        glyph: Glyph::Mirror,
                        kind: mirror_of,
                        count: 1,
                        counted: Counted::Nothing,
                    },
                ),
                None => {
                    for chip in self.folded_chips.get(assembly).into_iter().flatten() {
                        add_chip(&mut chips, chip.clone());
                    }
                }
            }
        }
        chips
    }

    /// What `part` is joined to, in order: its joints (in the design's
    /// order), the hardware fitted to it, hardware elsewhere that names
    /// it, and what its assembly turns or slides on.
    pub fn joined_to(&self, part: PartId) -> &[JoinedTo] {
        self.joined.get(&part).map_or(&[], Vec::as_slice)
    }

    /// The part whose joined-to rows show for `item`: itself for a part,
    /// the part a piece of hardware is fitted to, the part a joint's
    /// pieces sit in.
    pub fn home(&self, item: Item) -> Option<PartId> {
        match item {
            Item::Part(part) => Some(part),
            Item::Hardware(_) | Item::Member(_) => self.homes.get(&item).copied(),
        }
    }

    /// The heading of an assembly, or of the carcass (`None`).
    pub fn heading(&self, assembly: Option<AssemblyId>) -> Option<&Heading> {
        self.headings.get(&assembly)
    }

    /// Whether an assembly shows as one folded row unless it is opened: a
    /// mirror image (`door_r`) or one of a set (`drawer[top]`).
    pub fn folded_by_default(&self, assembly: AssemblyId) -> bool {
        self.folds.get(&assembly).is_some_and(|f| f.by_default)
    }

    /// The panel's blocks for `tree`: the carcass's heading and part rows,
    /// then each assembly (depth first) as a heading and its part rows when
    /// `open`, or else as a folded row; folded rows of one set next to each
    /// other share a row (`drawer[bottom, top]`). Hardware and joint
    /// pieces have no rows of their own: they show as chips and joined-to
    /// rows.
    pub fn blocks<'t>(
        &self,
        tree: &'t PartsTree,
        open: &dyn Fn(AssemblyId) -> bool,
    ) -> Vec<Block<'t>> {
        let mut blocks = Vec::new();
        let carcass: Vec<&TreeEntry> = tree.carcass.iter().filter(|e| is_part_row(e)).collect();
        if !carcass.is_empty() {
            blocks.push(Block::Heading {
                assembly: None,
                entries: carcass,
            });
        }
        let mut stack: Vec<&AssemblyNode> = tree.assemblies.iter().rev().collect();
        // The fold key of the folded row last pushed, to share it.
        let mut last_key: Option<(String, String)> = None;
        while let Some(node) = stack.pop() {
            if open(node.id) {
                last_key = None;
                blocks.push(Block::Heading {
                    assembly: Some(node.id),
                    entries: node.entries.iter().filter(|e| is_part_row(e)).collect(),
                });
                stack.extend(node.children.iter().rev());
                continue;
            }
            let fold = self.folds.get(&node.id);
            let key = fold.and_then(|f| f.key.clone());
            let shares = key.is_some() && key == last_key;
            match blocks.last_mut() {
                Some(Block::Folded {
                    assemblies, label, ..
                }) if shares => {
                    assemblies.push(node.id);
                    let labels: Vec<String> = assemblies
                        .iter()
                        .filter_map(|id| find(&tree.assemblies, *id))
                        .map(|n| n.label.clone())
                        .collect();
                    *label = group_label(&labels);
                }
                _ => blocks.push(Block::Folded {
                    assemblies: NonEmpty::one(node.id),
                    label: node.label.clone(),
                    detail: fold.and_then(|f| f.mirror_of.clone()),
                }),
            }
            last_key = key;
        }
        blocks
    }
}

/// The node for an assembly, anywhere in `nodes`.
fn find(nodes: &[AssemblyNode], id: AssemblyId) -> Option<&AssemblyNode> {
    let mut stack: Vec<&AssemblyNode> = nodes.iter().collect();
    while let Some(node) = stack.pop() {
        if node.id == id {
            return Some(node);
        }
        stack.extend(node.children.iter());
    }
    None
}

/// Whether a parts-tree row is a part's (not hardware's or a joint
/// piece's).
pub fn is_part_row(entry: &TreeEntry) -> bool {
    matches!(entry.items.first(), Item::Part(_))
}

/// The joined-to rows of one part.
fn joined_to(design: &SolvedDesign, glyphs: &[Glyph], id: PartId) -> Vec<JoinedTo> {
    let Some(part) = design.part(id) else {
        return Vec::new();
    };
    let label = |name: &str| relative(design, name, part.assembly).to_owned();
    let mut rows = Vec::new();
    for (joint, glyph) in design.joints().iter().zip(glyphs) {
        let mut partners = Vec::new();
        let mut stands_for = Vec::new();
        let mut focus = None;
        for (index, pair) in joint.indexed_pairs() {
            let in_pair = pair.roles.iter().any(|r| r.part == id) || pair.made_parts.contains(&id);
            if !in_pair {
                continue;
            }
            focus.get_or_insert(Target::Joint {
                joint: joint.id,
                pair: index,
            });
            stands_for.push(Selection::Joint {
                joint: joint.id,
                pair: index,
            });
            for body in &pair.members {
                stands_for.push(Selection::Member(stave_model::solved::PairMember {
                    joint: joint.id,
                    pair: index,
                    body: body.id.body,
                }));
            }
            for other in pair
                .roles
                .iter()
                .map(|r| r.part)
                .chain(pair.made_parts.iter().copied())
            {
                if other != id
                    && let Some(other) = design.part(other)
                {
                    partners.push((label(&other.name), Item::Part(other.id)));
                }
            }
        }
        if focus.is_some() {
            rows.push(JoinedTo {
                glyph: *glyph,
                words: joint_words(joint),
                partners: grouped(partners),
                focus,
                stands_for,
            });
        }
    }
    // Hardware fitted to it, by kind and face; then hardware fitted
    // elsewhere that names it (a hinge's other leaf), by kind.
    let mut fitted: Vec<(String, Glyph, String, Vec<&Hardware>)> = Vec::new();
    let mut naming: Vec<(String, Glyph, Vec<&Hardware>)> = Vec::new();
    for hardware in design.hardware() {
        let kind = kind_words(&hardware.kind);
        if hardware.piece == id {
            let face = hardware.mount_face.word().to_owned();
            match fitted
                .iter_mut()
                .find(|(k, _, f, _)| *k == kind && *f == face)
            {
                Some((_, _, _, found)) => found.push(hardware),
                None => fitted.push((kind, hardware_glyph(hardware), face, vec![hardware])),
            }
        } else if hardware
            .roles
            .iter()
            .any(|(_, value)| *value == RoleValue::Part(id))
        {
            match naming.iter_mut().find(|(k, _, _)| *k == kind) {
                Some((_, _, found)) => found.push(hardware),
                None => naming.push((kind, hardware_glyph(hardware), vec![hardware])),
            }
        }
    }
    let hardware_row = |words: String, glyph: Glyph, pieces: &[&Hardware]| JoinedTo {
        glyph,
        words,
        partners: grouped(
            pieces
                .iter()
                .map(|h| (label(&h.name), Item::Hardware(h.id)))
                .collect(),
        ),
        focus: pieces.first().map(|h| Target::Hardware(h.id)),
        stands_for: pieces.iter().map(|h| Selection::Hardware(h.id)).collect(),
    };
    for (kind, glyph, face, pieces) in &fitted {
        let times = match pieces.len() {
            0 | 1 => String::new(),
            many => format!(" ×{many}"),
        };
        rows.push(hardware_row(
            fill_in(
                &tr("{kind}{times}, {face} face"),
                &[("kind", kind), ("times", &times), ("face", face)],
            ),
            *glyph,
            pieces,
        ));
    }
    for (kind, glyph, pieces) in &naming {
        let times = match pieces.len() {
            0 | 1 => String::new(),
            many => format!(" ×{many}"),
        };
        rows.push(hardware_row(format!("{kind}{times}"), *glyph, pieces));
    }
    // What its assembly moves on, when hardware fitted to it moves it.
    if let Some((motion, outside)) = moves_on(design, part.assembly, Some(id)) {
        let words = match motion {
            Motion::Turn => tr("turns on"),
            Motion::Travel => tr("slides in"),
        };
        rows.push(JoinedTo {
            glyph: match motion {
                Motion::Turn => Glyph::Turn,
                Motion::Travel => Glyph::Travel,
            },
            words: words.to_owned(),
            partners: outside
                .into_iter()
                .map(|(name, item)| Partner {
                    label: label(&name),
                    selection: item.map(Selection::of_item),
                    items: item.into_iter().collect(),
                })
                .collect(),
            focus: None,
            stands_for: Vec::new(),
        });
    }
    rows
}

/// A part (with its item) or a space (with none) outside an assembly that
/// its moving hardware names.
type Outside = (String, Option<Item>);

/// How the assembly moves and what it moves on: the parts (and spaces)
/// outside it that its moving hardware names, in order, each once. With
/// `piece`, only the hardware fitted to that part counts. `None` when
/// nothing moves it (or, with `piece`, none of its moving hardware is on
/// that part).
fn moves_on(
    design: &SolvedDesign,
    assembly: Option<AssemblyId>,
    piece: Option<PartId>,
) -> Option<(Motion, Vec<Outside>)> {
    let id = assembly?;
    let mechanism = design.assembly(id)?.mechanism.as_ref()?;
    let motion = match mechanism.drive().extent() {
        DriveValue::Turn(_) => Motion::Turn,
        DriveValue::Travel(_) => Motion::Travel,
    };
    let mut outside: Vec<Outside> = Vec::new();
    let mut any = false;
    for hardware in mechanism
        .hardware()
        .iter()
        .filter_map(|h| design.hardware_piece(*h))
        .filter(|h| piece.is_none_or(|p| h.piece == p))
    {
        any = true;
        for (_, value) in &hardware.roles {
            let named = match *value {
                RoleValue::Part(part) => design
                    .part(part)
                    .filter(|p| !inside(design, p.assembly, id))
                    .map(|p| (p.name.clone(), Some(Item::Part(p.id)))),
                RoleValue::Region(region) => design
                    .regions()
                    .get(region.index())
                    .filter(|r| !inside(design, r.assembly, id))
                    .map(|r| (r.name.clone(), None)),
            };
            if let Some(named) = named
                && !outside.contains(&named)
            {
                outside.push(named);
            }
        }
    }
    any.then_some((motion, outside))
}

fn carcass_heading(design: &SolvedDesign) -> Heading {
    let parts = design
        .parts()
        .iter()
        .filter(|p| p.assembly.is_none())
        .count();
    let joints = design
        .joints()
        .iter()
        .filter(|j| j.assembly.is_none())
        .count();
    Heading {
        title: tr("Carcass"),
        note: counts_note(parts, joints),
        motion: None,
        motion_words: None,
    }
}

fn counts_note(parts: usize, joints: usize) -> String {
    let parts = fill_in(
        &ntr("{count} part", "{count} parts", parts),
        &[("count", &parts)],
    );
    let joints = fill_in(
        &ntr("{count} joint", "{count} joints", joints),
        &[("count", &joints)],
    );
    fill_in(
        &tr("{parts} · {joints}"),
        &[("parts", &parts), ("joints", &joints)],
    )
}

fn assembly_heading(design: &SolvedDesign, id: AssemblyId) -> Heading {
    let name = design
        .assemblies()
        .get(id.index())
        .map_or_else(String::new, |a| a.name.clone());
    let parts = design
        .parts()
        .iter()
        .filter(|p| p.assembly == Some(id))
        .count();
    let joints = design
        .joints()
        .iter()
        .filter(|j| j.assembly == Some(id))
        .count();
    let extent = design
        .assemblies()
        .get(id.index())
        .and_then(|a| a.mechanism.as_ref())
        .map(|m| m.drive().extent());
    let moves = moves_on(design, Some(id), None);
    let (note, motion_words) = match (extent, moves) {
        (Some(extent), Some((_, outside))) => {
            let names: Vec<&str> = outside.iter().map(|(n, _)| n.as_str()).collect();
            let turns = matches!(extent, DriveValue::Turn(_));
            let amount = match extent {
                DriveValue::Turn(angle) => inspect::degrees(angle),
                DriveValue::Travel(travel) => inspect::mm(travel),
            };
            if names.is_empty() {
                let words = fill_in(
                    &if turns {
                        tr("turns {amount}")
                    } else {
                        tr("slides {amount}")
                    },
                    &[("amount", &amount)],
                );
                (words.clone(), Some(words))
            } else {
                let names = names.join(", ");
                let values: [(&str, &dyn std::fmt::Display); 2] =
                    [("amount", &amount), ("names", &names)];
                let (note, words) = if turns {
                    (tr("{amount} on {names}"), tr("turns {amount} on {names}"))
                } else {
                    (tr("{amount} in {names}"), tr("slides {amount} in {names}"))
                };
                (fill_in(&note, &values), Some(fill_in(&words, &values)))
            }
        }
        _ => (counts_note(parts, joints), None),
    };
    Heading {
        title: name,
        note,
        motion: extent.map(|extent| match extent {
            DriveValue::Turn(_) => Motion::Turn,
            DriveValue::Travel(_) => Motion::Travel,
        }),
        motion_words,
    }
}

fn folded_chips(design: &SolvedDesign, glyphs: &[Glyph], id: AssemblyId) -> Vec<Chip> {
    let mut chips = Vec::new();
    for (joint, glyph) in design.joints().iter().zip(glyphs) {
        if !inside(design, joint.assembly, id) {
            continue;
        }
        for pair in &joint.pairs {
            let (count, counted) = match pair.members.first() {
                Some(body) => (
                    pair.members.iter().map(|b| b.copies()).sum(),
                    Counted::Pieces(body.name.clone()),
                ),
                None => (1, Counted::Joints),
            };
            add_chip(
                &mut chips,
                Chip {
                    glyph: *glyph,
                    kind: joint_words(joint),
                    count,
                    counted,
                },
            );
        }
    }
    for hardware in design
        .hardware()
        .iter()
        .filter(|h| inside(design, h.assembly, id))
    {
        add_chip(
            &mut chips,
            Chip {
                glyph: hardware_glyph(hardware),
                kind: kind_words(&hardware.kind),
                count: 1,
                counted: Counted::Fitted,
            },
        );
    }
    chips
}

/// How much the panel shows at its width.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Density {
    /// Everything: chips with counts, heading notes, joined-to words.
    Full,
    /// Below [`Density::COMPACT_BELOW`] pixels: chips without counts (the
    /// count is in the tooltip), no heading notes (the motion icon
    /// stays), joined-to rows keep their icon and partners.
    Compact,
    /// Below [`Density::NARROW_BELOW`] pixels: each row's chips fold into
    /// one badge (a joint glyph and how many kinds), and names ellipsize.
    Narrow,
}

impl Density {
    /// Narrower than this many pixels, the panel is compact.
    pub const COMPACT_BELOW: i32 = 220;
    /// Narrower than this many pixels, the panel is narrow.
    pub const NARROW_BELOW: i32 = 160;

    /// The density for a panel `width` pixels wide.
    pub fn for_width(width: i32) -> Density {
        if width < Density::NARROW_BELOW {
            Density::Narrow
        } else if width < Density::COMPACT_BELOW {
            Density::Compact
        } else {
            Density::Full
        }
    }

    /// Whether chips show their counts.
    pub fn chip_counts(self) -> bool {
        matches!(self, Density::Full)
    }

    /// Whether a row's chips fold into one badge.
    pub fn badge(self) -> bool {
        matches!(self, Density::Narrow)
    }

    /// Whether headings show their grey note (the motion icon shows at
    /// every width).
    pub fn heading_notes(self) -> bool {
        matches!(self, Density::Full)
    }

    /// Whether joined-to rows say the joint kind in words (they always
    /// keep the icon and the partners).
    pub fn joined_words(self) -> bool {
        matches!(self, Density::Full)
    }

    /// Whether a folded row says what it is ("mirror of door_l").
    pub fn folded_detail(self) -> bool {
        matches!(self, Density::Full)
    }

    /// The most characters a partner's name keeps before it is cut short
    /// (it keeps them ahead of the joint's words, which are cut first).
    pub fn partner_chars(self) -> usize {
        match self {
            Density::Full => 22,
            Density::Compact => 16,
            Density::Narrow => 10,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::OnceLock;

    use super::*;

    fn cabinet() -> &'static SolvedDesign {
        static DESIGN: OnceLock<SolvedDesign> = OnceLock::new();
        DESIGN.get_or_init(|| {
            let folder =
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/clothing_cabinet");
            let opened = crate::open(&folder).expect("the cabinet opens");
            opened.design.expect("the cabinet solves")
        })
    }

    fn part(name: &str) -> PartId {
        cabinet().part_named(name).expect("the part exists").id
    }

    fn summary(chips: &[Chip]) -> Vec<(Glyph, String, u32)> {
        chips
            .iter()
            .map(|c| (c.glyph, c.kind.clone(), c.count))
            .collect()
    }

    fn chip(glyph: Glyph, kind: &str, count: u32) -> (Glyph, String, u32) {
        (glyph, kind.to_owned(), count)
    }

    #[test]
    fn a_side_counts_pins_and_joints() {
        let joints = PartJoints::new(cabinet());
        let chips = joints.chips(&[Item::Part(part("side_l"))]);
        assert_eq!(
            summary(&chips),
            vec![
                chip(Glyph::Channel, "dado", 2),
                chip(Glyph::Channel, "rabbet", 1),
                chip(Glyph::Pins, "dowel", 8),
                chip(Glyph::Hole, "socket", 1),
            ]
        );
        assert_eq!(
            chips.iter().map(Chip::tooltip).collect::<Vec<_>>(),
            [
                "dado: 2 joints",
                "rabbet: 1 joint",
                "dowel: 8 pins",
                "socket: 1 joint"
            ]
        );
        assert_eq!(
            chips.iter().map(Chip::shows_count).collect::<Vec<_>>(),
            [true, false, true, false]
        );
        let base = joints.chips(&[Item::Part(part("base"))]);
        assert_eq!(
            summary(&base),
            vec![
                chip(Glyph::Pins, "dowel", 8),
                chip(Glyph::Tenon, "round tenon", 4)
            ]
        );
    }

    #[test]
    fn a_door_stile_has_its_frame_its_panel_and_its_hinges() {
        let joints = PartJoints::new(cabinet());
        let stile = part("door_l.stile[hinge]");
        assert_eq!(
            summary(&joints.chips(&[Item::Part(stile)])),
            vec![
                chip(Glyph::Frame, "cope and stick", 2),
                chip(Glyph::Channel, "groove, no glue", 1),
                chip(Glyph::Hinge, "euro hinge", 2),
            ]
        );
        let rows = joints.joined_to(stile);
        let said: Vec<(Glyph, &str, Vec<&str>)> = rows
            .iter()
            .map(|row| {
                (
                    row.glyph,
                    row.words.as_str(),
                    row.partners.iter().map(|p| p.label.as_str()).collect(),
                )
            })
            .collect();
        assert_eq!(
            said,
            vec![
                (Glyph::Frame, "cope and stick", vec!["rail[bottom, top]"]),
                (Glyph::Channel, "groove, no glue", vec!["panel"]),
                (
                    Glyph::Hinge,
                    "euro hinge ×2, back face",
                    vec!["hinges[lower, upper]"]
                ),
                (Glyph::Turn, "turns on", vec!["side_l"]),
            ]
        );
        // Partners are things to select, and light every one they name.
        let rails = rows.first().and_then(|r| r.partners.first());
        assert_eq!(
            rails.and_then(|p| p.selection.clone()),
            Some(Selection::Part(part("door_l.rail[bottom]")))
        );
        assert_eq!(rails.map(|p| p.items.len()), Some(2));
        let turns = rows.last().and_then(|r| r.partners.first());
        assert_eq!(
            turns.and_then(|p| p.selection.clone()),
            Some(Selection::Part(part("side_l")))
        );
        // A frame joint opens focus mode on its first pair.
        assert!(matches!(
            rows.first().and_then(|r| r.focus),
            Some(Target::Joint {
                pair: stave_model::solved::PairIndex::FIRST,
                ..
            })
        ));
    }

    #[test]
    fn rows_for_several_parts_add_up() {
        let joints = PartJoints::new(cabinet());
        let rails = joints.chips(&[
            Item::Part(part("door_l.rail[bottom]")),
            Item::Part(part("door_l.rail[top]")),
        ]);
        assert_eq!(
            summary(&rails),
            vec![
                chip(Glyph::Frame, "cope and stick", 4),
                chip(Glyph::Channel, "groove, no glue", 2),
            ]
        );
    }

    #[test]
    fn a_side_lists_the_hinges_that_hang_on_it() {
        let joints = PartJoints::new(cabinet());
        let rows = joints.joined_to(part("side_l"));
        let hinges = rows
            .iter()
            .find(|r| r.glyph == Glyph::Hinge)
            .expect("side_l carries door_l's hinges");
        assert_eq!(hinges.words, "euro hinge ×2");
        assert_eq!(
            hinges
                .partners
                .iter()
                .map(|p| p.label.as_str())
                .collect::<Vec<_>>(),
            ["door_l.hinges[lower, upper]"]
        );
    }

    #[test]
    fn hardware_and_pins_belong_with_their_part() {
        let design = cabinet();
        let joints = PartJoints::new(design);
        let hinge = design
            .hardware()
            .iter()
            .find(|h| h.name == "door_l.hinges[upper]")
            .expect("the hinge exists");
        assert_eq!(
            joints.home(Item::Hardware(hinge.id)),
            Some(part("door_l.stile[hinge]"))
        );
        let (pin, _) = design.pair_members().next().expect("a dowel's pins");
        assert!(joints.home(Item::Member(pin)).is_some());
        // The hinge's row stands for it, so choosing it marks that row.
        let rows = joints.joined_to(part("door_l.stile[hinge]"));
        assert!(
            rows.iter()
                .any(|r| r.stands_for.contains(&Selection::Hardware(hinge.id)))
        );
    }

    #[test]
    fn headings_say_how_an_assembly_moves_or_what_it_holds() {
        let design = cabinet();
        let joints = PartJoints::new(design);
        let carcass = joints.heading(None).expect("the carcass heading");
        assert_eq!(carcass.title, "Carcass");
        assert_eq!(carcass.note, "12 parts · 6 joints");
        assert_eq!(carcass.motion, None);
        let door = design
            .assemblies()
            .iter()
            .find(|a| a.name == "door_l")
            .expect("door_l");
        let heading = joints.heading(Some(door.id)).expect("door_l's heading");
        assert_eq!(heading.title, "door_l");
        assert_eq!(heading.note, "110° on side_l");
        assert_eq!(heading.motion, Some(Motion::Turn));
        assert_eq!(
            heading.motion_words.as_deref(),
            Some("turns 110° on side_l")
        );
    }

    #[test]
    fn mirrors_and_sets_fold_into_rows() {
        let design = cabinet();
        let joints = PartJoints::new(design);
        let tree = inspect::parts_tree(design);
        let blocks = joints.blocks(&tree, &|id| !joints.folded_by_default(id));
        let said: Vec<String> = blocks
            .iter()
            .map(|block| match block {
                Block::Heading { assembly, entries } => format!(
                    "{} [{}]",
                    joints.heading(*assembly).map_or("", |h| h.title.as_str()),
                    entries
                        .iter()
                        .map(|e| e.label.as_str())
                        .collect::<Vec<_>>()
                        .join("; ")
                ),
                Block::Folded { label, detail, .. } => {
                    format!("folded {label} {}", detail.as_deref().unwrap_or("-"))
                }
            })
            .collect();
        assert_eq!(
            said,
            vec![
                "Carcass [base; side_l, side_r; top; back; divider; shelf; rod; foot[4]]",
                "door_l [stile[hinge, latch]; rail[bottom, top]; panel]",
                "folded drawer[bottom, top] -",
                "folded door_r mirror of door_l",
            ]
        );
        let Some(Block::Folded { assemblies, .. }) = blocks.get(2) else {
            panic!("the drawers fold");
        };
        let drawers: Vec<AssemblyId> = assemblies.iter().copied().collect();
        let chips = joints.folded_chips(&drawers);
        assert!(
            chips
                .iter()
                .any(|c| c.glyph == Glyph::Slide && c.count == 4)
        );
        assert!(
            chips
                .iter()
                .any(|c| c.kind == "screw, no glue" && c.count == 4)
        );
        let Some(Block::Folded { assemblies, .. }) = blocks.get(3) else {
            panic!("door_r folds");
        };
        assert_eq!(
            summary(&joints.folded_chips(&[*assemblies.first()])),
            vec![chip(Glyph::Mirror, "mirror of door_l", 1)]
        );
        // Opened, a drawer is a heading of its own and its twin stays a row.
        let top = design
            .assemblies()
            .iter()
            .find(|a| a.name == "drawer[top]")
            .expect("drawer[top]");
        let blocks = joints.blocks(&tree, &|id| id == top.id || !joints.folded_by_default(id));
        assert!(blocks.iter().any(|b| matches!(
            b,
            Block::Heading { assembly: Some(id), .. } if *id == top.id
        )));
        assert!(blocks.iter().any(|b| matches!(
            b,
            Block::Folded { label, .. } if label == "drawer[bottom]"
        )));
    }

    #[test]
    fn the_same_design_gives_the_same_chips() {
        assert_eq!(PartJoints::new(cabinet()), PartJoints::new(cabinet()));
    }

    #[test]
    fn narrow_panels_show_less() {
        assert_eq!(Density::for_width(300), Density::Full);
        assert_eq!(Density::for_width(220), Density::Full);
        assert_eq!(Density::for_width(219), Density::Compact);
        assert_eq!(Density::for_width(160), Density::Compact);
        assert_eq!(Density::for_width(159), Density::Narrow);
        assert_eq!(Density::for_width(0), Density::Narrow);
        assert!(Density::Full.chip_counts() && Density::Full.heading_notes());
        assert!(!Density::Compact.chip_counts() && !Density::Compact.heading_notes());
        assert!(!Density::Compact.badge() && Density::Narrow.badge());
        assert!(Density::Full.joined_words() && !Density::Compact.joined_words());
        assert!(Density::Full.partner_chars() > Density::Narrow.partner_chars());
    }

    #[test]
    fn a_badge_counts_kinds_and_lists_them() {
        let joints = PartJoints::new(cabinet());
        let chips = joints.chips(&[Item::Part(part("side_l"))]);
        assert_eq!(
            badge(&chips),
            Some((
                4,
                "dado: 2 joints\nrabbet: 1 joint\ndowel: 8 pins\nsocket: 1 joint".to_owned()
            ))
        );
        assert_eq!(badge(&[]), None);
    }
}
