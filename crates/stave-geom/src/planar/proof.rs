//! Proving that no step comes within [`DEAD_POINT_MARGIN`] of a dead point
//! anywhere between two samples, not only at them.
//!
//! # The quantities
//!
//! Every two-answer step has a **margin**, a function of where its inputs
//! (the members placed by earlier steps) are:
//!
//! - two circles, centres `c1`, `c2`, radii `r1`, `r2`, `d = |c2 − c1|`:
//!   `m = min(r1 + r2 − d, d − |r1 − r2|)`;
//! - a circle (centre `c`, radius `r`) and a line through `p` with unit
//!   normal `n`: `m = r − |n·(c − p)|`;
//! - a slot in a link, over a pin `reach` from the link's pivot, the slot's
//!   line `dist` from that pivot in the link's own frame: `m = reach −
//!   dist`.
//!
//! A step's two answers exist and are distinct exactly while `m > 0`, and
//! then the answer on the chosen side is a smooth function of the inputs.
//!
//! # Rates
//!
//! Over one interval `I = [a, b]` of the drive (radians or nanometres,
//! `h = b − a`) the proof keeps, for each rigid body `B`, a bound `W_B` on
//! how fast it turns and `U_B` on how fast its origin moves, per unit of
//! drive; `T_B` bounds how fast its travel changes. A point `q` of `B`
//! (closed coordinates) is at `R(θ_B) q + o_B`, so it moves at most
//! `W_B·|q| + U_B`, a constant: no position enters. Two points of the same
//! body move apart at most `W_B·|q1 − q2|`. Held bodies have `W = U = 0`;
//! a turning drive gives its member `W = 1`, a travelling one `T = 1`.
//!
//! Each step turns the rates of its inputs into a bound `L` on how fast its
//! margin can change, and its margin's least value over `I` into rates
//! for what it places:
//!
//! - **Margins.** For two circles `|m'| ≤ |d'| ≤ |c2' − c1'|`. For a circle
//!   and a line `|m'| ≤ |c' − p'| + W_line·|c − p|`, where `W_line` is how
//!   fast the line turns and `|c − p|` is bounded over `I` by its two
//!   sampled values and its own rate, as for a margin (below). For a slot
//!   in a link `|m'| = |reach'| ≤` the speed of the pin relative to the
//!   pivot.
//! - **Two circles, margin at least `μ`.** Differentiating `|J − ci| = ri`
//!   gives `ei·(J' − ci') = 0` with `ei` the unit vectors from the centres
//!   to the answer `J`; solving the 2×2 system, `|(J − ci)'| ≤ |c2' −
//!   c1'| / |e1 × e2|`. `|e1 × e2| = sin φ`, the angle between the links,
//!   and by Heron, `4 r1² r2² sin² φ = (r1 + r2 + d)(r1 + r2 − d)(d + Δ)(d
//!   − Δ)` with `Δ = |r1 − r2|`. With `m ≥ μ`: `d ≥ Δ + μ`, and the two
//!   middle factors are each at least `μ` and sum to `2·min(r1, r2)`, so
//!   `sin φ ≥ √((r1 + r2 + Δ + μ)(2Δ + μ)·μ·(2 min(r1, r2) − μ)) / (2 r1
//!   r2)`.
//! - **A circle and a line, margin at least `μ`.** The rows are the unit
//!   radius `e` and the line's normal `n`, and `|e × n| = half / r` with
//!   `half² = (r − dist)(r + dist) ≥ μ(2r − μ)`. The line's equation
//!   `n·(J − p) = 0` differentiates to `n·J' = n·p' − n'·(J − p)`, and
//!   `|J − p| ≤ r + |c − p|`, so `|(J − c)'| ≤ (|c' − p'| + W_line·(r +
//!   |c − p|)) · r / half`.
//! - **A slot in a link, margin at least `μ`.** `reach ≥ dist + μ` and the
//!   pin's place along the slot is `half = √(reach² − dist²) ≥ √(μ(2 dist +
//!   μ))` from the foot of the pivot; the link turns at most `|reach'| /
//!   reach + |reach'| / half`.
//!
//! Then `J' = ci' + (J − ci)'`; a link of length `r` pinned at `c` turns
//! at `|(J − c)'| / r`, and its origin moves at the speed of its pivot
//! plus that turn times the pivot's distance from the plane's origin.
//!
//! # The argument
//!
//! Steps are taken in their order. Suppose every step before step `k` has
//! been shown to have a margin at least `μ_j ≥ DEAD_POINT_MARGIN > 0` on
//! all of `I`. Then all of their answers exist and move smoothly on `I`,
//! with the rates above (each step's rates use its own `μ_j`, which holds
//! everywhere on `I`). Step `k`'s margin is a function of those inputs
//! alone, defined on all of `I` (it is a plain number even where its paths
//! don't meet), and changes at most `L_k` per unit of drive. From its two
//! samples `m(a)` and `m(b)`, everywhere on `I`
//! `m(s) ≥ max(m(a) − L_k (s − a), m(b) − L_k (b − s)) ≥ (m(a) + m(b) −
//! L_k h) / 2`, so `μ_k = min(m(a), m(b), (m(a) + m(b) − L_k h) / 2)` is a
//! lower bound on `I`. If `μ_k ≥ DEAD_POINT_MARGIN` the induction goes on;
//! otherwise the interval is not proven. The rates never depend on a
//! margin we have not already proven, so the bound that blows up near a
//! dead point (`1 / sin φ`) is only ever taken where the margin is known to
//! stay at least `DEAD_POINT_MARGIN`.
//!
//! When every step is proven, every answer on `I` is the continuation of
//! the one at `a` on the same side: no dead point is reached and no branch
//! can flip in between.
//!
//! The samples are computed in `f64` nanometres; their rounding is far
//! below a nanometre for any mechanism Stave draws, and each sampled
//! margin has [`ROUNDING_NM`] taken off before it is used, which only makes
//! the proof stricter.
//!
//! [`DEAD_POINT_MARGIN`]: super::DEAD_POINT_MARGIN

