//! Fit and geometry checks (layer 4) and scene poses (the static part of
//! layer 5) of a [`SolvedDesign`].
//!
//! ```text
//! SolvedDesign ──stave-geom::solid──▶ cut solids ──stave-geom::collide──▶ overlaps, contact
//!              ──stave-check──▶ diagnostics
//! ```
//!
//! What is checked, in order:
//!
//! 1. **Cuts the geometry can't make yet** (`warning[not-supported]`: a
//!    cut other than `drill` and `spigot` on a round or turned part): the
//!    part is drawn uncut there and checked as drawn. **Cuts whose shape
//!    is a mistake** (`error[type-mismatch]`: an `outline` that crosses
//!    itself, a dovetail at 90deg): the part is drawn uncut there and left
//!    out of the overlap check, which would only report the same mistake
//!    again.
//! 2. **Contact** (`error[no-contact]`): every part a joint lists must touch
//!    at least one of its counterparts over an area, after every cut; a
//!    part that touches only along an edge or at a corner (an offset half
//!    lap whose notches don't meet) is reported as such.
//! 3. **Spacing** (`error[overlap]`): an element spaced with `pitch` or
//!    `gap` that runs past its `to` anchor. Hardware whose kind's
//!    `require` or `adjust` failed where the solver placed it
//!    ([`Rules::Broken`], already reported) is left out of the overlap and
//!    motion checks, for every kind alike.
//! 4. **Overlap at rest** (`error[overlap]`): one body for every part's
//!    cut solid and one for every hardware member's shapes, each against
//!    every other (grammar, "One collision model"). A cup sits in its door
//!    only because its kind bores the hole; the one rule for every kind is
//!    that members of one piece of hardware are not checked against each
//!    other. A broad phase sorts bounding boxes along `across` and sweeps
//!    (a fixed order, so the result is the same on every run); the narrow
//!    phase measures the shared wood exactly ([`stave_geom::collide`]).
//!    Touching within the 0.05mm contact tolerance is fine.
//! 5. **Joints across motions** (`error[motion]`, [`motion`]): a joint's
//!    parts must move alike; a joint between a part an assembly's motion
//!    moves and a part it doesn't would hold the assembly shut.
//!    **Scene poses** (`error[motion]`): each scene's poses applied, every
//!    moved body against everything that moved differently (a link its
//!    hinge's mechanism turns moves differently from its door).
//! 6. **Sweeps** (`error[motion]`, [`motion`]): every motion through its
//!    whole travel, every body it moves against every body that moves
//!    differently.
//! 7. **Models** (`warning[model-mismatch]`): each hardware kind's `.glb`,
//!    read from disk, against its text: every node the text declares, by
//!    name, to 0.01mm, and the file against its `sha256:` pin. A model
//!    that can't be read is not checked; no other check reads a model.
//!
//! **One mistake, one error.** The checker runs only on a design that
//! solved without errors. A part reported for spacing or for a misshapen
//! cut, and a slide reported for its gap, are left out of the overlap
//! checks, and an item with an error at rest is left out of the
//! scene checks. When one line causes several problems (a component used
//! twice, a joint applied to four pairs), they are reported once, at that
//! line, with the others as notes.
//!
//! **Two tiers.** An editor that checks a candidate on every pointer move
//! runs [`check_fast`]: the layer 1 to 3 diagnostics this crate makes
//! (the cuts of step 1 and `error[mechanism]`, which runs each mechanism
//! through its range) and box-level overlap at rest. Once the pointer
//! rests it runs [`check_slow`], everything else above, which finishes
//! the fast tier's list (reusing its mechanisms and taking its
//! diagnostics where the single check makes them) and which a flag
//! cancels between checks and between motions. On
//! `examples/clothing_cabinet` (release build) the fast tier takes about
//! 4 ms (the box overlaps about 21 µs of it, the mechanisms the rest), the
//! slow tier about 19 ms, the single check about 23 ms. Run to completion,
//! [`check_slow`] gives exactly what [`check_with_contacts`] gives, in the
//! same order, and [`Fast::diagnostics`] are its layer 1 to 3 diagnostics
//! (`tests/tiers.rs`). See the `tiers` module for what the fast tier's box
//! overlaps mean.

#![forbid(unsafe_code)]
#![warn(clippy::redundant_clone, clippy::needless_pass_by_value)]
#![warn(missing_docs)]

mod explain;
mod fixes;
mod held;
mod instance;
mod models;
mod motion;
mod names;
mod sequence;
mod tiers;
mod walls;
mod whole;

pub use tiers::{BoxOverlap, Cancelled, Fast, check_fast, check_slow};

use std::collections::{BTreeMap, BTreeSet};

use stave_geom::collide::{
    self, CONTACT_TOLERANCE, Overlap, Placement, WorldSolid, boxes_overlap, placed_bounds,
};
use stave_geom::planar::MechanismError;
use stave_geom::solid::Solid;
use stave_geom::sweep::Path;
use stave_lang::ast::Span;
use stave_lang::{Code, Diagnostic, FileId, Label, Severity};
use stave_model::Model;
use stave_model::solved::{
    AnchorItem, AssemblyId, BodyCarrier, BodyOwner, BodyRef, CutSource, FixedSpacing, HardwareId,
    MemberIndex, PairMember, PartId, PartOrigin, PoseValue, Rules, SolvedDesign, WorldBox,
};

use crate::names::{length_text, pose_text};

/// Something a diagnostic is about, for a viewer to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ContactItem {
    /// A part.
    Part(PartId),
    /// A piece of hardware (either member).
    Hardware(HardwareId),
    /// A joint pair's member (a dowel's pins); the pair's parts are
    /// listed with it.
    Member(PairMember),
}

/// Where a motion hits something.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContactMotion {
    /// The assembly that moves.
    pub assembly: AssemblyId,
    /// Where it first touches: an angle or a distance.
    pub at: PoseValue,
    /// A little further (2deg or 2mm, within its travel), where the
    /// shared wood in [`Contact::regions`] is measured.
    pub shown: PoseValue,
}

