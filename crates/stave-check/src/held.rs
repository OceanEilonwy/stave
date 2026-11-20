//! Held (grammar, "Held"): every part is joined to the ground part
//! through joints and hardware, or says it is meant to lift out.
//!
//! The parts a joint pair binds (and the parts it makes) hold each other,
//! as do a part made of boards and the parts its board joints make;
//! a piece of hardware holds the part it is mounted on, the parts its
//! roles name, the parts that carry its members and the parts its members
//! touch (a slide's runner fixed to a cabinet side through the opening it
//! is placed in). Parts that hold each other form groups; the group with
//! the ground part is held; when the root is a box, every group with a
//! part standing on the floor (the box's bottom face) is. Every other group is `warning[loose-part]`,
//! once, at its first part, unless one of its parts says `loose: true` (a
//! loose shelf, a tray: the whole group lifts out together). It is asked
//! only of a design with no errors.

use std::collections::BTreeSet;

use stave_lang::placement::Axis;
use stave_lang::{Code, Severity};
use stave_model::defs::{PartKind, PlaceTermKind};
use stave_model::solved::{
    BodyCarrier, CutSource, JointId, PairIndex, PartId, PartOrigin, RoleValue, WorldBox,
};

use crate::names::list;
use crate::{Checker, Contact, Item};

/// A list of names, the first four and how many more: `a, b, c, d and 11
/// more`.
fn few(names: &[String]) -> String {
    const SHOWN: usize = 4;
    if names.len() <= SHOWN + 1 {
        return list(names);
    }
    let shown: Vec<String> = names.iter().take(SHOWN).cloned().collect();
    format!("{} and {} more", shown.join(", "), names.len() - SHOWN)
}

/// The group a part is in, with path halving.
fn root(parent: &mut [usize], mut index: usize) -> usize {
    while let Some(&up) = parent.get(index) {
        if up == index {
            break;
        }
        let grand = parent.get(up).copied().unwrap_or(up);
        if let Some(entry) = parent.get_mut(index) {
            *entry = grand;
        }
        index = grand;
    }
    index
}

/// Puts two parts in one group; the lower index stays the root, so the
/// groups don't depend on the order things are joined in.
fn join(parent: &mut [usize], a: usize, b: usize) {
    let (a, b) = (root(parent, a), root(parent, b));
    let (low, high) = if a <= b { (a, b) } else { (b, a) };
    if let Some(entry) = parent.get_mut(high) {
        *entry = low;
    }
}

/// Puts every part of a list in one group.
fn bind(parent: &mut [usize], parts: &[PartId]) {
    if let Some((first, rest)) = parts.split_first() {
        for part in rest {
            join(parent, first.index(), part.index());
        }
    }
}

/// `true` when two boxes touch or overlap over an area (not only along an
/// edge or at a corner).
fn touch(a: &WorldBox, b: &WorldBox) -> bool {
    shared(a, b).is_some()
}

/// How much two boxes share where they touch or overlap over an area:
/// the product of their overlaps along the axes they overlap on (the
/// area of the touching faces, or the shared volume). `None` when they
/// are apart or meet only along an edge.
fn shared(a: &WorldBox, b: &WorldBox) -> Option<i128> {
    let mut overlapping = 0;
    let mut product: i128 = 1;
    for axis in Axis::ALL {
        let low = a.min().get(axis).max(b.min().get(axis));
        let high = a.max().get(axis).min(b.max().get(axis));
        if low > high {
            return None;
        }
        if low < high {
            overlapping += 1;
            product = product.saturating_mul(i128::from(high.nm()) - i128::from(low.nm()));
        }
    }
    (overlapping >= 2).then_some(product)
}

/// What a held group is joined to.
#[derive(Debug, Clone, Copy)]
enum Holder {
    /// The part placed with `ground`.
    Ground(PartId),
    /// The floor, the root box's bottom face: a group with a part whose
    /// bottom face lies on it stands on it.
    Floor(stave_lang::units::Length),
}