use glam::DVec2;

use super::Drive;
use super::path::Crossing;
use super::plan::{Body, Dyad, Locus, Mechanism, SlotInLink, Step, Unknown};

/// Taken off every sampled margin before the proof uses it, for `f64`
/// rounding (a nanometre): it only makes the proof stricter.
pub(super) const ROUNDING_NM: f64 = 1.0;

/// What the proof found over one interval.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Outcome {
    /// Every step's margin is at least the floor over the whole interval;
    /// the least of them (`None` when no step has two answers).
    Proven(Option<f64>),
    /// Step `step` could not be shown clear; `at_end` when its sample at
    /// the interval's end is the nearer its dead point.
    Unproven { step: usize, at_end: bool },
}

/// A mechanism's steps read something not placed yet (never for one built
/// by [`Mechanism::new`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Fault;

/// A point that moves: `body`'s point `point`, less `shift` turned with the
/// member whose offset is solved (zero when a joint is solved).
#[derive(Debug, Clone, Copy)]
struct Anchor {
    body: Body,
    point: DVec2,
    shift: DVec2,
}

/// A step's path, for rates.
#[derive(Debug, Clone, Copy)]
enum Track {
    Circle { anchor: Anchor, radius: f64 },
    Line { anchor: Anchor, turns_with: Body },
}

/// Bounds on how fast each body moves, per unit of drive, over the
/// interval.
struct Rates {
    turn: Vec<Option<f64>>,
    offset: Vec<Option<f64>>,
    travel: Vec<Option<f64>>,
}

fn get(values: &[Option<f64>], body: Body) -> Result<f64, Fault> {
    values.get(body.0).copied().flatten().ok_or(Fault)
}

fn put(values: &mut [Option<f64>], body: Body, value: f64) -> Result<(), Fault> {
    *values.get_mut(body.0).ok_or(Fault)? = Some(value);
    Ok(())
}