/// What a fit or motion diagnostic is about, in the design: the items
/// involved, the shared wood, and the pose in which they collide.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Contact {
    /// The parts and hardware involved.
    pub items: Vec<ContactItem>,
    /// The shared wood, as world boxes, in the pose below.
    pub regions: Vec<WorldBox>,
    /// The scene whose poses apply; `None` for rest.
    pub scene: Option<stave_model::solved::SceneId>,
    /// The motion that collides, and where.
    pub motion: Option<ContactMotion>,
}

impl Contact {
    /// Adds what an item is about: a part, a piece of hardware, or for a
    /// joint's member the parts of its pair.
    fn add_item(&mut self, design: &SolvedDesign, item: Item) {
        let items: Vec<ContactItem> = match item {
            Item::Part(part) => vec![ContactItem::Part(part)],
            Item::Member(member) => match member.owner {
                BodyOwner::Hardware { hardware } => vec![ContactItem::Hardware(hardware)],
                BodyOwner::Joint { joint, pair } => {
                    let parts = design
                        .joints()
                        .get(joint.index())
                        .and_then(|joint| joint.pair(pair))
                        .map(|pair| pair.roles.iter().map(|role| ContactItem::Part(role.part)));
                    member
                        .pair_member()
                        .map(ContactItem::Member)
                        .into_iter()
                        .chain(parts.into_iter().flatten())
                        .collect()
                }
            },
        };
        for item in items {
            if !self.items.contains(&item) {
                self.items.push(item);
            }
        }
    }

    /// A contact about some items and the wood they share.
    fn of(design: &SolvedDesign, items: &[Item], found: &[Overlap]) -> Contact {
        let mut contact = Contact::default();
        for item in items {
            contact.add_item(design, *item);
        }
        contact.regions = found.iter().map(|o| o.region).collect();
        contact
    }
}

/// Checks a solved design. Diagnostics come back in a fixed order; the
/// application sorts them by position with everything else.
pub fn check(model: &Model, design: &SolvedDesign) -> Vec<Diagnostic> {
    check_with_contacts(model, design)
        .into_iter()
        .map(|(diagnostic, _)| diagnostic)
        .collect()
}

/// [`check`], with what each diagnostic is about in the design.
pub fn check_with_contacts(
    model: &Model,
    design: &SolvedDesign,
) -> Vec<(Diagnostic, Option<Contact>)> {
    match run(model, design, None, &tiers::Never) {
        Ok(found) => found,
        Err(never) => match never {},
    }
}

/// Every check in order, reading `stop` between checks and inside the
/// sweep check: the single check never stops, the slow tier
/// ([`check_slow`]) stops when cancelled.
///
/// `fast` is what the fast tier found for this model and design, which
/// the checks finish: its mechanisms are not run through their range
/// again, and each layer 1 to 3 diagnostic is taken from it, in turn,
/// where the checks would make it ([`Checker::report`]). `None` makes
/// everything here.
fn run<'m, S: tiers::Stop + ?Sized>(
    model: &'m Model,
    design: &'m SolvedDesign,
    fast: Option<&tiers::Fast>,
    stop: &S,
) -> Result<Vec<(Diagnostic, Option<Contact>)>, S::Stopped> {
    stop.check()?;
    let mut checker = Checker::new(model, design, fast.and_then(|fast| fast.mechanisms.clone()));
    checker.given = fast.map(|fast| fast.made.clone().into_iter());
    let steps: [fn(&mut Checker<'m>); 12] = [
        Checker::misfits,
        Checker::misshapen,
        Checker::not_supported,
        Checker::cut_sizes,
        Checker::mechanisms,
        Checker::contact,
        Checker::overruns,
        Checker::rest,
        Checker::walls,
        Checker::edge_contacts,
        Checker::joint_motions,
        Checker::scenes,
    ];
    for step in steps {
        stop.check()?;
        step(&mut checker);
    }
    stop.check()?;
    checker.sweeps(&|| stop.stopped());
    // The sweeps may have stopped part way: then this is the end.
    stop.check()?;
    checker.sequences();
    // Held is a question about the whole project, asked once nothing
    // else is wrong: a part with an error may be the one that would join
    // the rest.
    if !checker.diagnostics.iter().any(|(d, _)| d.is_error()) {
        stop.check()?;
        checker.held();
    }
    stop.check()?;
    // Models are read from disk; one that can't be read is not checked.
    for diagnostic in models::models(design, &|path| std::fs::read(path)) {
        checker.diagnostics.push((diagnostic, None));
    }
    Ok(checker.finish())
}

/// The fast tier's checks: the layer 1 to 3 diagnostics of the steps
/// that make them, in the single check's order, and the mechanisms run
/// through their range.
fn fast_steps(
    model: &Model,
    design: &SolvedDesign,
) -> (Vec<(Diagnostic, Option<Contact>)>, Mechanisms) {
    let mut checker = Checker::new(model, design, None);
    // The steps that report layers 1 to 3, in the order `run` runs them;
    // none of them reads what an earlier step found.
    checker.misshapen();
    checker.not_supported();
    checker.mechanisms();
    let made = std::mem::take(&mut checker.diagnostics)
        .into_iter()
        .filter(|(diagnostic, _)| diagnostic.code.layer() <= 3)
        .collect();
    let mechanisms = Mechanisms {
        paths: std::mem::take(&mut checker.paths),
        broken: std::mem::take(&mut checker.broken),
    };
    (made, mechanisms)
}

/// A design's mechanisms run through their range, by assembly, and the
/// ones that can't be.
#[derive(Debug, Clone, PartialEq)]
struct Mechanisms {
    paths: Vec<Option<Path>>,
    broken: Vec<(AssemblyId, MechanismError)>,
}

/// Something that takes up room: a part, or a member of a piece of
/// hardware or of a joint pair (grammar, "One collision model").
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Item {
    /// A member of a piece of hardware or of a joint pair: all its shapes
    /// (for a joint's member, every copy of it).
    Member(BodyRef),
    /// A part.
    Part(PartId),
}

impl Item {
    /// The piece of hardware a member belongs to.
    fn hardware(self) -> Option<HardwareId> {
        match self {
            Item::Member(member) => member.hardware(),
            Item::Part(_) => None,
        }
    }

    /// What a member belongs to.
    fn owner(self) -> Option<BodyOwner> {
        match self {
            Item::Member(member) => Some(member.owner),
            Item::Part(_) => None,
        }
    }
}

/// How a joint's part meets its counterparts once every cut is made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Meeting {
    /// It touches none of them.
    Apart,
    /// It touches this one, but only along an edge or at a corner.
    Edge(PartId),
    /// It touches one over an area.
    Area,
}

