//! A hardware kind's mechanism with its points evaluated exactly, for one
//! piece of hardware (grammar, "Mechanisms").
//!
//! A [`Mechanism`] can only be built when it has exactly one freedom and
//! an order in which its members can be placed, one step at a time, from
//! the members that are held and from its drive. Version 1 knows one step:
//! a driven member hung on two links from a held member (a cup hinge's
//! four-bar linkage). Its dead points are checked exactly, in whole
//! nanometres, by [`stave_model::solved::Linkage`].

use std::collections::BTreeSet;

use stave_lang::units::{ExactAngle, ExactLength};
use stave_model::defs::{BodyId, Carrier, FrameAxis, NonEmpty};
use stave_model::solved::{Link, Linkage, LinkageError, LinkageFrame, PlanePoint};

/// A point in the kind's frame, exact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ExactPoint {
    /// Along the piece's length.
    pub along: ExactLength,
    /// Across the mount face, toward the middle of the piece.
    pub across: ExactLength,
    /// From the mount face into the piece.
    pub depth: ExactLength,
}

/// A pivot, slot or rail between two members, exact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Constraint {
    /// Two members turn on one pin.
    Pivot {
        /// The two members.
        bodies: [BodyId; 2],
        /// The pin.
        at: ExactPoint,
    },
    /// A pin on the first member runs in a straight slot in the second.
    Slot {
        /// The member with the pin, then the member with the slot.
        bodies: [BodyId; 2],
        /// One end of the slot.
        from: ExactPoint,
        /// The other end.
        to: ExactPoint,
    },
    /// The first member runs straight along the second.
    Rail {
        /// The member that runs, then the member it runs along.
        bodies: [BodyId; 2],
        /// The direction.
        along: FrameAxis,
    },
}

impl Constraint {
    fn bodies(&self) -> [BodyId; 2] {
        match self {
            Constraint::Pivot { bodies, .. }
            | Constraint::Slot { bodies, .. }
            | Constraint::Rail { bodies, .. } => *bodies,
        }
    }

    /// The freedoms it takes away: two for a pivot or a rail, one for a
    /// slot.
    fn takes(&self) -> i64 {
        match self {
            Constraint::Pivot { .. } | Constraint::Rail { .. } => 2,
            Constraint::Slot { .. } => 1,
        }
    }
}

/// The one input of a mechanism, exact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Drive {
    /// The member turns from one angle to another.
    Turn {
        /// The driven member.
        body: BodyId,
        /// The closed pose's angle.
        from: ExactAngle,
        /// The widest angle.
        to: ExactAngle,
    },
    /// The member travels from one length to another.
    Travel {
        /// The driven member.
        body: BodyId,
        /// The closed pose's travel.
        from: ExactLength,
        /// The furthest travel.
        to: ExactLength,
    },
}

impl Drive {
    fn body(&self) -> BodyId {
        match self {
            Drive::Turn { body, .. } | Drive::Travel { body, .. } => *body,
        }
    }
}

/// One link of a four-bar: pinned once to a held member and once to the
/// driven member.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HungLink {
    /// The link.
    pub link: BodyId,
    /// The held member it is pinned to.
    pub held: BodyId,
    /// Its pin in the held member.
    pub held_pin: ExactPoint,
    /// Its pin in the driven member.
    pub driven_pin: ExactPoint,
}

/// A step that places members, in the order the solver takes them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    /// A driven member whose angle is known, hung on two links from held
    /// members: its position is where two circles cross.
    HungOnTwoLinks {
        /// The driven member.
        driven: BodyId,
        /// The two links, in the order the kind declares them.
        links: [HungLink; 2],
    },
}

/// Why a mechanism can't be built.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum MechanismError {
    /// The members don't add up to exactly one freedom.
    #[error("the mechanism has {found} freedoms; it needs exactly one")]
    Freedom {
        /// How many it has (negative when it is over-constrained).
        found: i64,
    },
    /// Some members can't be reached one step at a time from the drive.
    #[error("the mechanism can't be solved one step at a time from its drive")]
    NoOrder {
        /// The members left over, in order.
        left: Vec<BodyId>,
    },
    /// It has one freedom and an order, but not in a form version 1
    /// solves.
    #[error("this mechanism is not supported yet")]
    NotSupported,
    /// The links lock (or come apart) before the drive's end.
    #[error("the links lock before the drive's end")]
    DeadPoint,
    /// A size too big for the arithmetic (never a design's).
    #[error("the mechanism is too large to check")]
    Overflow,
}

