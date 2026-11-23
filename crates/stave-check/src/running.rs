//! Running clearance (layer 5; grammar, "Running clearance"): wood that
//! moves past wood needs a gap greater than the seasons close up, or it
//! binds (`error[motion]`), and a running gap beyond that, or it is tight
//! (`warning[clearance]`).
//!
//! Every sweep the motion check makes ([`Checker::sweeps`]: each drive
//! from rest, from each scene's poses and at each set value of the other
//! drives) measures every wooden body the motion moves against every
//! wooden body that stays put, with [`stave_geom::running::least_gap`],
//! which leaves out seating. Bodies that move together are never
//! measured. Each pair is reported once for each drive, with the least
//! gap any of its sweeps finds; a drive whose sweep hits something is not
//! measured at all, since the hit is the mistake.
//!
//! The swell along the line of the closest approach is worked out exactly
//! from each board's size along it and its stock's `max_held_width`, and
//! rounded once, to the nanometre.
//!
//! **A spacer is a bearing, not a rub.** A body in a clamp's solid stack at
//! rest that touches nothing but its two neighbours in the stack and the
//! clamp's own members ([`Checker::spacers`]) is not measured as rubbing
//! where it touches what moves past it; a spacer that is a part is wood,
//! and wears (`warning[clearance]`, once for each spacer and drive).

use std::collections::{BTreeMap, BTreeSet};

use stave_geom::collide::{CONTACT_TOLERANCE, Placement, WorldSolid, placed_bounds, touches};
use stave_geom::running::{LeastGap, least_gap};
use stave_geom::sweep::{Mover, Path};
use stave_lang::placement::{Axis, PartAxis};
use stave_lang::units::{Length, Rational};
use stave_lang::{Code, Severity};
use stave_model::mechanism::DriveValue;
use stave_model::solved::{AssemblyId, Part, PoseValue, Shape, WorldBox};
use stave_schema::fields::DEFAULT_MAX_HELD_WIDTH_NM;
use stave_schema::species::Species;
use stave_schema::words::{Grain, StockForm};

use crate::motion::{Moving, SweepStart, angle_text};
use crate::names::length_text;
use crate::{Checker, Contact, ContactMotion, Item};

/// One millimetre, in nanometres: what a board as wide as its stock's
/// `max_held_width` moves with the seasons.
const ONE_MM_NM: i128 = 1_000_000;

/// The least gap one moving body comes to one that stays put, in one
/// sweep of one drive, with where the sweep started and the path it ran.
#[derive(Debug, Clone)]
pub(crate) struct Passing {
    /// The body the motion moves.
    pub moving: Item,
    /// The body it moves past.
    pub fixed: Item,
    /// The least gap, where it is, and the line of it.
    pub gap: LeastGap,
    /// Where the sweep started.
    pub start: SweepStart,
    /// The path it ran.
    pub path: Path,
}

/// What one drive's sweeps found, gathered over its starts: the least gap
/// of each pair of bodies, by the pair.
#[derive(Debug, Default)]
pub(crate) struct DriveGaps {
    least: BTreeMap<(Item, Item), Passing>,
}

impl DriveGaps {
    /// `true` when a gap between two bodies is less than any found for
    /// them so far: worth keeping.
    pub(crate) fn improves(&self, moving: Item, fixed: Item, gap: &LeastGap) -> bool {
        self.least
            .get(&(moving, fixed))
            .is_none_or(|earlier| gap.gap < earlier.gap.gap)
    }

    /// Keeps a sweep's gap where it is the least found for its pair (the
    /// first found, of equals).
    pub(crate) fn add(&mut self, found: Passing) {
        if self.improves(found.moving, found.fixed, &found.gap) {
            self.least.insert((found.moving, found.fixed), found);
        }
    }
}