/// An item's solid and where it moves.
struct Body {
    item: Item,
    solid: WorldSolid,
    bounds: WorldBox,
    /// The assemblies it moves with, outermost first.
    chain: Vec<AssemblyId>,
    /// For a member its assembly's mechanism places (a link): that
    /// assembly, and which member of its mechanism.
    link: Option<(AssemblyId, MemberIndex)>,
}

struct Checker<'a> {
    model: &'a Model,
    design: &'a SolvedDesign,
    solids: Vec<Solid>,
    bodies: Vec<Body>,
    /// Each assembly's mechanism, run through its range, by assembly;
    /// `None` for one nothing moves (or whose mechanism can't move,
    /// reported by [`Checker::mechanisms`]).
    paths: Vec<Option<Path>>,
    /// The mechanisms that can't be run through their range.
    broken: Vec<(AssemblyId, MechanismError)>,
    /// Parts already reported, for their spacing or for a cut whose shape
    /// is a mistake: an overlap would report the same mistake again.
    reported: BTreeSet<PartId>,
    /// Items with an error at rest.
    failed: BTreeSet<Item>,
    /// Items with an error at a scene's poses.
    posed_failed: BTreeSet<Item>,
    /// Hardware whose kind's rules failed where it is placed (reported by
    /// the solver): its members are left out of the overlap and motion
    /// checks, which would only report the same mistake again.
    misfit: BTreeSet<HardwareId>,
    /// The joint pairs and hardware whose cuts already have an error for
    /// the wood round them (a kind's wall rule, a cut too big or too
    /// deep): the backstops leave their cuts alone.
    wall_failed: Vec<CutSource>,
    /// Cuts too big or too deep that a wall rule of their own kind
    /// measures: reported after the wall rules, only if those didn't.
    held_back: Vec<walls::SizeProblem>,
    /// Joint parts that, once cut, touch a counterpart only along an
    /// edge: reported after the wall rules and cut sizes, only if those
    /// found nothing wrong with the joint's cuts (a cut too deep is the
    /// mistake, and the edge only what it leaves).
    edge_only: Vec<(stave_model::solved::JointId, PartId, PartId)>,
    /// The layer 1 to 3 diagnostics the fast tier already made, in
    /// order: [`Checker::report`] takes them instead of its own.
    given: Option<std::vec::IntoIter<(Diagnostic, Option<Contact>)>>,
    diagnostics: Vec<(Diagnostic, Option<Contact>)>,
    /// What the next reported diagnostic is about.
    next_contact: Option<Contact>,
}

impl<'a> Checker<'a> {
    /// `mechanisms`: the design's mechanisms already run through their
    /// range, when the fast tier has done it.
    fn new(model: &'a Model, design: &'a SolvedDesign, mechanisms: Option<Mechanisms>) -> Self {
        let solids: Vec<Solid> = design
            .parts()
            .iter()
            .map(|part| Solid::of(part, design))
            .collect();
        let chain_of = |mut assembly: Option<AssemblyId>| {
            let mut chain = Vec::new();
            while let Some(id) = assembly {
                if chain.contains(&id) {
                    break;
                }
                chain.push(id);
                assembly = design.assembly(id).and_then(|a| a.parent);
            }
            chain.reverse();
            chain
        };
        let mut bodies = Vec::new();
        for (part, solid) in design.parts().iter().zip(&solids) {
            let world = WorldSolid::of_part(part, solid);
            if let Some(bounds) = world.bounds() {
                bodies.push(Body {
                    item: Item::Part(part.id),
                    solid: world,
                    bounds,
                    chain: chain_of(part.assembly),
                    link: None,
                });
            }
        }
        for hardware in design.hardware() {
            for member in &hardware.members {
                let (assembly, link) = match member.carrier {
                    BodyCarrier::Part(part) => (design.part(part).and_then(|p| p.assembly), None),
                    BodyCarrier::Region(region) => {
                        (design.region(region).and_then(|r| r.assembly), None)
                    }
                    BodyCarrier::Link(link) => (
                        hardware.assembly,
                        hardware.assembly.map(|assembly| (assembly, link)),
                    ),
                };
                let world = WorldSolid::of_body(member);
                if let Some(bounds) = world.bounds() {
                    bodies.push(Body {
                        item: Item::Member(member.id),
                        solid: world,
                        bounds,
                        chain: chain_of(assembly),
                        link,
                    });
                }
            }
        }
        // Every joint's members (a dowel's pins), where its kind laid them
        // out: they move with the pair's first part.
        for joint in design.joints() {
            for pair in &joint.pairs {
                for member in &pair.members {
                    let assembly = match member.carrier {
                        BodyCarrier::Part(part) => design.part(part).and_then(|p| p.assembly),
                        BodyCarrier::Region(_) | BodyCarrier::Link(_) => joint.assembly,
                    };
                    let world = WorldSolid::of_body(member);
                    if let Some(bounds) = world.bounds() {
                        bodies.push(Body {
                            item: Item::Member(member.id),
                            solid: world,
                            bounds,
                            chain: chain_of(assembly),
                            link: None,
                        });
                    }
                }
            }
        }
        let Mechanisms { paths, broken } =
            mechanisms.unwrap_or_else(|| motion::mechanism_paths(design));
        Checker {
            model,
            design,
            solids,
            bodies,
            paths,
            broken,
            reported: BTreeSet::new(),
            failed: BTreeSet::new(),
            posed_failed: BTreeSet::new(),
            misfit: design
                .hardware()
                .iter()
                .filter(|hardware| match hardware.rules {
                    Rules::Hold => false,
                    Rules::Broken => true,
                })
                .map(|hardware| hardware.id)
                .collect(),
            wall_failed: Vec::new(),
            held_back: Vec::new(),
            edge_only: Vec::new(),
            given: None,
            diagnostics: Vec::new(),
            next_contact: None,
        }
    }

    // -----------------------------------------------------------------------
    // Where things are written
    // -----------------------------------------------------------------------

