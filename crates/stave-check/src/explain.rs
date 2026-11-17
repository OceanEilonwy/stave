//! What an overlap means, in woodworking words: which feature goes where
//! it shouldn't, by how much, and what to change.

use stave_geom::collide::Overlap;
use stave_lang::ast::CutOperation;
use stave_lang::placement::{Axis, Direction, PartAxis};
use stave_model::solved::{CutKind, CutPlace, Part, PartId, WorldBox};

use crate::names::length_text;
use crate::{Checker, Item, deepest};

/// The notes and help line of an overlap.
pub(crate) struct Explanation {
    pub(crate) notes: Vec<String>,
    pub(crate) help: String,
}

/// Explains an overlap between `a` and `b` (in message order).
pub(crate) fn overlap(checker: &Checker<'_>, a: Item, b: Item, found: &[Overlap]) -> Explanation {
    let (a_name, b_name) = (checker.item_name(a), checker.item_name(b));
    let Some(deepest) = deepest(found) else {
        return Explanation {
            notes: Vec::new(),
            help: format!("move {a_name} or {b_name} so they only touch"),
        };
    };
    let mut notes = Vec::new();
    let described = match (a, b) {
        (Item::Part(part), _) | (_, Item::Part(part)) => checker.design.part(part),
        _ => None,
    };
    let size = format!(
        "{} × {} × {}",
        length_text(deepest.region.size(Axis::Across))
            .trim_end_matches("mm")
            .to_owned(),
        length_text(deepest.region.size(Axis::Up))
            .trim_end_matches("mm")
            .to_owned(),
        length_text(deepest.region.size(Axis::Deep)),
    );
    match described {
        Some(part) => notes.push(format!(
            "they share {size} (across × up × deep) {}",
            whereabouts(part, &deepest.region)
        )),
        None => notes.push(format!("they share {size} (across × up × deep)")),
    }
    if let (Item::Part(x), Item::Part(y)) = (a, b)
        && let Some(help) =
            dovetail(checker, x, y, deepest).or_else(|| cavity(checker, x, y, found, deepest))
    {
        return Explanation { notes, help };
    }
    let direction = away(checker, a, deepest);
    let depth = length_text(deepest.depth);
    let joined = match (a, b) {
        (Item::Part(x), Item::Part(y)) => checker.shared_joint(x, y),
        _ => None,
    };
    let help = match (joined, direction) {
        (Some((joint, _)), Some(direction)) => format!(
            "{a_name} goes {depth} into {b_name}, and '{}' cuts no room for it there; move {a_name} {depth} {}, or change the joint's cuts",
            checker.joint_name(joint),
            direction.word()
        ),
        (None, Some(direction)) if matches!(a, Item::Part(_)) => format!(
            "{a_name} goes {depth} into {b_name}; move {a_name} {depth} {}, or join them with a joint that cuts room for one in the other",
            direction.word()
        ),
        (_, Some(direction)) => format!(
            "{a_name} goes {depth} into {b_name}; move {a_name} {depth} {}, or move {b_name} out of its way",
            direction.word()
        ),
        (_, None) => format!("{a_name} goes {depth} into {b_name}; move one of them"),
    };
    Explanation { notes, help }
}

/// The direction `item` moves to clear the overlap fastest: away from
/// the shared wood along the overlap's axis.
fn away(checker: &Checker<'_>, item: Item, overlap: &Overlap) -> Option<Direction> {
    let bounds = checker.bodies.iter().find(|b| b.item == item)?.bounds;
    let axis = overlap.axis;
    let middle =
        |b: &WorldBox| i128::from(b.min().get(axis).nm()) + i128::from(b.max().get(axis).nm());
    let positive = middle(&bounds) > middle(&overlap.region);
    Direction::ALL
        .into_iter()
        .find(|d| d.axis() == axis && d.is_positive() == positive)
}