/// A mechanism with one freedom and a known order of steps.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Mechanism {
    drive: Drive,
    /// The steps that place its members, in order; version 1 has one.
    steps: NonEmpty<Step>,
}

impl Mechanism {
    /// Checks the mechanism: exactly one freedom, an order of steps from
    /// its drive, and (in version 1) the form a driven member hung on two
    /// links from a held member. `carriers` is indexed by [`BodyId`];
    /// `couplings` is how many `couple` entries it has.
    pub fn new(
        carriers: &[Carrier],
        constraints: Vec<Constraint>,
        couplings: usize,
        drive: Drive,
    ) -> Result<Mechanism, MechanismError> {
        let count = |n: usize| i64::try_from(n).map_err(|_| MechanismError::Overflow);
        let driven = drive.body();
        let classes = classify(carriers, driven);
        let moving = classes
            .iter()
            .filter(|class| matches!(class, Class::Driven | Class::Free))
            .count();
        let taken = constraints
            .iter()
            .try_fold(0i64, |sum, constraint| sum.checked_add(constraint.takes()))
            .ok_or(MechanismError::Overflow)?;
        let found = count(moving)?
            .checked_mul(3)
            .and_then(|freedoms| freedoms.checked_sub(taken))
            .and_then(|freedoms| freedoms.checked_sub(count(couplings).ok()?))
            .ok_or(MechanismError::Overflow)?;
        if found != 1 {
            return Err(MechanismError::Freedom { found });
        }
        let left = unreachable(&classes, &constraints);
        if !left.is_empty() {
            return Err(MechanismError::NoOrder { left });
        }
        let step = four_bar(&classes, &constraints, couplings, &drive)?;
        Ok(Mechanism {
            drive,
            steps: NonEmpty::new(vec![step]).ok_or(MechanismError::NotSupported)?,
        })
    }

    /// The first step: the one that places the driven member.
    pub fn step(&self) -> Step {
        *self.steps.first()
    }

    /// The drive.
    pub fn drive(&self) -> Drive {
        self.drive
    }

    /// The four-bar linkage of a cup hinge, placed by `place` (from the
    /// kind's frame to the linkage's plane) and checked exactly free of
    /// dead points over the drive's range.
    pub fn linkage(
        &self,
        frame: LinkageFrame,
        place: impl Fn(ExactPoint) -> Option<PlanePoint>,
    ) -> Result<Linkage, MechanismError> {
        let Step::HungOnTwoLinks { links, .. } = self.step();
        let Drive::Turn { to, .. } = self.drive else {
            return Err(MechanismError::NotSupported);
        };
        let link = |hung: HungLink| -> Result<Link, MechanismError> {
            Ok(Link {
                ground: place(hung.held_pin).ok_or(MechanismError::Overflow)?,
                coupler: place(hung.driven_pin).ok_or(MechanismError::Overflow)?,
            })
        };
        let [first, second] = links;
        let opening = to.to_nearest_angle().ok_or(MechanismError::Overflow)?;
        match Linkage::new(frame, [link(first)?, link(second)?], opening) {
            Ok(linkage) => Ok(linkage),
            Err(LinkageError::DeadPoint(_) | LinkageError::Angle) => Err(MechanismError::DeadPoint),
            Err(
                LinkageError::Frame
                | LinkageError::NoLength(_)
                | LinkageError::SharedPin
                | LinkageError::Overflow,
            ) => Err(MechanismError::NotSupported),
        }
    }
}

/// How a member moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    /// The drive moves it.
    Driven,
    /// Carried by the driven member's part, or riding on the driven
    /// member: rigid with it.
    Rider,
    /// Carried by another part (or riding on such a member): it doesn't
    /// move in the mechanism.
    Held,
    /// No `with:`: the mechanism places it.
    Free,
}