/// How far a solid board closes up with the seasons along one of its own
/// world axes at rest: half its size along it, over its stock's
/// `max_held_width`, times 1mm; nothing along its grain, nor for a sheet,
/// a turned or round part, or anything else.
fn board_swell(part: &Part, axis: Axis) -> Rational {
    if part.stock.form != StockForm::Solid || part.shape != Shape::Board {
        return Rational::ZERO;
    }
    let grain = match part.grain {
        Grain::Length => PartAxis::Length,
        Grain::Width => PartAxis::Width,
    };
    let Some(own) = PartAxis::ALL
        .into_iter()
        .find(|own| part.pose.orientation.direction(*own).axis() == axis)
    else {
        return Rational::ZERO;
    };
    if own == grain {
        return Rational::ZERO;
    }
    let limit = part
        .stock
        .max_held_width
        .unwrap_or(Length::from_nm(DEFAULT_MAX_HELD_WIDTH_NM));
    let size = i128::from(part.drawn.get(own).nm());
    let over = i128::from(limit.nm()).saturating_mul(2);
    Rational::new(size.saturating_mul(ONE_MM_NM), over).unwrap_or(Rational::ZERO)
}

/// A rational number of nanometres, rounded once, as a length.
fn rounded(value: Rational) -> Length {
    Length::from_nm(i64::try_from(value.round()).unwrap_or(i64::MAX))
}

/// A rational number of nanometres, rounded up, as a length.
fn rounded_up(value: Rational) -> Length {
    let floor = value.floor();
    let up = if Rational::from(i64::try_from(floor).unwrap_or(i64::MAX)) == value {
        floor
    } else {
        floor.saturating_add(1)
    };
    Length::from_nm(i64::try_from(up).unwrap_or(i64::MAX))
}

/// How far two bodies close up with the seasons along the line between
/// them, each along its own axis nearest it, rounded once, with the
/// species of the boards that swell (each once, in order).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Swell {
    amount: Length,
    woods: Vec<Species>,
}