    /// The kind call of a joint, or for a splice the `lengths:` of the part
    /// it splices.
    fn joint_place(&self, joint: stave_model::solved::JointId) -> Option<(FileId, Span)> {
        if let Some(stave_model::solved::JointOrigin::Splice { part, .. }) =
            self.design.joint(joint).map(|j| j.origin)
        {
            return self.lengths_place(part);
        }
        let inst = self.model.joints.get(joint.index())?;
        let def = self.model.defs.joint(inst.def)?;
        Some((def.decl.file, def.call.span))
    }

    /// Where a spliced part's `lengths:` is written: its own, or its
    /// mirror source's.
    fn lengths_place(&self, part: PartId) -> Option<(FileId, Span)> {
        let mut current = part;
        while let Some(PartOrigin::Mirror { source } | PartOrigin::MirroredAssembly { source }) =
            self.design.part(current).map(|p| p.origin)
        {
            current = source;
        }
        let inst = self.model.parts.get(current.index())?;
        let def = self.model.defs.part(inst.def)?;
        match &def.kind {
            stave_model::defs::PartKind::FromStock(body) => {
                Some((def.decl.file, body.lengths.as_ref()?.span))
            }
            stave_model::defs::PartKind::Mirror(_) => None,
        }
    }

    /// The kind call of a piece of hardware.
    fn hardware_place(&self, hardware: HardwareId) -> Option<(FileId, Span)> {
        let inst = self.model.hardware.get(hardware.index())?;
        let def = self.model.defs.hardware_def(inst.def)?;
        Some((def.decl.file, def.call.span))
    }

    /// A part's name where it is declared (a made part: its joint).
    fn part_place(&self, part: PartId) -> Option<(FileId, Span)> {
        match self.design.part(part).map(|p| p.origin) {
            Some(PartOrigin::Made { joint, .. }) => return self.joint_place(joint),
            Some(PartOrigin::Piece { part: first, .. }) => return self.part_place(first),
            _ => {}
        }
        let inst = self.model.parts.get(part.index())?;
        let def = self.model.defs.part(inst.def)?;
        Some((def.decl.file, def.decl.name_span))
    }

    /// The project item, for anything that has no better place.
    fn project_place(&self) -> Option<(FileId, Span)> {
        let project = self.model.defs.project(self.model.root)?;
        Some((project.file, project.span))
    }

    fn report(
        &mut self,
        severity: Severity,
        code: Code,
        message: String,
        place: Option<(FileId, Span)>,
        notes: Vec<String>,
        help: Option<String>,
    ) {
        let Some((file, span)) = place.or_else(|| self.project_place()) else {
            return;
        };
        let mut diagnostic = Diagnostic::error(code, message, Label::new(file, span));
        diagnostic.severity = severity;
        diagnostic.notes = notes;
        diagnostic.help = help;
        let contact = self.next_contact.take();
        // The fast tier's layers come from the fast tier when it ran.
        let given = match &mut self.given {
            Some(given) if code.layer() <= 3 => given.next(),
            _ => None,
        };
        self.diagnostics
            .push(given.unwrap_or((diagnostic, contact)));
    }

    /// A member's body, wherever it is.
    pub(crate) fn member(&self, member: BodyRef) -> Option<&stave_model::solved::Body> {
        let members = match member.owner {
            BodyOwner::Hardware { hardware } => &self.design.hardware_piece(hardware)?.members,
            BodyOwner::Joint { joint, pair } => &self.design.joint(joint)?.pair(pair)?.members,
        };
        members.iter().find(|m| m.id == member)
    }

    /// A part's name, or a member's as `<hardware>'s <member>` or
    /// `<joint>'s <member>` (with the pair's parts, when the joint has
    /// more than one pair: `top_dowels's pin between side_l and top`).
    fn item_name(&self, item: Item) -> String {
        match item {
            Item::Part(part) => self.part_name(part),
            Item::Member(member) => {
                let name = self
                    .member(member)
                    .map(|m| m.name.as_str())
                    .unwrap_or_default();
                match member.owner {
                    BodyOwner::Hardware { hardware } => {
                        format!("{}'s {name}", self.hardware_name(hardware))
                    }
                    BodyOwner::Joint { joint, pair } => {
                        let found = self.design.joint(joint);
                        let between = found
                            .filter(|joint| joint.pairs.len() > 1)
                            .and_then(|joint| joint.pair(pair))
                            .map(|pair| {
                                let parts: Vec<String> =
                                    pair.roles.iter().map(|r| self.part_name(r.part)).collect();
                                format!(" between {}", names::and_list(&parts))
                            })
                            .unwrap_or_default();
                        format!("{}'s {name}{between}", self.joint_name(joint))
                    }
                }
            }
        }
    }

    pub(crate) fn hardware_name(&self, hardware: HardwareId) -> String {
        self.design
            .hardware()
            .get(hardware.index())
            .map(|h| h.name.clone())
            .unwrap_or_default()
    }

    /// The members of hardware whose kind's rules failed have an error
    /// already: later layers leave them out.
    fn misfits(&mut self) {
        let members: Vec<Item> = self
            .misfit
            .iter()
            .flat_map(|hardware| self.members_of(*hardware))
            .collect();
        self.failed.extend(members);
    }

    /// Every member of a piece of hardware, as items.
    fn members_of(&self, hardware: HardwareId) -> Vec<Item> {
        self.design
            .hardware()
            .get(hardware.index())
            .map(|h| h.members.iter().map(|m| Item::Member(m.id)).collect())
            .unwrap_or_default()
    }

    fn part_name(&self, part: PartId) -> String {
        self.design
            .part(part)
            .map(|p| p.name.clone())
            .unwrap_or_default()
    }

    pub(crate) fn joint_name(&self, joint: stave_model::solved::JointId) -> String {
        self.design
            .joints()
            .get(joint.index())
            .map(|j| j.name.clone())
            .unwrap_or_default()
    }

    // -----------------------------------------------------------------------
    // 1. Cuts that can't be made yet, and cuts whose shape is a mistake
    // -----------------------------------------------------------------------