impl Rates {
    fn turn(&self, body: Body) -> Result<f64, Fault> {
        get(&self.turn, body)
    }

    /// How fast `body`'s point `point` can move.
    fn point(&self, body: Body, point: DVec2) -> Result<f64, Fault> {
        Ok(self.turn(body)? * point.length() + get(&self.offset, body)?)
    }

    /// How fast two points can move apart.
    fn relative(&self, first: (Body, DVec2), second: (Body, DVec2)) -> Result<f64, Fault> {
        if first.0 == second.0 {
            Ok(self.turn(first.0)? * (first.1 - second.1).length())
        } else {
            Ok(self.point(first.0, first.1)? + self.point(second.0, second.1)?)
        }
    }

    /// How fast an anchor can move, the solved member turning at `turning`.
    fn anchor(&self, anchor: Anchor, turning: f64) -> Result<f64, Fault> {
        Ok(self.point(anchor.body, anchor.point)? + turning * anchor.shift.length())
    }

    /// How fast two anchors can move apart.
    fn anchors(&self, first: Anchor, second: Anchor, turning: f64) -> Result<f64, Fault> {
        Ok(
            self.relative((first.body, first.point), (second.body, second.point))?
                + turning * (first.shift - second.shift).length(),
        )
    }

    fn place(&mut self, body: Body, turn: f64, offset: f64) -> Result<(), Fault> {
        put(&mut self.turn, body, turn)?;
        put(&mut self.offset, body, offset)
    }
}

/// The least a margin can be over the interval, from its two samples and
/// how fast it can change (see the module docs).
fn floor(start: f64, end: f64, rate: f64, width: f64) -> f64 {
    let (start, end) = (start - ROUNDING_NM, end - ROUNDING_NM);
    start.min(end).min((start + end - rate * width) / 2.0)
}

/// Whether a floor clears the margin (`false` for a NaN).
fn clears(floor: f64, margin: f64) -> bool {
    floor
        .partial_cmp(&margin)
        .is_some_and(|order| order.is_ge())
}

/// The least `sin φ` between two links of lengths `r1` and `r2` whose
/// margin is at least `floor`.
fn circles_sine(r1: f64, r2: f64, floor: f64) -> f64 {
    let small = r1.min(r2);
    let gap = (r1 - r2).abs();
    let least = floor.min(small);
    let product = (r1 + r2 + gap + least) * (2.0 * gap + least) * least * (2.0 * small - least);
    product.max(0.0).sqrt() / (2.0 * r1 * r2)
}

/// The least `half / r` for a line whose margin inside a circle of radius
/// `r` is at least `floor`.
fn line_cosine(radius: f64, floor: f64) -> f64 {
    let least = floor.min(radius);
    (least * (2.0 * radius - least)).max(0.0).sqrt() / radius
}

fn track(locus: Locus, offset: bool, solved: Option<Body>) -> Result<Track, Fault> {
    let shift = |point: DVec2| if offset { point } else { DVec2::ZERO };
    Ok(match locus {
        Locus::Link {
            known,
            pivot,
            joint,
            radius,
            ..
        } => Track::Circle {
            anchor: Anchor {
                body: known,
                point: pivot,
                shift: shift(joint),
            },
            radius,
        },
        Locus::Rail { guide, joint, .. } => Track::Line {
            anchor: Anchor {
                body: guide,
                point: joint,
                shift: shift(joint),
            },
            turns_with: guide,
        },
        Locus::Slot { guide, from, .. } => Track::Line {
            anchor: Anchor {
                body: guide,
                point: from,
                shift: shift(from),
            },
            turns_with: guide,
        },
        Locus::SlotInUnknown { pin, from, .. } => Track::Line {
            anchor: Anchor {
                body: pin,
                point: from,
                shift: from,
            },
            turns_with: solved.ok_or(Fault)?,
        },
    })
}

