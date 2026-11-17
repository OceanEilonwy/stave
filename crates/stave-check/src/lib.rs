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
//!    at least one of its counterparts, after every cut.
//! 3. **Spacing** (`error[overlap]`): an element spaced with `pitch` or
//!    `gap` that runs past its `to` anchor. **Slide clearance**
//!    (`error[slide-gap]`): the gap between a slide's piece and the side of
//!    its opening is exactly the slide's `clearance`.
//! 4. **Overlap at rest** (`error[overlap]`): one body for every part's
//!    cut solid and one for every hardware member's shapes, each against
//!    every other (grammar, "One collision model"). A cup sits in its door
//!    only because its kind bores the hole; the one rule for every kind is
//!    that members of one piece of hardware are not checked against each
//!    other. A broad phase sorts bounding boxes along `across` and sweeps
//!    (a fixed order, so the result is the same on every run); the narrow
//!    phase measures the shared wood exactly ([`stave_geom::collide`]).
//!    Touching within the 0.05mm contact tolerance is fine.
//! 5. **Scene poses** (`error[motion]`): each scene's poses applied, every
//!    moved body against everything that moved differently (a link its
//!    hinge's mechanism turns moves differently from its door).
//! 6. **Sweeps** (`error[motion]`, [`motion`]): every motion through its
//!    whole travel, every body it moves against every body that moves
//!    differently.
//!
//! **One mistake, one error.** The checker runs only on a design that
//! solved without errors. A part reported for spacing or for a misshapen
//! cut, and a slide reported for its gap, are left out of the overlap
//! checks, and an item with an error at rest is left out of the
//! scene checks. When one line causes several problems (a component used
//! twice, a joint applied to four pairs), they are reported once, at that
//! line, with the others as notes.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod explain;
mod hinges;
mod instance;
mod motion;
mod names;
mod sequence;

use std::collections::{BTreeMap, BTreeSet};

use stave_geom::collide::{
    self, CONTACT_TOLERANCE, Overlap, Placement, WorldSolid, boxes_overlap, placed_bounds,
};
use stave_geom::solid::Solid;
use stave_lang::ast::Span;
use stave_lang::units::Length;
use stave_lang::{Code, Diagnostic, FileId, Label, Severity};
use stave_model::Model;
use stave_model::solved::{
    AnchorItem, AssemblyId, BodyCarrier, BodyRef, CutSource, FixedSpacing, HardwareData,
    HardwareId, LinkIndex, PartId, PartOrigin, PoseValue, SolvedDesign, WorldBox,
};

use crate::names::{length_text, pose_text};

/// Something a diagnostic is about, for a viewer to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ContactItem {
    /// A part.
    Part(PartId),
    /// A piece of hardware (either member).
    Hardware(HardwareId),
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
    fn add_item(&mut self, item: Item) {
        let item = match item {
            Item::Part(part) => ContactItem::Part(part),
            Item::Member(member) => ContactItem::Hardware(member.hardware),
        };
        if !self.items.contains(&item) {
            self.items.push(item);
        }
    }

    /// A contact about some items and the wood they share.
    fn of(items: &[Item], found: &[Overlap]) -> Contact {
        let mut contact = Contact::default();
        for item in items {
            contact.add_item(*item);
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
    let mut checker = Checker::new(model, design);
    checker.misshapen();
    checker.not_supported();
    checker.contact();
    checker.overruns();
    checker.slide_gaps();
    checker.hinge_overlays();
    checker.rest();
    checker.scenes();
    checker.sweeps();
    checker.sequences();
    checker.finish()
}

/// Something that takes up room: a part, or a member of a piece of
/// hardware (grammar, "One collision model").
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Item {
    /// A member of a piece of hardware: all its shapes.
    Member(BodyRef),
    /// A part.
    Part(PartId),
}

impl Item {
    /// The piece of hardware a member belongs to.
    fn hardware(self) -> Option<HardwareId> {
        match self {
            Item::Member(member) => Some(member.hardware),
            Item::Part(_) => None,
        }
    }
}

/// An item's solid and where it moves.
struct Body {
    item: Item,
    solid: WorldSolid,
    bounds: WorldBox,
    /// The assemblies it moves with, outermost first.
    chain: Vec<AssemblyId>,
    /// For a link its hardware's mechanism turns: the assembly whose
    /// linkage turns it, and which link.
    link: Option<(AssemblyId, LinkIndex)>,
}

struct Checker<'a> {
    model: &'a Model,
    design: &'a SolvedDesign,
    solids: Vec<Solid>,
    bodies: Vec<Body>,
    /// Parts already reported, for their spacing or for a cut whose shape
    /// is a mistake: an overlap would report the same mistake again.
    reported: BTreeSet<PartId>,
    /// Items with an error at rest.
    failed: BTreeSet<Item>,
    /// Items with an error at a scene's poses.
    posed_failed: BTreeSet<Item>,
    /// Slides reported for their gap: their members are left out of the
    /// overlap checks, which would only report the same mistake again.
    misfit_slides: BTreeSet<HardwareId>,
    diagnostics: Vec<(Diagnostic, Option<Contact>)>,
    /// What the next reported diagnostic is about.
    next_contact: Option<Contact>,
}