    fn misshapen(&mut self) {
        let mut found = Vec::new();
        for (part, solid) in self.design.parts().iter().zip(&self.solids) {
            for misshapen in solid.misshapen() {
                if let Some(cut) = stave_geom::solid::cut_at(part, misshapen.cut) {
                    found.push((part.id, cut.source, misshapen.error));
                }
            }
        }
        let mut seen: BTreeSet<(SourceKey, String)> = BTreeSet::new();
        for (part, source, error) in found {
            self.reported.insert(part);
            self.failed.insert(Item::Part(part));
            let key = SourceKey::of(source);
            let message = error.to_string();
            if !seen.insert((key, message.clone())) {
                continue;
            }
            let (place, owner) = match key {
                SourceKey::Joint(joint) => (self.joint_place(joint), self.joint_name(joint)),
                SourceKey::Hardware(hardware) => {
                    (self.hardware_place(hardware), self.hardware_name(hardware))
                }
                SourceKey::Boards(part) => (
                    self.part_place(part),
                    format!("{}'s boards", self.part_name(part)),
                ),
            };
            let note = format!(
                "{owner}: {} is drawn without this cut until it is fixed",
                self.part_name(part)
            );
            self.next_contact = Some(Contact::of(self.design, &[Item::Part(part)], &[]));
            self.report(
                Severity::Error,
                Code::TypeMismatch,
                format!("{owner}: {message}"),
                place,
                vec![note],
                Some(error.help()),
            );
        }
    }

    fn not_supported(&mut self) {
        // Per source: what can't be cut, and the parts drawn uncut.
        let mut found: BTreeMap<SourceKey, (Vec<String>, Vec<PartId>)> = BTreeMap::new();
        for (part, solid) in self.design.parts().iter().zip(&self.solids) {
            for skipped in solid.skipped() {
                let Some(cut) = part.cuts.get(skipped.cut) else {
                    continue;
                };
                let (what, parts) = found.entry(SourceKey::of(cut.source)).or_default();
                let description = skipped.reason.description();
                if !what.contains(&description) {
                    what.push(description);
                }
                if !parts.contains(&part.id) {
                    parts.push(part.id);
                }
            }
        }
        for (key, (what, parts)) in found {
            let names: Vec<String> = parts.iter().map(|p| self.part_name(*p)).collect();
            let (place, owner) = match key {
                SourceKey::Joint(joint) => (self.joint_place(joint), self.joint_name(joint)),
                SourceKey::Hardware(hardware) => {
                    (self.hardware_place(hardware), self.hardware_name(hardware))
                }
                SourceKey::Boards(part) => (
                    self.part_place(part),
                    format!("{}'s boards", self.part_name(part)),
                ),
            };
            let note = if names.len() == 1 {
                format!(
                    "{owner}: {} is drawn uncut there, and its fit is checked as drawn",
                    names::list(&names)
                )
            } else {
                format!(
                    "{owner}: {} are drawn uncut there, and their fit is checked as drawn",
                    names::list(&names)
                )
            };
            let items: Vec<Item> = parts.iter().map(|p| Item::Part(*p)).collect();
            self.next_contact = Some(Contact::of(self.design, &items, &[]));
            self.report(
                Severity::Warning,
                Code::NotSupported,
                format!("cutting {} is not supported yet", names::list(&what)),
                place,
                vec![note],
                Some(
                    "check this joint's fit by hand on a test piece before cutting the real parts"
                        .to_owned(),
                ),
            );
        }
    }

    // -----------------------------------------------------------------------
    // 2. Contact
    // -----------------------------------------------------------------------

    fn world_part(&self, part: PartId) -> Option<WorldSolid> {
        let solid = self.solids.get(part.index())?;
        Some(WorldSolid::of_part(self.design.part(part)?, solid))
    }

    fn contact(&mut self) {
        let mut problems = Vec::new();
        for joint in self.design.joints() {
            // Every listed part, in role order, and how it meets its
            // counterparts once cut.
            let mut touched: Vec<(PartId, Meeting)> = Vec::new();
            for pair in &joint.pairs {
                let parts: Vec<PartId> = pair.roles.iter().map(|r| r.part).collect();
                for (index, part) in parts.iter().enumerate() {
                    let position = match touched.iter().position(|(p, _)| p == part) {
                        Some(position) => position,
                        None => {
                            touched.push((*part, Meeting::Apart));
                            touched.len() - 1
                        }
                    };
                    if touched
                        .get(position)
                        .is_some_and(|(_, meeting)| *meeting == Meeting::Area)
                    {
                        continue;
                    }
                    let Some(own) = self.world_part(*part) else {
                        continue;
                    };
                    for (other_index, other) in parts.iter().enumerate() {
                        if other_index == index || other == part {
                            continue;
                        }
                        let Some(solid) = self.world_part(*other) else {
                            continue;
                        };
                        let meeting = if collide::touches_over_area(&own, &solid) {
                            Meeting::Area
                        } else if collide::touches(&own, &solid) {
                            Meeting::Edge(*other)
                        } else {
                            continue;
                        };
                        if let Some(entry) = touched.get_mut(position) {
                            // An area beats an edge; the first edge found
                            // is the one named.
                            match (entry.1, meeting) {
                                (Meeting::Area, _) | (Meeting::Edge(_), Meeting::Edge(_)) => {}
                                (Meeting::Apart | Meeting::Edge(_), _) => entry.1 = meeting,
                            }
                        }
                        if meeting == Meeting::Area {
                            break;
                        }
                    }
                }
            }
            let mut apart: Vec<(PartId, Meeting)> = joint
                .unmatched
                .iter()
                .map(|part| (*part, Meeting::Apart))
                .collect();
            for (part, meeting) in touched {
                if meeting != Meeting::Area && !apart.iter().any(|(p, _)| *p == part) {
                    apart.push((part, meeting));
                }
            }
            for (part, meeting) in apart {
                // Two parts that touch only along an edge are one mistake.
                let mirrored = |(j, p, m): &(stave_model::solved::JointId, PartId, Meeting)| {
                    *j == joint.id
                        && matches!(meeting, Meeting::Edge(other) if other == *p)
                        && *m == Meeting::Edge(part)
                };
                if !problems.iter().any(mirrored) {
                    problems.push((joint.id, part, meeting));
                }
            }
        }
        for (joint, part, meeting) in problems {
            if let Meeting::Edge(other) = meeting {
                self.edge_only.push((joint, part, other));
                continue;
            }
            let (joint_name, part_name) = (self.joint_name(joint), self.part_name(part));
            self.next_contact = Some(Contact::of(self.design, &[Item::Part(part)], &[]));
            self.report(
                Severity::Error,
                Code::NoContact,
                format!("{joint_name}: {part_name} meets none of the parts it should join"),
                self.joint_place(joint),
                Vec::new(),
                Some(format!(
                    "place {part_name} so it touches a part it joins in '{joint_name}', or take it out of the joint"
                )),
            );
        }
    }