impl Checker<'_> {
    /// `true` for a wooden body: a part, or a joint's member whose
    /// fastener kind is made of wood. A hardware member is never wood.
    fn wooden(&self, item: Item) -> bool {
        match item {
            Item::Part(_) => true,
            Item::Member(member) => {
                member.hardware().is_none()
                    && self
                        .member(member)
                        .is_some_and(|body| body.item.as_ref().is_some_and(|i| i.wood.is_some()))
            }
        }
    }

    /// How far a body closes up along one of its own world axes at rest:
    /// a part's board swell, nothing for a joint's member.
    fn item_swell(&self, item: Item, axis: Axis) -> Rational {
        match item {
            Item::Part(part) => self
                .design
                .part(part)
                .map(|found| board_swell(found, axis))
                .unwrap_or(Rational::ZERO),
            Item::Member(_) => Rational::ZERO,
        }
    }

    /// The most a body can close up along any line: what the running
    /// clearance must look within.
    fn most_swell(&self, item: Item) -> Rational {
        Axis::ALL
            .into_iter()
            .map(|axis| self.item_swell(item, axis))
            .max()
            .unwrap_or(Rational::ZERO)
    }

    /// The swell of two bodies along the line of their closest approach.
    fn swell(&self, gap: &LeastGap, moving: Item, fixed: Item) -> Swell {
        let mut total = Rational::ZERO;
        let mut woods = Vec::new();
        for (item, axis) in [(moving, gap.moving_axis), (fixed, gap.fixed_axis)] {
            let amount = self.item_swell(item, axis);
            if amount > Rational::ZERO {
                total = total.checked_add(amount).unwrap_or(total);
                if let Item::Part(part) = item
                    && let Some(found) = self.design.part(part)
                    && !woods.contains(&found.stock.species)
                {
                    woods.push(found.stock.species);
                }
            }
        }
        Swell {
            amount: rounded(total),
            woods,
        }
    }

    /// The spacers (grammar, "Running clearance"): each body in a clamp's
    /// solid stack at rest that touches nothing but its two neighbours in
    /// the stack and the clamp's own members.
    pub(crate) fn spacers(&self) -> BTreeSet<Item> {
        let mut spacers = BTreeSet::new();
        for hardware in self.design.hardware() {
            if hardware.clamp.is_none() || self.misfit.contains(&hardware.id) {
                continue;
            }
            let Some(stack) = self.clamp_stack(hardware) else {
                continue;
            };
            let own = |item: Item| item.hardware() == Some(hardware.id);
            for entry in stack {
                let Some(body) = self.bodies.iter().find(|b| b.item == entry.item) else {
                    continue;
                };
                let alone = self.bodies.iter().all(|other| {
                    other.item == entry.item
                        || entry.neighbours.contains(&other.item)
                        || own(other.item)
                        || !near(&body.bounds, &other.bounds, CONTACT_TOLERANCE)
                        || !touches(&body.solid, &other.solid)
                });
                if alone {
                    spacers.insert(entry.item);
                }
            }
        }
        spacers
    }

    /// The least gap between every wooden body a sweep moves and every
    /// wooden body that stays put (at `placements`, seen from where the
    /// moving assemblies it is inside hold it), where it is within what
    /// matters: the most the two can swell, and the running gap.
    pub(crate) fn passing(
        &self,
        assembly: AssemblyId,
        path: &Path,
        moving: &[Moving<'_>],
        placements: &[Placement],
    ) -> Vec<(Item, Item, LeastGap)> {
        let running = self.design.project.running_gap;
        let mut found = Vec::new();
        for moved in moving {
            let Some(body) = self.bodies.get(moved.index) else {
                continue;
            };
            if !self.wooden(body.item) {
                continue;
            }
            let mover = self.moving_mover(moved, assembly, path);
            let Some(reach) = path.reach(&moved.solid, &mover) else {
                continue;
            };
            let own_swell = self.most_swell(body.item);
            for (j, fixed) in self.bodies.iter().enumerate() {
                if fixed.chain.contains(&assembly)
                    || fixed.loose
                    || !self.wooden(fixed.item)
                    || self.failed.contains(&fixed.item)
                    || self.posed_failed.contains(&fixed.item)
                    || self.exempt(body.item, fixed.item)
                {
                    continue;
                }
                let Some(placement) = placements.get(j).copied() else {
                    continue;
                };
                let most = own_swell
                    .checked_add(self.most_swell(fixed.item))
                    .unwrap_or(own_swell);
                let within = Length::from_nm(
                    rounded_up(most)
                        .nm()
                        .saturating_add(running.nm())
                        .saturating_add(CONTACT_TOLERANCE.nm())
                        .saturating_add(1),
                );
                if !near(&reach, &placed_bounds(&fixed.bounds, &placement), within) {
                    continue;
                }
                if let Some(gap) = least_gap(&moved.solid, mover, &fixed.solid, placement, within) {
                    found.push((body.item, fixed.item, gap));
                }
            }
        }
        found
    }

    /// `true` when a body moved by `mover` passes one that stays at
    /// `placement` with the gap running clearance asks for (or either is
    /// not wood): what a gap the help suggests must leave.
    pub(crate) fn runs_clear(
        &self,
        (moving, moving_solid, mover): (Item, &WorldSolid, Mover<'_>),
        (fixed, fixed_solid, placement): (Item, &WorldSolid, Placement),
    ) -> bool {
        if !self.wooden(moving) || !self.wooden(fixed) {
            return true;
        }
        let running = self.design.project.running_gap;
        let most = self
            .most_swell(moving)
            .checked_add(self.most_swell(fixed))
            .unwrap_or(Rational::ZERO);
        let within = Length::from_nm(
            rounded_up(most)
                .nm()
                .saturating_add(running.nm())
                .saturating_add(CONTACT_TOLERANCE.nm())
                .saturating_add(1),
        );
        match least_gap(moving_solid, mover, fixed_solid, placement, within) {
            None => true,
            Some(gap) => {
                let swell = self.swell(&gap, moving, fixed);
                gap.gap > CONTACT_TOLERANCE
                    && gap.gap.nm() >= swell.amount.nm().saturating_add(running.nm())
            }
        }
    }

    /// Reports what one drive's sweeps found: each pair too close for the
    /// seasons and the running gap, and each wooden spacer something turns
    /// or slides on, once.
    pub(crate) fn report_running(
        &mut self,
        assembly: AssemblyId,
        gaps: DriveGaps,
        spacers: &BTreeSet<Item>,
    ) {
        let running = self.design.project.running_gap;
        let tolerance = CONTACT_TOLERANCE;
        let mut worn: BTreeSet<Item> = BTreeSet::new();
        for passing in gaps.least.into_values() {
            let gap = passing.gap.gap;
            let spacer = [passing.moving, passing.fixed]
                .into_iter()
                .find(|item| spacers.contains(item));
            if let Some(spacer) = spacer
                && gap <= tolerance
            {
                // A bearing: wood wears, anything else is clean.
                if matches!(spacer, Item::Part(_)) && worn.insert(spacer) {
                    self.report_wear(assembly, &passing, spacer);
                }
                continue;
            }
            let swell = self.swell(&passing.gap, passing.moving, passing.fixed);
            let needed = Length::from_nm(swell.amount.nm().saturating_add(running.nm()));
            if gap <= swell.amount || gap <= tolerance {
                self.report_binds(assembly, &passing, &swell, needed);
            } else if gap < needed {
                self.report_tight(assembly, &passing, &swell, needed);
            }
        }
    }

    /// The words for where along a sweep a gap is least and the motion:
    /// `the whole way as frame.tilt turns`, `at 35.2deg as frame.tilt
    /// turns`, `at 120mm as drawer slides`, or `as drawer starts to slide`
    /// where it is least as the motion leaves closed.
    fn where_least(gap: &LeastGap, name: &str, path: &Path) -> String {
        let (verb, start) = match path.mechanism().extent() {
            DriveValue::Turn(_) => ("turns", "turn"),
            DriveValue::Travel(_) => ("slides", "slide"),
        };
        if gap.whole_way {
            return format!("the whole way as {name} {verb}");
        }
        // Where it rounds to nothing, it is where the motion starts.
        let tenth_of_a_degree = 100_000;
        let micrometre = 1_000;
        match gap.at {
            PoseValue::Angle(angle) if angle.microdeg() >= tenth_of_a_degree => {
                format!("at {} as {name} {verb}", angle_text(angle))
            }
            PoseValue::Travel(length) if length.nm() >= micrometre => {
                format!("at {} as {name} {verb}", length_text(length))
            }
            PoseValue::Angle(_) | PoseValue::Travel(_) | PoseValue::Closed | PoseValue::Open => {
                format!("as {name} starts to {start}")
            }
        }
    }

    /// The verb for a drive: a turn `turns`, a travel `slides`.
    fn moves_word(path: &Path) -> &'static str {
        match path.mechanism().extent() {
            DriveValue::Turn(_) => "turns",
            DriveValue::Travel(_) => "slides",
        }
    }

    /// What the seasons and the running gap add up to, in brackets:
    /// `0.563mm for the seasons and a running gap of 1mm`.
    fn needed_words(&self, swell: &Swell) -> String {
        let running = length_text(self.design.project.running_gap);
        if swell.amount > Length::ZERO {
            format!(
                "{} for the seasons and a running gap of {running}",
                length_text(swell.amount)
            )
        } else {
            format!("a running gap of {running}")
        }
    }

    /// The contact of a running clearance diagnostic: the two items, where
    /// along the motion, and the scene.
    fn running_contact(&self, assembly: AssemblyId, passing: &Passing) -> Contact {
        let mut contact = Contact::of(self.design, &[passing.moving, passing.fixed], &[]);
        contact.scene = (passing.start.scene_index > 0)
            .then(|| {
                self.design
                    .scenes
                    .get(passing.start.scene_index)
                    .map(|s| s.id)
            })
            .flatten();
        contact.motion = Some(ContactMotion {
            assembly,
            drive: passing.path.mechanism().swept(),
            at: passing.gap.at,
            shown: passing.gap.at,
        });
        contact
    }

    /// `error[motion]`: wood that binds as it moves past wood.
    fn report_binds(
        &mut self,
        assembly: AssemblyId,
        passing: &Passing,
        swell: &Swell,
        needed: Length,
    ) {
        let name = self.motion_name(assembly, &passing.path);
        let (a, b) = (
            self.item_name(passing.moving),
            self.item_name(passing.fixed),
        );
        let gap = length_text(passing.gap.gap);
        let seasons = if swell.amount > Length::ZERO {
            let woods: Vec<String> = swell
                .woods
                .iter()
                .map(|wood| format!("the {}", wood.word()))
                .collect();
            let verb = if woods.len() > 1 { "close" } else { "closes" };
            format!(
                ", and {} {verb} up to {} with the seasons",
                crate::names::and_list(&woods),
                length_text(swell.amount)
            )
        } else {
            String::new()
        };
        let message = format!(
            "{a} rubs {b} {}: the gap is {gap}{seasons}",
            Self::where_least(&passing.gap, &name, &passing.path),
        );
        // Touching: something has to bear the load, and steel does.
        let bearing = if passing.gap.gap <= CONTACT_TOLERANCE {
            ", and let them bear on steel, such as a washer, rather than on each other"
        } else {
            ""
        };
        let help = format!(
            "leave at least {} between them ({}){bearing}",
            length_text(needed),
            self.needed_words(swell)
        );
        let notes = self.sweep_notes(assembly, &passing.start, &passing.path);
        let place = self.sweep_place(assembly);
        self.next_contact = Some(self.running_contact(assembly, passing));
        self.report(
            Severity::Error,
            Code::Motion,
            message,
            place,
            notes,
            Some(help),
        );
    }

    /// `warning[clearance]`: wood that passes wood with less than the
    /// running gap to spare.
    fn report_tight(
        &mut self,
        assembly: AssemblyId,
        passing: &Passing,
        swell: &Swell,
        needed: Length,
    ) {
        let name = self.motion_name(assembly, &passing.path);
        let (a, b) = (
            self.item_name(passing.moving),
            self.item_name(passing.fixed),
        );
        let needed = length_text(needed);
        let message = format!(
            "{a} passes {} from {b} {}; leave {needed} ({})",
            length_text(passing.gap.gap),
            Self::where_least(&passing.gap, &name, &passing.path),
            self.needed_words(swell)
        );
        let help = format!(
            "move them at least {needed} apart, or write a smaller running_gap: in the project if this fit is meant to be close"
        );
        let notes = self.sweep_notes(assembly, &passing.start, &passing.path);
        let place = self.sweep_place(assembly);
        self.next_contact = Some(self.running_contact(assembly, passing));
        self.report(
            Severity::Warning,
            Code::Clearance,
            message,
            place,
            notes,
            Some(help),
        );
    }

    /// `warning[clearance]` at a wooden spacer something moves on.
    fn report_wear(&mut self, assembly: AssemblyId, passing: &Passing, spacer: Item) {
        let spacer_name = self.item_name(spacer);
        let verb = Self::moves_word(&passing.path);
        let message = if spacer == passing.moving {
            format!(
                "{spacer_name} is wood: it wears as it {verb} on {}; a steel or nylon washer lasts longer",
                self.item_name(passing.fixed)
            )
        } else {
            format!(
                "{spacer_name} is wood: it wears as {} {verb} on it; a steel or nylon washer lasts longer",
                self.item_name(passing.moving)
            )
        };
        let help = format!("make {spacer_name} a washer member of its hardware instead of a part");
        let notes = self.sweep_notes(assembly, &passing.start, &passing.path);
        let place = match spacer {
            Item::Part(part) => self.part_place(part),
            Item::Member(_) => None,
        }
        .or_else(|| self.sweep_place(assembly));
        self.next_contact = Some(self.running_contact(assembly, passing));
        self.report(
            Severity::Warning,
            Code::Clearance,
            message,
            place,
            notes,
            Some(help),
        );
    }
}

