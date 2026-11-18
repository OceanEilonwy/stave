//! The checks in two tiers, for an editor that checks a candidate while
//! the pointer moves (grammar, "Fast and slow diagnostics").
//!
//! - **Fast** ([`check_fast`]): layers 1 to 3 (syntax, names and types,
//!   placement and sizes) are done by the stages before this crate; the
//!   fast tier adds box-level overlap: each part's board box and each
//!   member's box at rest, against every other, keeping the pairs whose
//!   boxes share more than the contact tolerance and that nothing explains
//!   (they are not joined to each other, and neither cuts room in the
//!   other). It needs no cut solids, no mechanism and no sweep, so it takes
//!   microseconds. Its findings are not diagnostics: a box is bigger than
//!   the wood in it, so a box overlap means "these may overlap"; every
//!   overlap at rest between two such items is among them.
//! - **Slow** ([`check_slow`]): everything this crate checks (fit, scene
//!   poses, motion through its travel, held, models), exactly as
//!   [`crate::check_with_contacts`]: the same diagnostics, in the same
//!   order. It can be cancelled: the flag is read between checks and
//!   between the motions of the sweep check, the costly one.

use std::convert::Infallible;
use std::sync::atomic::{AtomicBool, Ordering};

use stave_geom::collide::boxes_overlap;
use stave_lang::Diagnostic;
use stave_model::Model;
use stave_model::solved::{
    Body, BodyOwner, BodyRef, CutSource, HardwareId, PairMember, PartId, PartOrigin, Rules,
    SolvedDesign, WorldBox,
};

use crate::{Contact, ContactItem};

/// Two items whose boxes overlap at rest: what the fast tier finds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoxOverlap {
    /// The first item (the smaller of the two).
    pub a: ContactItem,
    /// The second item.
    pub b: ContactItem,
    /// Where their boxes overlap, one box per pair of bodies that do.
    pub regions: Vec<WorldBox>,
}

/// The slow tier was cancelled before it finished; what it had found is
/// dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the check was cancelled before it finished")]
pub struct Cancelled;

/// The fast tier: every pair of items whose boxes overlap at rest and that
/// nothing explains, sorted by item. Cheap enough to run on every pointer
/// move.
pub fn check_fast(design: &SolvedDesign) -> Vec<BoxOverlap> {
    let mut boxes: Vec<(Thing, WorldBox)> = design
        .parts
        .iter()
        .map(|part| (Thing::Part(part.id), part.pose.world_box(part.cut)))
        .collect();
    let members = design
        .hardware
        .iter()
        .flat_map(|hardware| &hardware.members)
        .chain(
            design
                .joints
                .iter()
                .flat_map(|joint| &joint.pairs)
                .flat_map(|pair| &pair.members),
        );
    // A member's shapes one by one: a member's box round all of them
    // (a screw and its head, a hinge's arm) holds a lot of air.
    boxes.extend(members.flat_map(|member: &Body| {
        member
            .shapes()
            .iter()
            .map(|shape| (Thing::Member(member.id), shape.bounds()))
    }));
    let explained = Explained::of(design);
    let only_boxes: Vec<WorldBox> = boxes.iter().map(|(_, bounds)| *bounds).collect();
    let mut found: Vec<BoxOverlap> = Vec::new();
    for (i, j) in crate::sweep(&only_boxes) {
        let (Some((a, a_box)), Some((b, b_box))) = (boxes.get(i), boxes.get(j)) else {
            continue;
        };
        if !boxes_overlap(a_box, b_box) || explained.covers(*a, *b) {
            continue;
        }
        let Some(region) = intersection(a_box, b_box) else {
            continue;
        };
        let (first, second) = {
            let (x, y) = (a.item(), b.item());
            if y < x { (y, x) } else { (x, y) }
        };
        match found
            .iter_mut()
            .find(|overlap| overlap.a == first && overlap.b == second)
        {
            Some(overlap) => overlap.regions.push(region),
            None => found.push(BoxOverlap {
                a: first,
                b: second,
                regions: vec![region],
            }),
        }
    }
    found.sort_by_key(|overlap| (overlap.a, overlap.b));
    found
}

/// The slow tier: every check of [`crate::check_with_contacts`], with the
/// same result, unless `cancel` is set before it finishes. Set `cancel`
/// from another thread (or from a callback) to stop it at the next point
/// it is read.
pub fn check_slow(
    model: &Model,
    design: &SolvedDesign,
    cancel: &AtomicBool,
) -> Result<Vec<(Diagnostic, Option<Contact>)>, Cancelled> {
    crate::run(model, design, cancel)
}

/// Something that may ask the checks to stop.
pub(crate) trait Stop {
    /// What stopping gives.
    type Stopped;
    /// `true` once asked to stop; stays `true`.
    fn stopped(&self) -> bool;
    /// `Err` once asked to stop.
    fn check(&self) -> Result<(), Self::Stopped>;
}

/// Never stops: the single check.
pub(crate) struct Never;

impl Stop for Never {
    type Stopped = Infallible;

    fn stopped(&self) -> bool {
        false
    }

    fn check(&self) -> Result<(), Infallible> {
        Ok(())
    }
}

impl Stop for AtomicBool {
    type Stopped = Cancelled;

    fn stopped(&self) -> bool {
        self.load(Ordering::Relaxed)
    }

    fn check(&self) -> Result<(), Cancelled> {
        if self.stopped() {
            Err(Cancelled)
        } else {
            Ok(())
        }
    }
}

/// A part or a member, in the fast tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Thing {
    Part(PartId),
    Member(BodyRef),
}