/// One step's proof: `Ok(None)` when it is not proven, else its floor.
struct StepProof<'a> {
    rates: &'a mut Rates,
    width: f64,
    margin: f64,
}

impl StepProof<'_> {
    fn dyad(&mut self, dyad: &Dyad, start: Crossing, end: Crossing) -> Result<Option<f64>, Fault> {
        let solved = match dyad.unknown {
            Unknown::Joint => None,
            Unknown::Offset(body) => Some(body),
        };
        let turning = match solved {
            Some(body) => self.rates.turn(body)?,
            None => 0.0,
        };
        let offset = solved.is_some();
        let first = track(dyad.first, offset, solved)?;
        let second = track(dyad.second, offset, solved)?;
        // How fast the answer moves against each circle's centre, and how
        // fast it moves.
        let (floor, against, speed) = match (first, second) {
            (
                Track::Circle {
                    anchor: a1,
                    radius: r1,
                },
                Track::Circle {
                    anchor: a2,
                    radius: r2,
                },
            ) => {
                let apart = self.rates.anchors(a1, a2, turning)?;
                let floor = floor(start.margin, end.margin, apart, self.width);
                if !clears(floor, self.margin) {
                    return Ok(None);
                }
                let against = apart / circles_sine(r1, r2, floor);
                let centre = self
                    .rates
                    .anchor(a1, turning)?
                    .min(self.rates.anchor(a2, turning)?);
                (floor, against, centre + against)
            }
            (
                Track::Circle { anchor, radius },
                Track::Line {
                    anchor: line,
                    turns_with,
                },
            )
            | (
                Track::Line {
                    anchor: line,
                    turns_with,
                },
                Track::Circle { anchor, radius },
            ) => {
                let apart = self.rates.anchors(anchor, line, turning)?;
                let line_turn = self.rates.turn(turns_with)?;
                let spread = (start.spread + end.spread + apart * self.width) / 2.0;
                let rate = apart + line_turn * spread;
                let floor = floor(start.margin, end.margin, rate, self.width);
                if !clears(floor, self.margin) {
                    return Ok(None);
                }
                let against = (apart + line_turn * (radius + spread)) / line_cosine(radius, floor);
                let centre = self.rates.anchor(anchor, turning)?;
                (floor, against, centre + against)
            }
            (Track::Line { .. }, Track::Line { .. }) => return Err(Fault),
        };
        if let Some(body) = solved {
            put(&mut self.rates.offset, body, speed)?;
        }
        for locus in [dyad.first, dyad.second] {
            // How fast the point where this locus's member meets the
            // answer moves.
            let meet = |joint: DVec2| speed + turning * if offset { joint.length() } else { 0.0 };
            match locus {
                Locus::Link {
                    link,
                    known,
                    pivot,
                    radius,
                    ..
                } => {
                    let turn = against / radius;
                    let origin = self.rates.point(known, pivot)? + turn * pivot.length();
                    self.rates.place(link, turn, origin)?;
                }
                Locus::Rail {
                    runner,
                    guide,
                    joint,
                    ..
                } => {
                    let turn = self.rates.turn(guide)?;
                    let origin = meet(joint) + turn * joint.length();
                    self.rates.place(runner, turn, origin)?;
                }
                Locus::Slot { .. } | Locus::SlotInUnknown { .. } => {}
            }
        }
        Ok(Some(floor))
    }

    fn slot_in_link(
        &mut self,
        slot: &SlotInLink,
        start: Crossing,
        end: Crossing,
    ) -> Result<Option<f64>, Fault> {
        let apart = self
            .rates
            .relative((slot.pin, slot.from), (slot.known, slot.pivot))?;
        let floor = floor(start.margin, end.margin, apart, self.width);
        if !clears(floor, self.margin) {
            return Ok(None);
        }
        let direction = (slot.to - slot.from).try_normalize().ok_or(Fault)?;
        let dist = (slot.pivot - slot.from).perp_dot(direction).abs();
        let reach = dist + floor;
        let half = (floor * (2.0 * dist + floor)).max(0.0).sqrt();
        let turn = apart / reach + apart / half;
        let origin = self.rates.point(slot.known, slot.pivot)? + turn * slot.pivot.length();
        self.rates.place(slot.link, turn, origin)?;
        Ok(Some(floor))
    }
}