impl<'a> Checker<'a> {
    fn new(model: &'a Model, design: &'a SolvedDesign) -> Self {
        let solids: Vec<Solid> = design
            .parts
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
                assembly = design.assemblies.get(id.index()).and_then(|a| a.parent);
            }
            chain.reverse();
            chain
        };
        let mut bodies = Vec::new();
        for (part, solid) in design.parts.iter().zip(&solids) {
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
        for hardware in &design.hardware {
            for member in &hardware.members {
                let (assembly, link) = match member.carrier {
                    BodyCarrier::Part(part) => (design.part(part).and_then(|p| p.assembly), None),
                    BodyCarrier::Region(region) => (
                        design.regions.get(region.index()).and_then(|r| r.assembly),
                        None,
                    ),
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
        Checker {
            model,
            design,
            solids,
            bodies,
            reported: BTreeSet::new(),
            failed: BTreeSet::new(),
            posed_failed: BTreeSet::new(),
            misfit_slides: BTreeSet::new(),
            diagnostics: Vec::new(),
            next_contact: None,
        }
    }

    // -----------------------------------------------------------------------
    // Where things are written
    // -----------------------------------------------------------------------

    /// The kind call of a joint.
    fn joint_place(&self, joint: stave_model::solved::JointId) -> Option<(FileId, Span)> {
        let inst = self.model.joints.get(joint.index())?;
        let def = self.model.defs.joint(inst.def)?;
        Some((def.decl.file, def.call.span))
    }

    /// The kind call of a piece of hardware.
    fn hardware_place(&self, hardware: HardwareId) -> Option<(FileId, Span)> {
        let inst = self.model.hardware.get(hardware.index())?;
        let def = self.model.defs.hardware_def(inst.def)?;
        Some((def.decl.file, def.call.span))
    }

    /// A part's name where it is declared (a made part: its joint).
    fn part_place(&self, part: PartId) -> Option<(FileId, Span)> {
        if let Some(PartOrigin::Made { joint, .. }) = self.design.part(part).map(|p| p.origin) {
            return self.joint_place(joint);
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
        self.diagnostics.push((diagnostic, contact));
    }

    /// A part's name, or a member's as `<hardware>'s <member>`.
    fn item_name(&self, item: Item) -> String {
        match item {
            Item::Part(part) => self.part_name(part),
            Item::Member(member) => {
                let hardware = self.design.hardware.get(member.hardware.index());
                let name = hardware
                    .and_then(|h| h.members.iter().find(|m| m.id == member))
                    .map(|m| m.name.as_str())
                    .unwrap_or_default();
                format!("{}'s {name}", self.hardware_name(member.hardware))
            }
        }
    }

    fn hardware_name(&self, hardware: HardwareId) -> String {
        self.design
            .hardware
            .get(hardware.index())
            .map(|h| h.name.clone())
            .unwrap_or_default()
    }

    /// Every member of a piece of hardware, as items.
    fn members_of(&self, hardware: HardwareId) -> Vec<Item> {
        self.design
            .hardware
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

    fn joint_name(&self, joint: stave_model::solved::JointId) -> String {
        self.design
            .joints
            .get(joint.index())
            .map(|j| j.name.clone())
            .unwrap_or_default()
    }

    // -----------------------------------------------------------------------
    // 1. Cuts that can't be made yet, and cuts whose shape is a mistake
    // -----------------------------------------------------------------------

    fn misshapen(&mut self) {
        let mut found = Vec::new();
        for (part, solid) in self.design.parts.iter().zip(&self.solids) {
            for misshapen in solid.misshapen() {
                if let Some(cut) = part.cuts.get(misshapen.cut) {
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
            };
            let note = format!(
                "{owner}: {} is drawn without this cut until it is fixed",
                self.part_name(part)
            );
            self.next_contact = Some(Contact::of(&[Item::Part(part)], &[]));
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
        for (part, solid) in self.design.parts.iter().zip(&self.solids) {
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
            self.next_contact = Some(Contact::of(&items, &[]));
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
        for joint in &self.design.joints {
            // Every listed part, in role order, and whether it touches a
            // counterpart.
            let mut touched: Vec<(PartId, bool)> = Vec::new();
            for pair in &joint.pairs {
                let parts: Vec<PartId> = pair.roles.iter().map(|r| r.part).collect();
                for (index, part) in parts.iter().enumerate() {
                    let position = match touched.iter().position(|(p, _)| p == part) {
                        Some(position) => position,
                        None => {
                            touched.push((*part, false));
                            touched.len() - 1
                        }
                    };
                    if touched.get(position).is_some_and(|(_, done)| *done) {
                        continue;
                    }
                    let Some(own) = self.world_part(*part) else {
                        continue;
                    };
                    let touches_any = parts.iter().enumerate().any(|(other_index, other)| {
                        other_index != index
                            && other != part
                            && self
                                .world_part(*other)
                                .is_some_and(|solid| collide::touches(&own, &solid))
                    });
                    if touches_any && let Some(entry) = touched.get_mut(position) {
                        entry.1 = true;
                    }
                }
            }
            let mut apart: Vec<PartId> = joint.unmatched.clone();
            for (part, touches) in touched {
                if !touches && !apart.contains(&part) {
                    apart.push(part);
                }
            }
            for part in apart {
                problems.push((joint.id, part));
            }
        }
        for (joint, part) in problems {
            let (joint_name, part_name) = (self.joint_name(joint), self.part_name(part));
            self.next_contact = Some(Contact::of(&[Item::Part(part)], &[]));
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
                    .regions
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
            let contact = Contact::of(&[Item::Part(overrun.part)], &[]);
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
                    help: Some(help),
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
    // Slide clearance
    // -----------------------------------------------------------------------

    /// A slide needs exactly its `clearance` between the piece's mount
    /// face and the facing side of its opening.
    fn slide_gaps(&mut self) {
        let mut found = Vec::new();
        for hardware in &self.design.hardware {
            let HardwareData::Slide {
                clearance, opening, ..
            } = hardware.data
            else {
                continue;
            };
            let (Some(piece), Some(region)) = (
                self.design.part(hardware.piece),
                self.design.regions.get(opening.index()),
            ) else {
                continue;
            };
            let face = hardware.mount_face;
            let mount = piece.drawn_box().face(face).nm();
            let side = region.bounds.face(face).nm();
            let gap = if face.is_positive() {
                side.saturating_sub(mount)
            } else {
                mount.saturating_sub(side)
            };
            if gap != clearance.nm() {
                found.push((hardware.id, piece.id, Length::from_nm(gap), clearance));
            }
        }
        for (hardware, piece, gap, clearance) in found {
            self.misfit_slides.insert(hardware);
            let (name, piece_name) = (self.hardware_name(hardware), self.part_name(piece));
            let difference =
                Length::from_nm(clearance.nm().saturating_sub(gap.nm()).saturating_abs());
            let help = if gap < clearance {
                format!(
                    "make the drawer {} narrower on this side, so {piece_name} is {} from the side of the opening",
                    length_text(difference),
                    length_text(clearance)
                )
            } else {
                format!(
                    "make the drawer {} wider on this side, so {piece_name} is {} from the side of the opening",
                    length_text(difference),
                    length_text(clearance)
                )
            };
            let mut items = self.members_of(hardware);
            items.push(Item::Part(piece));
            self.next_contact = Some(Contact::of(&items, &[]));
            self.report(
                Severity::Error,
                Code::SlideGap,
                format!(
                    "{name}: {piece_name} is {} from the opening, but the slide needs {}",
                    length_text(gap),
                    length_text(clearance)
                ),
                self.hardware_place(hardware),
                Vec::new(),
                Some(help),
            );
        }
    }

    // -----------------------------------------------------------------------
    // 4. Overlap at rest
    // -----------------------------------------------------------------------

    /// Items that may share room: two members of one piece of hardware (a
    /// kind's own members are not checked against each other, for every
    /// kind alike), the members of a slide already reported for its gap,
    /// and parts already reported (for their spacing, or for a cut whose
    /// shape is a mistake). One mistake, one error.
    fn exempt(&self, a: Item, b: Item) -> bool {
        if [a, b]
            .into_iter()
            .filter_map(Item::hardware)
            .any(|h| self.misfit_slides.contains(&h))
        {
            return true;
        }
        if let (Some(x), Some(y)) = (a.hardware(), b.hardware()) {
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
            self.report_group(group);
        }
        for (a, b, found) in rest {
            self.overlap(a, b, &found);
        }
    }

    /// The joint pair two parts are in together, if any.
    fn shared_joint(&self, a: PartId, b: PartId) -> Option<(stave_model::solved::JointId, usize)> {
        self.design.joints.iter().find_map(|joint| {
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
                            .joints
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
            (Item::Member(member), _) | (_, Item::Member(member)) => {
                self.hardware_place(member.hardware)
            }
        };
        let explanation = explain::overlap(self, a, b, found);
        self.next_contact = Some(Contact::of(&[a, b], found));
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
                        self.design
                            .assemblies
                            .get(assembly.index())?
                            .motion
                            .map(|_| (*assembly, value))
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
                let motions: Vec<_> = moves
                    .steps
                    .iter()
                    .filter_map(|(assembly, value)| {
                        let motion = self.design.assemblies.get(assembly.index())?.motion?;
                        Some((motion, *value))
                    })
                    .collect();
                Placement::of_member(&motions, moves.link)
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
            .assemblies
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
        let mut merged: Vec<(Diagnostic, Option<Contact>)> = Vec::new();
        let mut extra: BTreeMap<usize, Vec<String>> = BTreeMap::new();
        let key = |d: &Diagnostic| {
            (
                d.severity,
                d.code,
                d.primary().map(|l| (l.file, l.span.start(), l.span.end())),
            )
        };
        for (diagnostic, contact) in self.diagnostics {
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
}

/// The poses that move a body in a scene, outermost assembly first, and
/// the link it is when the innermost one turns it on its linkage.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Moves {
    steps: Vec<(AssemblyId, PoseValue)>,
    link: Option<LinkIndex>,
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
}

impl SourceKey {
    fn of(source: CutSource) -> SourceKey {
        match source {
            CutSource::Joint { joint, .. } => SourceKey::Joint(joint),
            CutSource::Hardware { hardware } => SourceKey::Hardware(hardware),
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