impl Thing {
    fn item(self) -> ContactItem {
        match self {
            Thing::Part(part) => ContactItem::Part(part),
            Thing::Member(member) => match member.owner {
                BodyOwner::Hardware { hardware } => ContactItem::Hardware(hardware),
                BodyOwner::Joint { joint, pair } => ContactItem::Member(PairMember {
                    joint,
                    pair,
                    body: member.body,
                }),
            },
        }
    }
}

/// What lets two items share room, read off the design: what the overlap
/// check at rest leaves out, and what joins or cuts them.
struct Explained {
    /// Hardware whose kind's rules failed (already an error).
    misfit: Vec<HardwareId>,
    /// Parts spaced past their `to` (already an error).
    overrun: Vec<PartId>,
    /// The parts of each joint pair, made parts included, by owner.
    pairs: Vec<(BodyOwner, Vec<PartId>)>,
    /// Which owner cuts which part.
    cuts: Vec<(BodyOwner, PartId)>,
    /// Parts a board joint makes (a spline, a key), with the part made of
    /// boards it sits in: the board joint cuts the room for it.
    board_parts: Vec<(PartId, PartId)>,
}

impl Explained {
    fn of(design: &SolvedDesign) -> Self {
        let pairs = design
            .joints
            .iter()
            .flat_map(|joint| {
                joint.pairs.iter().enumerate().map(|(index, pair)| {
                    let parts = pair
                        .roles
                        .iter()
                        .map(|role| role.part)
                        .chain(pair.made_parts.iter().copied())
                        .collect();
                    (
                        BodyOwner::Joint {
                            joint: joint.id,
                            pair: index,
                        },
                        parts,
                    )
                })
            })
            .collect();
        let cuts = design
            .parts
            .iter()
            .flat_map(|part| {
                part.cuts.iter().filter_map(|cut| {
                    let owner = match cut.source {
                        CutSource::Joint { joint, pair } => BodyOwner::Joint { joint, pair },
                        CutSource::Hardware { hardware } => BodyOwner::Hardware { hardware },
                        CutSource::Boards { .. } => return None,
                    };
                    Some((owner, part.id))
                })
            })
            .collect();
        Explained {
            misfit: design
                .hardware
                .iter()
                .filter(|hardware| match hardware.rules {
                    Rules::Hold => false,
                    Rules::Broken => true,
                })
                .map(|hardware| hardware.id)
                .collect(),
            overrun: design.overruns.iter().map(|overrun| overrun.part).collect(),
            pairs,
            cuts,
            board_parts: design
                .parts
                .iter()
                // Only a board joint's parts sit in another part without
                // being in a joint pair with it.
                .filter_map(|part| {
                    let PartOrigin::Boards { part: boards, .. } = part.origin else {
                        return None;
                    };
                    Some((part.id, boards))
                })
                .collect(),
        }
    }

    fn covers(&self, a: Thing, b: Thing) -> bool {
        match (a, b) {
            (Thing::Part(x), Thing::Part(y)) => {
                self.overrun.contains(&x)
                    || self.overrun.contains(&y)
                    || self.board_parts.contains(&(x, y))
                    || self.board_parts.contains(&(y, x))
                    || self
                        .pairs
                        .iter()
                        .any(|(_, parts)| parts.contains(&x) && parts.contains(&y))
            }
            (Thing::Member(x), Thing::Member(y)) => {
                x.owner == y.owner || self.misfit_member(x) || self.misfit_member(y)
            }
            (Thing::Part(part), Thing::Member(member))
            | (Thing::Member(member), Thing::Part(part)) => {
                self.misfit_member(member)
                    || self.cuts.contains(&(member.owner, part))
                    || self
                        .pairs
                        .iter()
                        .any(|(owner, parts)| *owner == member.owner && parts.contains(&part))
            }
        }
    }

    fn misfit_member(&self, member: BodyRef) -> bool {
        member
            .hardware()
            .is_some_and(|hardware| self.misfit.contains(&hardware))
    }
}

/// The box two boxes share, when they share one.
fn intersection(a: &WorldBox, b: &WorldBox) -> Option<WorldBox> {
    use stave_lang::placement::Axis;
    let mut low = a.min();
    let mut high = a.max();
    for axis in Axis::ALL {
        let from = a.min().get(axis).max(b.min().get(axis));
        let to = a.max().get(axis).min(b.max().get(axis));
        if to <= from {
            return None;
        }
        low = low.with(axis, from);
        high = high.with(axis, to);
    }
    Some(WorldBox::new(low, high))
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::path::Path;

    use super::*;

    /// Asks to stop at the `limit`th time it is read, and from then on.
    struct After {
        reads: Cell<usize>,
        limit: usize,
    }

    impl Stop for After {
        type Stopped = ();

        fn stopped(&self) -> bool {
            self.reads.set(self.reads.get() + 1);
            self.reads.get() > self.limit
        }

        fn check(&self) -> Result<(), ()> {
            if self.stopped() { Err(()) } else { Ok(()) }
        }
    }

    #[test]
    fn stopping_at_any_point_gives_nothing_and_not_stopping_gives_the_single_check() {
        let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/nightstand");
        let modules = stave_modules::ModuleSet::load(&folder).unwrap();
        let (defs, _) = stave_model::resolve(&modules);
        let (model, _) = stave_model::instance::instantiate(defs, modules.root);
        let (design, _) = stave_solve::solve(&model);
        let design = design.unwrap();
        let single = crate::check_with_contacts(&model, &design);
        let mut limit = 0;
        loop {
            let stop = After {
                reads: Cell::new(0),
                limit,
            };
            match crate::run(&model, &design, &stop) {
                Err(()) => limit += 1,
                Ok(found) => {
                    assert_eq!(found, single);
                    break;
                }
            }
            assert!(limit < 10_000, "the checks finish");
        }
        // Read between every check, and in the sweeps.
        assert!(limit > 15, "{limit} points to stop at");
    }
}