/// Where a region lies on a part: "at the right end of apron[front]".
pub(crate) fn whereabouts(part: &Part, region: &WorldBox) -> String {
    // Twice the region's centre, to stay in whole nanometres.
    let mut best: Option<(i128, i128, PartAxis, bool)> = None;
    for axis in PartAxis::ALL {
        let direction = part.pose.orientation.direction(axis);
        let world = direction.axis();
        let centre2 =
            i128::from(region.min().get(world).nm()) + i128::from(region.max().get(world).nm());
        let origin2 = 2 * i128::from(part.pose.origin.get(world).nm());
        let local2 = if direction.is_positive() {
            centre2 - origin2
        } else {
            origin2 - centre2
        };
        let size2 = 2 * i128::from(part.cut.get(axis).nm());
        if size2 <= 0 {
            continue;
        }
        let off = (2 * local2 - size2).abs();
        let is_better = best.is_none_or(|(o, s, _, _)| off * s > o * size2);
        if is_better {
            best = Some((off, size2, axis, 2 * local2 > size2));
        }
    }
    let Some((_, _, axis, high)) = best else {
        return format!("in {}", part.name);
    };
    let direction = part.pose.orientation.direction(axis);
    let side = if high {
        direction
    } else {
        direction.opposite()
    };
    let word = match axis {
        PartAxis::Length => "end",
        PartAxis::Width => "edge",
        PartAxis::Thickness => "face",
    };
    format!("at the {} {word} of {}", side.face().word(), part.name)
}

/// A dovetail tail that doesn't fit its socket: the tail's sloped faces go
/// into the socket's.
fn dovetail(checker: &Checker<'_>, a: PartId, b: PartId, deepest: &Overlap) -> Option<String> {
    let design = checker.design;
    let ends = |part: PartId, other: PartId, socket: bool| {
        design.part(part).is_some_and(|p| {
            p.cuts.iter().any(|cut| {
                let toward = matches!(cut.place, CutPlace::End { toward } if toward == other);
                let kind = match cut.kind {
                    CutKind::DovetailSocket { .. } => socket,
                    CutKind::DovetailTail { .. } => !socket,
                    CutKind::Notch { .. }
                    | CutKind::Slot { .. }
                    | CutKind::Pocket { .. }
                    | CutKind::Tenon { .. }
                    | CutKind::Spigot { .. }
                    | CutKind::Round { .. }
                    | CutKind::Drill { .. }
                    | CutKind::Outline { .. } => false,
                };
                toward && kind
            })
        })
    };
    let (host, guest) = [(a, b), (b, a)]
        .into_iter()
        .find(|(host, guest)| ends(*host, *guest, true) && ends(*guest, *host, false))?;
    let (host, guest) = (&design.part(host)?.name, &design.part(guest)?.name);
    let depth = length_text(deepest.depth);
    Some(format!(
        "the sloped faces of the dovetail tail on {guest} go {depth} into the sloped sides of its socket in {host}; cut the tail and the socket with the same neck, depth and angle"
    ))
}