    /// The joint parts that touch only along an edge once cut, unless a
    /// wall rule or a cut size already reported the joint's cuts.
    fn edge_contacts(&mut self) {
        for (joint, part, other) in std::mem::take(&mut self.edge_only) {
            let cuts_failed = self.wall_failed.iter().any(|source| {
                matches!(source, CutSource::Joint { joint: failed, .. } if *failed == joint)
            });
            if cuts_failed || self.reported.contains(&part) || self.reported.contains(&other) {
                continue;
            }
            let (joint_name, part_name, other_name) = (
                self.joint_name(joint),
                self.part_name(part),
                self.part_name(other),
            );
            self.next_contact = Some(Contact::of(
                self.design,
                &[Item::Part(part), Item::Part(other)],
                &[],
            ));
            self.report(
                Severity::Error,
                Code::NoContact,
                format!("{joint_name}: once cut, {part_name} and {other_name} touch only along an edge"),
                self.joint_place(joint),
                Vec::new(),
                Some(format!(
                    "place {part_name} and {other_name} so their cuts meet face to face, or make the cuts no deeper than the parts cross"
                )),
            );
        }
    }

    // -----------------------------------------------------------------------
    // 3. Spacing past `to`
    // -----------------------------------------------------------------------

    fn overruns(&mut self) {
        for overrun in &self.design.overruns {
            self.reported.insert(overrun.part);
            let part = self.part_name(overrun.part);
            let to = match overrun.to {
                AnchorItem::Part(id) => self.part_name(id),
                AnchorItem::Region(id) => self
                    .design
                    .regions()
                    .get(id.index())
                    .map(|r| r.name.clone())
                    .unwrap_or_default(),
            };
            let anchor = match overrun.face {
                Some(face) => format!("{to}.{}", face.word()),
                None => to.clone(),
            };
            let (word, value) = match overrun.spacing {
                FixedSpacing::Pitch(value) => ("pitch", value),
                FixedSpacing::Gap(value) => ("gap", value),
            };
            let help = match overrun.largest {
                Some(largest) => format!(
                    "use {word}({}) or less, or fewer elements",
                    length_text(largest)
                ),
                None => {
                    "use fewer elements, or make the space they are spaced in larger".to_owned()
                }
            };
            let contact = Contact::of(self.design, &[Item::Part(overrun.part)], &[]);
            self.diagnostics.push((
                Diagnostic {
                    severity: Severity::Error,
                    code: Code::Overlap,
                    message: format!("{part} and {to} overlap"),
                    labels: Vec::new(),
                    notes: vec![format!(
                        "with {word}({}), {part} ends {} past {anchor}",
                        length_text(value),
                        length_text(overrun.past)
                    )],
                    merged: Vec::new(),
                    help: Some(help),
                    fixes: Vec::new(),
                    cause: None,
                },
                Some(contact),
            ));
            if let Some((file, span)) = self
                .part_place(overrun.part)
                .or_else(|| self.project_place())
                && let Some((last, _)) = self.diagnostics.last_mut()
            {
                last.labels.push(Label::new(file, span));
            }
        }
        // A diagnostic without a place is dropped.
        self.diagnostics.retain(|(d, _)| !d.labels.is_empty());
    }

    // -----------------------------------------------------------------------
    // 4. Overlap at rest
    // -----------------------------------------------------------------------

    /// Items that may share room: two members of one piece of hardware (a
    /// kind's own members are not checked against each other, for every
    /// kind alike), the members of hardware whose kind's rules failed,
    /// and parts already reported (for their spacing, or for a cut whose
    /// shape is a mistake). One mistake, one error.
    fn exempt(&self, a: Item, b: Item) -> bool {
        if [a, b]
            .into_iter()
            .filter_map(Item::hardware)
            .any(|h| self.misfit.contains(&h))
        {
            return true;
        }
        if let (Some(x), Some(y)) = (a.owner(), b.owner()) {
            return x == y;
        }
        if let (Item::Part(x), Item::Part(y)) = (a, b) {
            return self.reported.contains(&x) || self.reported.contains(&y);
        }
        false
    }

    fn rest(&mut self) {
        let boxes: Vec<WorldBox> = self.bodies.iter().map(|b| b.bounds).collect();
        let mut clashes = Vec::new();
        for (i, j) in sweep(&boxes) {
            let (Some(a), Some(b)) = (self.bodies.get(i), self.bodies.get(j)) else {
                continue;
            };
            if self.exempt(a.item, b.item) {
                continue;
            }
            let found = collide::overlaps(&a.solid, &b.solid);
            if !found.is_empty() {
                clashes.push((a.item, b.item, found));
            }
        }
        for (a, b, _) in &clashes {
            self.failed.insert(*a);
            self.failed.insert(*b);
        }
        let (groups, rest) = self.group(clashes);
        for group in groups {
            self.report_group(&group);
        }
        for (a, b, found) in rest {
            self.overlap(a, b, &found);
        }
    }

    /// The joint pair two parts are in together, if any.
    fn shared_joint(&self, a: PartId, b: PartId) -> Option<(stave_model::solved::JointId, usize)> {
        self.design.joints().iter().find_map(|joint| {
            joint.pairs.iter().enumerate().find_map(|(index, pair)| {
                let has = |part: PartId| {
                    pair.roles.iter().any(|r| r.part == part) || pair.made_parts.contains(&part)
                };
                (has(a) && has(b)).then_some((joint.id, index))
            })
        })
    }