/// `true` when two boxes come within `within` of each other.
fn near(a: &WorldBox, b: &WorldBox, within: Length) -> bool {
    Axis::ALL.into_iter().all(|axis| {
        let low = a.min().get(axis).max(b.min().get(axis)).nm();
        let high = a.max().get(axis).min(b.max().get(axis)).nm();
        high.saturating_add(within.nm()) >= low
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounding_up_keeps_whole_values() {
        assert_eq!(rounded_up(Rational::from(5_i64)), Length::from_nm(5));
        let half = Rational::new(11, 2).expect("a ratio");
        assert_eq!(rounded_up(half), Length::from_nm(6));
    }

    #[test]
    fn boxes_within_a_distance_are_near() {
        let point = |x: i64| stave_model::solved::Point {
            across: Length::from_mm(x),
            up: Length::ZERO,
            deep: Length::ZERO,
        };
        let block = |from: i64, to: i64| {
            WorldBox::new(
                point(from),
                stave_model::solved::Point {
                    across: Length::from_mm(to),
                    up: Length::from_mm(10),
                    deep: Length::from_mm(10),
                },
            )
        };
        assert!(near(&block(0, 10), &block(12, 20), Length::from_mm(3)));
        assert!(!near(&block(0, 10), &block(14, 20), Length::from_mm(3)));
    }
}