/// The cavity a joint cut in one part for the other, and how the other
/// misses it.
fn cavity(
    checker: &Checker<'_>,
    a: PartId,
    b: PartId,
    found: &[Overlap],
    deepest: &Overlap,
) -> Option<String> {
    let design = checker.design;
    let (host, guest, cut) = [(b, a), (a, b)].into_iter().find_map(|(host, guest)| {
        let part = design.part(host)?;
        part.cuts
            .iter()
            .find_map(|cut| match (&cut.place, cut.kind.operation()) {
                (
                    CutPlace::Crossing { other, .. },
                    CutOperation::Notch
                    | CutOperation::Slot
                    | CutOperation::Pocket
                    | CutOperation::Drill,
                ) if *other == guest => Some((host, guest, cut)),
                _ => None,
            })
    })?;
    let host_part = design.part(host)?;
    let guest_part = design.part(guest)?;
    let kind = match cut.source {
        stave_model::solved::CutSource::Joint { joint, .. } => design
            .joints
            .get(joint.index())
            .map(|j| j.kind.clone())
            .unwrap_or_default(),
        stave_model::solved::CutSource::Hardware { .. } => String::new(),
    };
    let noun = cavity_noun(cut.kind.operation(), &kind);
    let (feature, short) = guest_part
        .cuts
        .iter()
        .find_map(|c| match (&c.kind, &c.place) {
            (CutKind::Tenon { .. }, CutPlace::End { toward }) if *toward == host => Some((
                format!("the tenon on {}", guest_part.name),
                "the tenon".to_owned(),
            )),
            (CutKind::Spigot { .. }, CutPlace::End { toward }) if *toward == host => Some((
                format!("the round tenon on {}", guest_part.name),
                "the round tenon".to_owned(),
            )),
            _ => None,
        })
        .unwrap_or_else(|| (guest_part.name.clone(), guest_part.name.clone()));
    let normal = host_part.pose.orientation.direction(cut.face.axis).axis();
    let host_name = &host_part.name;
    let depth = length_text(deepest.depth);
    if deepest.axis == normal {
        return Some(format!(
            "{feature} goes {depth} past the bottom of its {noun} in {host_name}; make the {noun} {depth} deeper, or {short} {depth} shorter"
        ));
    }
    // Into the walls: on one side, or on both (too thick).
    let axis = deepest.axis;
    let guest_box = guest_part.cut_box();
    let middle =
        |b: &WorldBox| i128::from(b.min().get(axis).nm()) + i128::from(b.max().get(axis).nm());
    let centre = middle(&guest_box);
    let side_depth = |positive: bool| {
        found
            .iter()
            .filter(|o| o.axis == axis && (middle(&o.region) > centre) == positive)
            .map(|o| o.depth)
            .max()
    };
    let local = guest_part.pose.orientation.part_axis(axis);
    match (side_depth(false), side_depth(true)) {
        (Some(low), Some(high)) => {
            let total = stave_lang::units::Length::from_nm(low.nm().saturating_add(high.nm()));
            let total = length_text(total);
            let (more, less) = match local {
                PartAxis::Thickness => ("thicker", "thinner"),
                PartAxis::Width => ("wider", "narrower"),
                PartAxis::Length => ("longer", "shorter"),
            };
            Some(format!(
                "{feature} is {total} {more} than its {noun} in {host_name}; make the {noun} {total} wider, or {short} {total} {less}"
            ))
        }
        (low, high) => {
            let positive = high.is_some() && low.is_none();
            let side = Direction::ALL
                .into_iter()
                .find(|d| d.axis() == axis && d.is_positive() == positive)?;
            Some(format!(
                "{feature} goes {depth} past the {} side of its {noun} in {host_name}; move the {noun} {depth} {}, or move {} {depth} {}",
                side.face().word(),
                side.word(),
                guest_part.name,
                side.opposite().word()
            ))
        }
    }
}

/// What a beginner calls the hole a cut makes: the joint's own word when
/// it is one (`dado`, `rabbet`, `groove`), otherwise the operation's.
fn cavity_noun(operation: CutOperation, kind: &str) -> &'static str {
    let named = ["dado", "rabbet", "groove", "mortise", "housing", "socket"]
        .into_iter()
        .find(|word| kind.contains(word));
    match operation {
        CutOperation::Pocket => "mortise",
        CutOperation::Drill => named.filter(|w| *w == "socket").unwrap_or("hole"),
        CutOperation::Notch => named.unwrap_or("notch"),
        CutOperation::Slot => named.unwrap_or("slot"),
        CutOperation::Tenon
        | CutOperation::Spigot
        | CutOperation::Round
        | CutOperation::DovetailSocket
        | CutOperation::DovetailTail
        | CutOperation::Outline => "cut",
    }
}