    fn overlap(&mut self, a: Item, b: Item, found: &[Overlap]) {
        // Name them in a natural order: hardware first, then the joint's
        // role order, then declaration order.
        let (a, b) = match (a, b) {
            (Item::Part(x), Item::Part(y)) => match self.shared_joint(x, y) {
                Some((joint, pair)) => {
                    let position = |part: PartId| {
                        self.design
                            .joints()
                            .get(joint.index())
                            .and_then(|j| j.pairs.get(pair))
                            .and_then(|p| p.roles.iter().position(|r| r.part == part))
                            .unwrap_or(usize::MAX)
                    };
                    if position(y) < position(x) {
                        (b, a)
                    } else {
                        (a, b)
                    }
                }
                None => (a, b),
            },
            (Item::Part(_), _) => (b, a),
            _ => (a, b),
        };
        let place = match (a, b) {
            (Item::Part(x), Item::Part(y)) => match self.shared_joint(x, y) {
                Some((joint, _)) => self.joint_place(joint),
                None => self.part_place(x.max(y)),
            },
            (Item::Member(member), _) | (_, Item::Member(member)) => match member.owner {
                BodyOwner::Hardware { hardware } => self.hardware_place(hardware),
                BodyOwner::Joint { joint, .. } => self.joint_place(joint),
            },
        };
        let explanation = explain::overlap(self, a, b, found);
        self.next_contact = Some(Contact::of(self.design, &[a, b], found));
        self.report(
            Severity::Error,
            Code::Overlap,
            format!("{} and {} overlap", self.item_name(a), self.item_name(b)),
            place,
            explanation.notes,
            Some(explanation.help),
        );
    }

    // -----------------------------------------------------------------------
    // 5. Scene poses
    // -----------------------------------------------------------------------

    /// Per body, in one scene: the poses that move it (outermost first),
    /// with the link it is when its own assembly's linkage turns it, and
    /// its placement. Index 0 is the built-in `rest`.
    fn scene_moves(&self, scene_index: usize) -> (Vec<Moves>, Vec<Placement>) {
        let Some(scene) = self.design.scenes.get(scene_index) else {
            return (Vec::new(), Vec::new());
        };
        let pose_of = |assembly: AssemblyId| {
            scene
                .poses
                .iter()
                .find(|pose| pose.assembly == assembly)
                .map(|pose| pose.value)
                .filter(|value| !matches!(value, PoseValue::Closed))
        };
        let moves: Vec<Moves> = self
            .bodies
            .iter()
            .map(|body| {
                let steps: Vec<(AssemblyId, PoseValue)> = body
                    .chain
                    .iter()
                    .filter_map(|assembly| {
                        let value = pose_of(*assembly)?;
                        self.path(*assembly).map(|_| (*assembly, value))
                    })
                    .collect();
                // A link is turned by its own assembly's linkage only when
                // that assembly is the innermost one posed.
                let link = body
                    .link
                    .filter(|(assembly, _)| steps.last().map(|(a, _)| a) == Some(assembly))
                    .map(|(_, link)| link);
                Moves { steps, link }
            })
            .collect();
        let placements = moves
            .iter()
            .map(|moves: &Moves| {
                let chain: Vec<_> = moves
                    .steps
                    .iter()
                    .filter_map(|(assembly, value)| {
                        Some((self.path(*assembly)?.mechanism(), *value))
                    })
                    .collect();
                Placement::of_mechanisms(&chain, moves.link)
            })
            .collect();
        (moves, placements)
    }

    fn scenes(&mut self) {
        let mut reported: BTreeSet<(Item, Item)> = BTreeSet::new();
        for (scene_index, _) in self.design.scenes.iter().enumerate().skip(1) {
            let (moves, placements) = self.scene_moves(scene_index);
            if moves.iter().all(Moves::is_empty) {
                continue;
            }
            let boxes: Vec<WorldBox> = self
                .bodies
                .iter()
                .zip(&placements)
                .map(|(body, placement)| placed_bounds(&body.bounds, placement))
                .collect();
            let mut clashes = Vec::new();
            for (i, j) in sweep(&boxes) {
                let (Some(a), Some(b)) = (self.bodies.get(i), self.bodies.get(j)) else {
                    continue;
                };
                let (Some(ma), Some(mb)) = (moves.get(i), moves.get(j)) else {
                    continue;
                };
                if ma == mb
                    || self.failed.contains(&a.item)
                    || self.failed.contains(&b.item)
                    || reported.contains(&ordered_items(a.item, b.item))
                    || self.exempt(a.item, b.item)
                {
                    continue;
                }
                let (Some(pa), Some(pb)) = (placements.get(i), placements.get(j)) else {
                    continue;
                };
                let found = collide::overlaps_placed(&a.solid, pa, &b.solid, pb);
                if !found.is_empty() {
                    // The item that moved (or moved further) comes first.
                    let swap = ma.is_empty() || (!mb.is_empty() && mb.steps.len() > ma.steps.len());
                    let (first, second, moved, other_moves) = if swap {
                        (b.item, a.item, mb.steps.clone(), ma.steps.clone())
                    } else {
                        (a.item, b.item, ma.steps.clone(), mb.steps.clone())
                    };
                    clashes.push((first, second, moved, other_moves, found));
                }
            }
            for (first, second, moved, other_moves, found) in clashes {
                reported.insert(ordered_items(first, second));
                self.posed_failed.insert(first);
                self.posed_failed.insert(second);
                self.motion(scene_index, first, second, &moved, &other_moves, &found);
            }
        }
    }

    fn assembly_name(&self, assembly: AssemblyId) -> String {
        self.design
            .assemblies()
            .get(assembly.index())
            .map(|a| a.name.clone())
            .unwrap_or_default()
    }

    fn motion(
        &mut self,
        scene_index: usize,
        a: Item,
        b: Item,
        moved: &[(AssemblyId, PoseValue)],
        other_moves: &[(AssemblyId, PoseValue)],
        found: &[Overlap],
    ) {
        let describe = |moves: &[(AssemblyId, PoseValue)]| -> Vec<String> {
            moves
                .iter()
                .map(|(assembly, value)| {
                    format!("{} is {}", self.assembly_name(*assembly), pose_text(*value))
                })
                .collect()
        };
        let mut conditions = describe(moved);
        for condition in describe(other_moves) {
            if !conditions.contains(&condition) {
                conditions.push(condition);
            }
        }
        let Some(scene) = self.design.scenes.get(scene_index) else {
            return;
        };
        // The pose line of the assembly that moved `a`.
        let place = scene_index
            .checked_sub(1)
            .and_then(|index| self.model.scenes.get(index))
            .and_then(|id| self.model.defs.scene(*id))
            .and_then(|def| {
                let moving = moved.last().map(|(assembly, _)| *assembly)?;
                let index = scene.poses.iter().position(|p| p.assembly == moving)?;
                let pose = def.poses.get(index)?;
                Some((def.decl.file, pose.span))
            });
        let deepest = deepest(found);
        let (a_name, b_name) = (self.item_name(a), self.item_name(b));
        let assembly = moved
            .last()
            .map(|(assembly, _)| self.assembly_name(*assembly))
            .unwrap_or_default();
        let mut notes = vec![format!("in scene '{}'", scene.name)];
        if let Some(deepest) = deepest {
            notes.push(format!(
                "{a_name} goes {} into {b_name} there",
                length_text(deepest.depth)
            ));
        }
        self.report(
            Severity::Error,
            Code::Motion,
            format!(
                "{a_name} hits {b_name} when {}",
                names::and_list(&conditions)
            ),
            place,
            notes,
            Some(format!(
                "pose {assembly} less far open in '{}', or move {b_name} out of its way",
                scene.name
            )),
        );
    }