/// Every member's class, indexed by [`BodyId`].
fn classify(carriers: &[Carrier], driven: BodyId) -> Vec<Class> {
    let driven_role = match carriers.get(driven.index()) {
        Some(Carrier::Role(role)) => Some(*role),
        Some(Carrier::Body(_) | Carrier::Mechanism) | None => None,
    };
    (0..carriers.len())
        .map(|index| {
            let mut at = index;
            // Follow `with:` from member to member; a chain longer than the
            // list is a loop, which never reaches a part.
            for _ in 0..=carriers.len() {
                if at == driven.index() {
                    return if at == index {
                        Class::Driven
                    } else {
                        Class::Rider
                    };
                }
                match carriers.get(at) {
                    Some(Carrier::Role(role)) => {
                        return if Some(*role) == driven_role {
                            Class::Rider
                        } else {
                            Class::Held
                        };
                    }
                    Some(Carrier::Body(other)) => at = other.index(),
                    Some(Carrier::Mechanism) => {
                        return if at == index {
                            Class::Free
                        } else {
                            Class::Rider
                        };
                    }
                    None => return Class::Held,
                }
            }
            Class::Held
        })
        .collect()
}

/// The free members no chain of constraints reaches from the members
/// already placed (held ones, riders and the driven one).
fn unreachable(classes: &[Class], constraints: &[Constraint]) -> Vec<BodyId> {
    let mut known: BTreeSet<usize> = classes
        .iter()
        .enumerate()
        .filter(|(_, class)| !matches!(class, Class::Free))
        .map(|(index, _)| index)
        .collect();
    loop {
        let before = known.len();
        for constraint in constraints {
            let [a, b] = constraint.bodies();
            if known.contains(&a.index()) || known.contains(&b.index()) {
                known.insert(a.index());
                known.insert(b.index());
            }
        }
        if known.len() == before {
            break;
        }
    }
    (0..classes.len())
        .filter(|index| !known.contains(index))
        .filter_map(BodyId::new)
        .collect()
}

/// The one form version 1 solves: the drive turns a member from `0deg`;
/// exactly two free members, each pinned once to the driven member and
/// once to a held member; nothing else.
fn four_bar(
    classes: &[Class],
    constraints: &[Constraint],
    couplings: usize,
    drive: &Drive,
) -> Result<Step, MechanismError> {
    let Drive::Turn {
        body: driven, from, ..
    } = *drive
    else {
        return Err(MechanismError::NotSupported);
    };
    if from != ExactAngle::ZERO || couplings != 0 {
        return Err(MechanismError::NotSupported);
    }
    let class = |body: BodyId| classes.get(body.index()).copied();
    let free: Vec<BodyId> = (0..classes.len())
        .filter_map(BodyId::new)
        .filter(|body| class(*body) == Some(Class::Free))
        .collect();
    let [first, second] = free.as_slice() else {
        return Err(MechanismError::NotSupported);
    };
    let hung = |link: BodyId| -> Result<HungLink, MechanismError> {
        let mut held: Option<(BodyId, ExactPoint)> = None;
        let mut on_driven: Option<ExactPoint> = None;
        for constraint in constraints {
            let Constraint::Pivot { bodies, at } = constraint else {
                return Err(MechanismError::NotSupported);
            };
            let other = match bodies {
                [a, b] if *a == link => *b,
                [a, b] if *b == link => *a,
                _ => continue,
            };
            match class(other) {
                Some(Class::Driven) if other == driven && on_driven.is_none() => {
                    on_driven = Some(*at);
                }
                Some(Class::Held) if held.is_none() => held = Some((other, *at)),
                _ => return Err(MechanismError::NotSupported),
            }
        }
        match (held, on_driven) {
            (Some((held, held_pin)), Some(driven_pin)) => Ok(HungLink {
                link,
                held,
                held_pin,
                driven_pin,
            }),
            _ => Err(MechanismError::NotSupported),
        }
    };
    let links = [hung(*first)?, hung(*second)?];
    // Every constraint is one of the four pins found above.
    if constraints.len() != 4 {
        return Err(MechanismError::NotSupported);
    }
    Ok(Step::HungOnTwoLinks { driven, links })
}