/// Proves every step of `mechanism` at least `margin` (nanometres) from a
/// dead point over a drive interval `width` wide (radians or nanometres),
/// from the crossings sampled at its two ends. See the module docs.
pub(super) fn prove(
    mechanism: &Mechanism,
    width: f64,
    margin: f64,
    start: &[Option<Crossing>],
    end: &[Option<Crossing>],
) -> Result<Outcome, Fault> {
    let count = mechanism.body_count();
    let held = |b: usize| mechanism.is_held(Body(b)).then_some(0.0);
    let mut rates = Rates {
        turn: (0..count).map(held).collect(),
        offset: (0..count).map(held).collect(),
        travel: vec![None; count],
    };
    let driven = mechanism.drive_body();
    // The held drives don't change over the interval; the swept one
    // changes one unit for each unit of drive.
    for (index, drive) in mechanism.drives().iter().enumerate() {
        let rate = if index == mechanism.swept() { 1.0 } else { 0.0 };
        match drive {
            Drive::Turn { .. } => put(&mut rates.turn, driven, rate)?,
            Drive::Travel { .. } => put(&mut rates.travel, driven, rate)?,
        }
    }
    let mut least: Option<f64> = None;
    for (index, step) in mechanism.steps().iter().enumerate() {
        let sampled = (
            start.get(index).copied().flatten(),
            end.get(index).copied().flatten(),
        );
        let mut proof = StepProof {
            rates: &mut rates,
            width,
            margin,
        };
        let found = match (step, sampled) {
            (Step::Couple { from, to, factor }, _) => {
                let travel = get(&proof.rates.travel, *from)? * factor.abs();
                put(&mut proof.rates.travel, *to, travel)?;
                continue;
            }
            (Step::Hang { body, known, pin }, _) => {
                let turn = proof.rates.turn(*body)?;
                let origin = proof.rates.point(*known, *pin)? + turn * pin.length();
                put(&mut proof.rates.offset, *body, origin)?;
                continue;
            }
            (
                Step::HangInSlot {
                    body,
                    guide,
                    from,
                    to,
                    ..
                },
                _,
            ) => {
                // The pin's point in the guide is within the slot's
                // length of `from`, and runs along it at the travel's rate.
                let turn = proof.rates.turn(*body)?;
                let reach = from.length() + (*to - *from).length();
                let along = get(&proof.rates.travel, *body)?;
                let origin =
                    proof.rates.point(*guide, *from)? + proof.rates.turn(*guide)? * reach + along;
                put(
                    &mut proof.rates.offset,
                    *body,
                    origin + turn * from.length(),
                )?;
                continue;
            }
            (Step::Rail { runner, guide, .. }, _) => {
                // The planner only puts a rail along a member that doesn't
                // turn.
                if proof.rates.turn(*guide)? != 0.0 {
                    return Err(Fault);
                }
                let origin = get(&proof.rates.offset, *guide)? + get(&proof.rates.travel, *runner)?;
                proof.rates.place(*runner, 0.0, origin)?;
                continue;
            }
            (Step::Dyad(dyad), (Some(a), Some(b))) => (proof.dyad(dyad, a, b)?, a, b),
            (Step::SlotInLink(slot), (Some(a), Some(b))) => (proof.slot_in_link(slot, a, b)?, a, b),
            (Step::Dyad(_) | Step::SlotInLink(_), _) => return Err(Fault),
        };
        match found {
            (Some(floor), _, _) => least = Some(least.map_or(floor, |least| least.min(floor))),
            (None, a, b) => {
                return Ok(Outcome::Unproven {
                    step: index,
                    at_end: b.margin < a.margin,
                });
            }
        }
    }
    Ok(Outcome::Proven(least))
}