    // -----------------------------------------------------------------------
    // Finishing
    // -----------------------------------------------------------------------

    /// One diagnostic per mistake: diagnostics with the same code at the
    /// same place are merged into the first, the others becoming notes.
    fn finish(self) -> Vec<(Diagnostic, Option<Contact>)> {
        merge(self.diagnostics)
    }
}

/// One diagnostic per mistake: diagnostics with the same code at the same
/// place are merged into the first, the others becoming a note and, in
/// full, [`Diagnostic::merged`].
fn merge(diagnostics: Vec<(Diagnostic, Option<Contact>)>) -> Vec<(Diagnostic, Option<Contact>)> {
    let mut merged: Vec<(Diagnostic, Option<Contact>)> = Vec::new();
    let mut extra: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    let key = |d: &Diagnostic| {
        (
            d.severity,
            d.code,
            d.primary().map(|l| (l.file, l.span.start(), l.span.end())),
        )
    };
    for (diagnostic, contact) in diagnostics {
        if let Some(index) = merged.iter().position(|(d, _)| key(d) == key(&diagnostic)) {
            if let Some((first, first_contact)) = merged.get_mut(index) {
                if first.message != diagnostic.message {
                    let list = extra.entry(index).or_default();
                    if !list.contains(&diagnostic.message) {
                        list.push(diagnostic.message);
                    }
                }
                // Show everything the line causes.
                if let (Some(first), Some(other)) = (first_contact.as_mut(), contact)
                    && first.scene == other.scene
                    && first.motion == other.motion
                {
                    for item in other.items {
                        if !first.items.contains(&item) {
                            first.items.push(item);
                        }
                    }
                    first.regions.extend(other.regions);
                }
            }
        } else {
            merged.push((diagnostic, contact));
        }
    }
    for (index, messages) in extra {
        if let Some((diagnostic, _)) = merged.get_mut(index) {
            diagnostic.merged.extend(messages.iter().cloned());
            let shown: Vec<String> = messages.iter().take(3).cloned().collect();
            let more = messages.len().saturating_sub(shown.len());
            let mut note = format!("the same line also causes: {}", shown.join("; "));
            if more > 0 {
                note.push_str(&format!("; and {more} more"));
            }
            diagnostic.notes.push(note);
        }
    }
    merged
}

/// The poses that move a body in a scene, outermost assembly first, and
/// the link it is when the innermost one turns it on its linkage.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Moves {
    steps: Vec<(AssemblyId, PoseValue)>,
    link: Option<MemberIndex>,
}

impl Moves {
    /// Nothing moves it.
    fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }
}

/// A cut's source, for grouping warnings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum SourceKey {
    Joint(stave_model::solved::JointId),
    Hardware(HardwareId),
    Boards(PartId),
}

impl SourceKey {
    fn of(source: CutSource) -> SourceKey {
        match source {
            CutSource::Joint { joint, .. } => SourceKey::Joint(joint),
            CutSource::Hardware { hardware } => SourceKey::Hardware(hardware),
            CutSource::Boards { part, .. } => SourceKey::Boards(part),
        }
    }
}

fn ordered_items(a: Item, b: Item) -> (Item, Item) {
    if a <= b { (a, b) } else { (b, a) }
}

/// The deepest overlap (the first of equals).
fn deepest(found: &[Overlap]) -> Option<&Overlap> {
    found.iter().rev().max_by_key(|o| o.depth)
}

/// The broad phase: index pairs whose boxes overlap by more than the
/// contact tolerance on every axis, by sorting along `across` and
/// sweeping. Pairs come back sorted.
fn sweep(boxes: &[WorldBox]) -> Vec<(usize, usize)> {
    use stave_lang::placement::Axis;
    let mut order: Vec<usize> = (0..boxes.len()).collect();
    order.sort_by_key(|&i| (boxes.get(i).map(|b| b.min().get(Axis::Across)), i));
    let mut active: Vec<usize> = Vec::new();
    let mut pairs = Vec::new();
    for i in order {
        let Some(current) = boxes.get(i) else {
            continue;
        };
        let start = current.min().get(Axis::Across).nm();
        active.retain(|&j| {
            boxes.get(j).is_some_and(|other| {
                other.max().get(Axis::Across).nm().saturating_sub(start) > CONTACT_TOLERANCE.nm()
            })
        });
        for &j in &active {
            if let Some(other) = boxes.get(j)
                && boxes_overlap(current, other)
            {
                pairs.push((i.min(j), i.max(j)));
            }
        }
        active.push(i);
    }
    pairs.sort();
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;
    use stave_lang::units::Length;
    use stave_model::solved::Point;

    fn cube(across: i64, size: i64) -> WorldBox {
        let mm = Length::from_mm;
        WorldBox::new(
            Point {
                across: mm(across),
                up: mm(0),
                deep: mm(0),
            },
            Point {
                across: mm(across + size),
                up: mm(size),
                deep: mm(size),
            },
        )
    }

    #[test]
    fn the_sweep_finds_exactly_the_overlapping_pairs() {
        let boxes = [
            cube(0, 10),
            cube(20, 10),
            cube(5, 10),
            cube(10, 10),
            cube(29, 3),
        ];
        // 0 and 2 overlap, 2 and 3 overlap, 1 and 4 overlap; 0 and 3 only
        // touch.
        assert_eq!(sweep(&boxes), vec![(0, 2), (1, 4), (2, 3)]);
    }
}