impl Checker<'_> {
    /// The part placed with `ground`.
    fn ground_part(&self) -> Option<PartId> {
        self.model.parts.iter().find_map(|inst| {
            if inst.mirror_of.is_some() || inst.assembly.is_some() {
                return None;
            }
            let def = self.model.defs.part(inst.def)?;
            let PartKind::FromStock(body) = &def.kind else {
                return None;
            };
            body.place
                .iter()
                .any(|term| term.kind == PlaceTermKind::Ground)
                .then_some(inst.id)
        })
    }

    pub(crate) fn held(&mut self) {
        let design = self.design;
        // What holds the design up: the ground part, or, when the root is
        // a box, the floor (its bottom face).
        let anchor = match (self.ground_part(), design.root_box()) {
            (Some(ground), _) => Holder::Ground(ground),
            (None, Some(root)) => Holder::Floor(root.bounds.min().up),
            (None, None) => return,
        };
        let count = design.parts().len();
        let mut parent: Vec<usize> = (0..count).collect();
        // A pair holds when it is glued, places a member or a fastener,
        // makes a part, or cuts its parts; one that does none of these (a
        // count of 0 screws with no glue) only says the parts are joined.
        let cut_pairs: BTreeSet<(JointId, PairIndex)> = design
            .parts()
            .iter()
            .flat_map(|part| &part.cuts)
            .filter_map(|cut| match cut.source {
                CutSource::Joint { joint, pair } => Some((joint, pair)),
                CutSource::Hardware { .. }
                | CutSource::Boards { .. }
                | CutSource::Edges { .. }
                | CutSource::Shape { .. } => None,
            })
            .collect();
        // Pairs that join nothing, with their joint's name.
        let mut idle: Vec<(Vec<PartId>, &str)> = Vec::new();
        for joint in design.joints() {
            let fastened = design
                .fasteners
                .iter()
                .any(|use_| use_.joint == joint.id && use_.count > 0);
            for (index, pair) in joint.indexed_pairs() {
                let mut parts: Vec<PartId> = pair.roles.iter().map(|r| r.part).collect();
                let holds = joint.glue
                    || fastened
                    || !pair.members.is_empty()
                    || !pair.made_parts.is_empty()
                    || cut_pairs.contains(&(joint.id, index));
                if !holds {
                    idle.push((parts, &joint.name));
                    continue;
                }
                parts.extend(pair.made_parts.iter().copied());
                bind(&mut parent, &parts);
            }
        }
        // A part a board joint makes (a spline, a butterfly key) holds the
        // part made of boards it joins.
        for part in design.parts() {
            if let PartOrigin::Boards { part: whole, .. } = part.origin {
                bind(&mut parent, &[whole, part.id]);
            }
        }
        for hardware in design.hardware() {
            let mut parts = vec![hardware.piece];
            for (_, value) in &hardware.roles {
                match value {
                    RoleValue::Part(part) => parts.push(*part),
                    RoleValue::Region(_) => {}
                }
            }
            for member in &hardware.members {
                match member.carrier {
                    BodyCarrier::Part(part) => parts.push(part),
                    BodyCarrier::Region(_) | BodyCarrier::Link(_) => {}
                }
                let bounds = member.bounds();
                parts.extend(
                    design
                        .parts()
                        .iter()
                        .filter(|part| touch(&part.cut_box(), &bounds))
                        .map(|part| part.id),
                );
            }
            bind(&mut parent, &parts);
        }
        let roots: Vec<usize> = (0..count).map(|index| root(&mut parent, index)).collect();
        let group_of = |part: PartId| roots.get(part.index()).copied();
        let held: Vec<Option<usize>> = match anchor {
            Holder::Ground(ground) => vec![group_of(ground)],
            Holder::Floor(floor) => design
                .parts()
                .iter()
                .filter(|part| part.drawn_box().min().up == floor)
                .map(|part| group_of(part.id))
                .collect(),
        };
        let is_held = |part: PartId| held.contains(&group_of(part));
        let mut groups: Vec<(Option<usize>, Vec<PartId>)> = Vec::new();
        for part in design.parts() {
            let group = group_of(part.id);
            if held.contains(&group) {
                continue;
            }
            match groups.iter_mut().find(|(root, _)| *root == group) {
                Some((_, parts)) => parts.push(part.id),
                None => groups.push((group, vec![part.id])),
            }
        }
        for (_, parts) in groups {
            if parts
                .iter()
                .any(|part| design.part(*part).is_some_and(|p| p.loose))
            {
                continue;
            }
            let names: Vec<String> = parts.iter().map(|p| self.part_name(*p)).collect();
            let Some(first) = parts.first().copied() else {
                continue;
            };
            let to = match anchor {
                Holder::Ground(ground) => {
                    let ground_name = self.part_name(ground);
                    format!("{ground_name} or to anything joined to {ground_name}")
                }
                Holder::Floor(_) => "a part that stands on the floor".to_owned(),
            };
            let idle_joint = idle
                .iter()
                .find(|(pair, _)| pair.iter().any(|part| parts.contains(part)))
                .map(|(_, joint)| *joint);
            let message = match (names.as_slice(), idle_joint) {
                ([only], Some(joint)) => format!(
                    "{only} is loose: {joint} is not glued and cuts or places nothing to hold it"
                ),
                (_, Some(joint)) => format!(
                    "{} are loose: {joint} is not glued and cuts or places nothing to hold them",
                    few(&names)
                ),
                ([only], None) => {
                    format!("{only} is loose: no joint or hardware joins it to {to}")
                }
                (_, None) => format!(
                    "{} are joined only to each other: no joint or hardware joins them to {to}",
                    few(&names)
                ),
            };
            // A held part one of them touches, to join it to: the one it
            // touches most (a slat rests on a stretcher more than it
            // brushes a leg), the first of equals.
            let mut touching: Option<(i128, PartId, PartId)> = None;
            for part in &parts {
                let Some(own) = design.part(*part).map(|p| p.drawn_box()) else {
                    continue;
                };
                for other in design.parts().iter().filter(|o| is_held(o.id)) {
                    if let Some(area) = shared(&own, &other.drawn_box())
                        && touching.is_none_or(|(best, _, _)| area > best)
                    {
                        touching = Some((area, *part, other.id));
                    }
                }
            }
            let touching = touching.map(|(_, part, other)| (part, other));
            let help = match (idle_joint, touching) {
                (Some(joint), _) => format!(
                    "glue {joint} (glue: true), or join the parts with a kind that cuts them or places a fastener"
                ),
                (None, touching) => match touching {
                    Some((part, other)) => format!(
                        "join {} to {}, which it touches, with a joint or hardware, or write loose: true in the part if it is meant to lift out",
                        self.part_name(part),
                        self.part_name(other)
                    ),
                    None => format!(
                        "place {} against a part it should be joined to and join them with a joint or hardware, or write loose: true in the part if it is meant to lift out",
                        self.part_name(first)
                    ),
                },
            };
            let items: Vec<Item> = parts.iter().map(|p| Item::Part(*p)).collect();
            self.next_contact = Some(Contact::of(self.design, &items, &[]));
            self.report(
                Severity::Warning,
                Code::LoosePart,
                message,
                self.part_place(first),
                Vec::new(),
                Some(help),
            );
        }
    }
}