#[cfg(test)]
mod tests {
    use super::*;
    use stave_lang::units::Length;
    use stave_model::defs::RoleIndex;

    fn id(index: usize) -> BodyId {
        BodyId::new(index).unwrap()
    }

    fn at(across: i64, depth: i64) -> ExactPoint {
        ExactPoint {
            along: ExactLength::ZERO,
            across: Length::from_mm(across).to_exact(),
            depth: Length::from_mm(depth).to_exact(),
        }
    }

    fn pivot(a: usize, b: usize, point: ExactPoint) -> Constraint {
        Constraint::Pivot {
            bodies: [id(a), id(b)],
            at: point,
        }
    }

    /// arm (held by `on`), cup (driven, with `piece`), two links.
    fn carriers() -> Vec<Carrier> {
        vec![
            Carrier::Role(RoleIndex::new(1).unwrap()),
            Carrier::Role(RoleIndex::new(0).unwrap()),
            Carrier::Mechanism,
            Carrier::Mechanism,
        ]
    }

    fn turn() -> Drive {
        Drive::Turn {
            body: id(1),
            from: ExactAngle::ZERO,
            to: stave_lang::units::Angle::from_deg(90).to_exact(),
        }
    }

    fn four_pins() -> Vec<Constraint> {
        vec![
            pivot(0, 2, at(13, 11)),
            pivot(1, 2, at(-9, 7)),
            pivot(0, 3, at(13, 4)),
            pivot(1, 3, at(-10, 1)),
        ]
    }

    #[test]
    fn a_cup_hinge_is_a_driven_member_hung_on_two_links() {
        let mechanism = Mechanism::new(&carriers(), four_pins(), 0, turn()).unwrap();
        let Step::HungOnTwoLinks { driven, links } = mechanism.step();
        assert_eq!(driven, id(1));
        assert_eq!(links.map(|link| link.link), [id(2), id(3)]);
        assert_eq!(links.map(|link| link.held), [id(0), id(0)]);
        assert_eq!(links[0].held_pin, at(13, 11));
        assert_eq!(links[1].driven_pin, at(-10, 1));
    }

    #[test]
    fn one_pin_too_few_leaves_three_freedoms() {
        let mut pins = four_pins();
        pins.pop();
        // 3 * 3 - 2 * 3
        assert_eq!(
            Mechanism::new(&carriers(), pins, 0, turn()),
            Err(MechanismError::Freedom { found: 3 })
        );
    }

    #[test]
    fn a_slot_in_place_of_a_pin_is_counted_as_one() {
        let mut pins = four_pins();
        pins.pop();
        pins.push(Constraint::Slot {
            bodies: [id(1), id(3)],
            from: at(-10, 1),
            to: at(-10, 5),
        });
        // 9 - 2 - 2 - 2 - 1: two freedoms.
        assert_eq!(
            Mechanism::new(&carriers(), pins, 0, turn()),
            Err(MechanismError::Freedom { found: 2 })
        );
    }

    #[test]
    fn a_member_nothing_reaches_has_no_order() {
        // Two more free members pinned only to each other: nothing placed
        // reaches them.
        let mut carriers = carriers();
        carriers.push(Carrier::Mechanism);
        carriers.push(Carrier::Mechanism);
        let mut pins = four_pins();
        pins.push(pivot(4, 5, at(1, 1)));
        // 3 * 5 - 2 * 5 = 5; four couplings bring it to 1.
        assert_eq!(
            Mechanism::new(&carriers, pins, 4, turn()),
            Err(MechanismError::NoOrder {
                left: vec![id(4), id(5)]
            })
        );
    }

    #[test]
    fn a_link_pinned_twice_to_the_cup_is_not_supported() {
        let mut pins = four_pins();
        pins[0] = pivot(1, 2, at(13, 11));
        assert_eq!(
            Mechanism::new(&carriers(), pins, 0, turn()),
            Err(MechanismError::NotSupported)
        );
    }
}
