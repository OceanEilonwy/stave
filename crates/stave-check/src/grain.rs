//! Grain (grammar, "Grain and wood movement"): solid wood is strong along
//! its grain and weak across it, and it swells and shrinks across its
//! grain with the seasons, hardly at all along it. Four warnings, each
//! `warning[grain]`, each for every kind alike, read from the language's
//! own words (`glue:`, the `tenon` and `spigot` operations, `drill`, a
//! joint's members and what it adds):
//!
//! 1. **End grain glued** (at the joint): a glued pair whose glue is all
//!    on the patch where the parts touch (the joint cuts, places, makes
//!    and adds nothing) and where one part meets the other with its end.
//! 2. **Glue across the grain** (at the joint): such a pair where the
//!    other part holds still along the way a solid part moves, over more
//!    than the moving part's limit (its stock's `max_held_width:`).
//! 3. **Short grain** (at the joint): a `tenon` or `spigot` cut on the end
//!    of a solid part whose grain runs along its width, across the tenon.
//! 4. **Held across the grain** (at the part): a solid part fixed at places
//!    further apart across its grain than its limit: by glue on a
//!    patch, by glue in a housing it is extended into across its grain, or
//!    by a member (a screw, a dowel) that has a round hole in it and in
//!    another part. Fixings in one other part count with their spread
//!    only when that part holds still that way (a sheet, or solid wood
//!    whose grain runs that way); fixings in different parts count with
//!    the distance between where each part holds it.
//!
//! Asked only of a design with no errors, as the held check is.

use std::collections::{BTreeMap, BTreeSet};

use stave_lang::placement::{Axis, PartAxis};
use stave_lang::units::Length;
use stave_lang::{Code, Severity};
use stave_model::solved::{
    BodyOwner, BodyRef, Contact as PairContact, Cut, CutKind, CutPlace, CutSource, End, JointId,
    Part, PartId, PartOrigin, RoleValue, Shape, Size3, WorldBox,
};
use stave_schema::fields::DEFAULT_MAX_HELD_WIDTH_NM;
use stave_schema::words::{Grain, StockForm};

use stave_geom::solid::across;

use crate::names::length_text;
use crate::walls::cut_noun;
use crate::{Checker, Contact, Item};

/// The widest a solid board may be held fast across its grain: its
/// stock's `max_held_width:`, which the solver fills with 80mm when the
/// stock doesn't say. The default here only stands in for a part that
/// isn't a solid board, which is never measured.
fn limit(part: &Part) -> Length {
    part.stock
        .max_held_width
        .unwrap_or(Length::from_nm(DEFAULT_MAX_HELD_WIDTH_NM))
}

/// Where a board's limit comes from, as a help line says it: `the
/// max_held_width of oak`.
fn limit_source(part: &Part) -> String {
    format!("the max_held_width of {}", part.stock.name)
}

/// The world axis a part's grain runs along.
fn grain_axis(part: &Part) -> Axis {
    let own = match part.grain {
        Grain::Length => PartAxis::Length,
        Grain::Width => PartAxis::Width,
    };
    part.pose.orientation.direction(own).axis()
}

/// The world axis a solid board moves along with the seasons: across its
/// grain, in its faces (its width, or its length for `grain: width`).
fn movement_axis(part: &Part) -> Axis {
    let own = match part.grain {
        Grain::Length => PartAxis::Width,
        Grain::Width => PartAxis::Length,
    };
    part.pose.orientation.direction(own).axis()
}

/// `true` for a board of solid wood: what moves across its grain.
fn moves(part: &Part) -> bool {
    part.stock.form == StockForm::Solid && part.shape == Shape::Board
}

/// `true` when a part has grain, so its end grain is end grain.
fn has_grain(part: &Part) -> bool {
    match part.stock.form {
        StockForm::Solid | StockForm::Dowel => true,
        StockForm::Plywood | StockForm::Mdf => false,
    }
}

/// `true` when a part holds still along a world axis: a sheet holds still
/// every way, solid wood along its grain.
fn holds_still(part: &Part, axis: Axis) -> bool {
    match part.stock.form {
        StockForm::Plywood | StockForm::Mdf => true,
        StockForm::Solid | StockForm::Dowel => grain_axis(part) == axis,
    }
}

/// `a - b`, never below what an `i64` holds.
fn difference(a: Length, b: Length) -> Length {
    Length::from_nm(a.nm().saturating_sub(b.nm()))
}

/// The box two boxes share (flat where they only touch).
fn shared_box(a: &WorldBox, b: &WorldBox) -> Option<WorldBox> {
    let mut low = a.min();
    let mut high = a.max();
    for axis in Axis::ALL {
        let from = a.min().get(axis).max(b.min().get(axis));
        let to = a.max().get(axis).min(b.max().get(axis));
        if from > to {
            return None;
        }
        low = low.with(axis, from);
        high = high.with(axis, to);
    }
    Some(WorldBox::new(low, high))
}

/// The group of every part, by index, joined through the links with
/// `left_out` taken out of each: parts in one group hold each other other
/// than through it.
fn joined_without(left_out: PartId, links: &[Vec<PartId>], count: usize) -> Vec<usize> {
    let mut parent: Vec<usize> = (0..count).collect();
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
    for link in links {
        let mut rest = link.iter().filter(|part| **part != left_out);
        let Some(first) = rest.next() else {
            continue;
        };
        for part in rest {
            let (a, b) = (
                root(&mut parent, first.index()),
                root(&mut parent, part.index()),
            );
            let (low, high) = if a <= b { (a, b) } else { (b, a) };
            if let Some(entry) = parent.get_mut(high) {
                *entry = low;
            }
        }
    }
    (0..count).map(|index| root(&mut parent, index)).collect()
}

/// One copy of a member: which member, and its shape's whole world box
/// (corners, in nanometres), the same in every part it is drilled into.
type HoleKey = (BodyRef, [i64; 6]);

/// Where along an axis a part is held, and by what.
#[derive(Debug, Clone)]
struct Fixing {
    /// The other part it is held to.
    other: PartId,
    /// Where it starts along the part's movement axis, in the world.
    low: Length,
    /// Where it ends.
    high: Length,
    /// The joint or hardware that holds it.
    by: String,
    /// `true` when a member holds it through a slot that runs the wrong
    /// way: along its grain, so the slot gives it no room to move.
    turned: bool,
}

/// A member's hole in one part: where the member is, and which world
/// axes the slots cut for it there run along (a `pocket` longer one way
/// than the other).
#[derive(Debug, Clone)]
struct Hole {
    /// The part.
    part: PartId,
    /// The member's shape, in the world.
    world: WorldBox,
    /// The world axis each slot for it runs along.
    slots: Vec<Axis>,
}

/// The world axis a `pocket` on a member runs along, when it is longer one
/// way than the other: its width lies along the part's length (or the
/// face's first axis), its thickness across it (grammar, "Cut
/// operations").
fn slot_axis(part: &Part, cut: &Cut) -> Option<Axis> {
    let CutKind::Pocket {
        width, thickness, ..
    } = &cut.kind
    else {
        return None;
    };
    let [first, second] = across(cut.face.axis);
    let world = |axis: PartAxis| part.pose.orientation.direction(axis).axis();
    if width > thickness {
        Some(world(first))
    } else if thickness > width {
        Some(world(second))
    } else {
        None
    }
}

/// How far apart across its grain a board is held fast, as the held-fast
/// warning reports it.
struct HeldSpan {
    /// The distance.
    width: Length,
    /// What holds it at the two places furthest apart.
    notes: Vec<String>,
    /// The members that hold it there through slots that run the wrong
    /// way, by name.
    turned: Vec<String>,
}

/// A glued joint pair whose glue is in its cuts, as [`Checker::glue_in_cuts`]
/// measures it.
struct InCut<'a> {
    /// The joint.
    joint: JointId,
    /// The pair's index in it.
    pair: usize,
    /// The joint's name, for messages.
    name: &'a str,
}

impl InCut<'_> {
    /// The cuts of this pair in `part` that house or shape wood (not a
    /// member's hole: glue in a hole is the member's, and a member is
    /// measured where it is).
    fn cuts<'p>(&self, part: &'p Part) -> impl Iterator<Item = &'p Cut> {
        part.cuts.iter().filter(move |cut| {
            let ours = matches!(
                cut.source,
                CutSource::Joint { joint, pair } if joint == self.joint && pair.index() == self.pair
            );
            let hole = matches!(cut.place, CutPlace::Member { .. })
                || matches!(cut.kind, CutKind::Drill { .. });
            ours && !hole
        })
    }
}

/// How wide a tenon or round tenon cut on `part` is along `axis` (a world
/// axis): the wood left in the housing. `None` for any other cut.
fn shaped_along(part: &Part, cut: &Cut, axis: Axis) -> Option<Length> {
    let local = part.pose.orientation.part_axis(axis);
    match &cut.kind {
        CutKind::Tenon {
            length,
            width,
            thickness,
        } => {
            // On an end, the tenon's width and thickness are across the
            // part's; on an edge (a tongue), its width runs along the
            // part's length (as the geometry cuts it).
            let (width_axis, thickness_axis) = match cut.face.axis {
                PartAxis::Length => (PartAxis::Width, PartAxis::Thickness),
                PartAxis::Width => (PartAxis::Length, PartAxis::Thickness),
                PartAxis::Thickness => (PartAxis::Length, PartAxis::Width),
            };
            Some(if local == width_axis {
                *width
            } else if local == thickness_axis {
                *thickness
            } else {
                *length
            })
        }
        CutKind::Spigot { length, diameter } => Some(if local == cut.face.axis {
            *length
        } else {
            *diameter
        }),
        CutKind::Notch { .. }
        | CutKind::Slot { .. }
        | CutKind::Pocket { .. }
        | CutKind::Round { .. }
        | CutKind::Drill { .. }
        | CutKind::DovetailSocket { .. }
        | CutKind::DovetailTail { .. }
        | CutKind::Outline { .. }
        | CutKind::Diagonal
        | CutKind::Chamfer { .. }
        | CutKind::Roundover { .. } => None,
    }
}

/// How wide a mortise (a `pocket`) cut in another part for `entering` is
/// along `axis` (a world axis): its width lies along the entering part's
/// width, its thickness along its thickness. `None` for any other cut,
/// or along the entering part's length.
fn housed_along(entering: &Part, cut: &Cut, axis: Axis) -> Option<Length> {
    let CutKind::Pocket {
        width, thickness, ..
    } = &cut.kind
    else {
        return None;
    };
    match entering.pose.orientation.part_axis(axis) {
        PartAxis::Width => Some(*width),
        PartAxis::Thickness => Some(*thickness),
        PartAxis::Length => None,
    }
}

impl Checker<'_> {
    /// Glue in the cuts of a glued pair (a tenon's cheeks in its mortise,
    /// a panel's edge in its groove, a shelf's end in its dado): where the
    /// two parts' cut boxes overlap, narrowed to a tenon or a mortise cut
    /// for the pair, measured across the grain of each solid board, and
    /// warned like glue on the patch when the other part holds still that
    /// way over more than the board's limit. It is not taken as a fixing
    /// for "held fast" (grammar, "Grain and wood movement"): a round tenon
    /// glued in its hole flexes with the legs it joins, as a chair's do.
    fn glue_in_cuts(
        &mut self,
        glued: &InCut<'_>,
        a: &Part,
        b: &Part,
        warned: &mut BTreeSet<PartId>,
    ) {
        let design = self.design;
        let Some(overlap) = shared_box(&a.cut_box(), &b.cut_box()) else {
            return;
        };
        // Parts that only touch house nothing.
        if Axis::ALL
            .into_iter()
            .any(|axis| overlap.size(axis) == Length::ZERO)
        {
            return;
        }
        for (part, other) in [(a, b), (b, a)] {
            if !moves(part) {
                continue;
            }
            // What messages call the housing: the cut in the other part,
            // or else the cut on this one.
            let Some(housing) = glued.cuts(other).next().or_else(|| glued.cuts(part).next()) else {
                continue;
            };
            let axis = movement_axis(part);
            let mut across = overlap.size(axis);
            for size in glued
                .cuts(part)
                .filter_map(|cut| shaped_along(part, cut, axis))
                .chain(
                    glued
                        .cuts(other)
                        .filter_map(|cut| housed_along(part, cut, axis)),
                )
            {
                across = across.min(size);
            }
            if !holds_still(other, axis) || across <= limit(part) || warned.contains(&part.id) {
                continue;
            }
            let noun = cut_noun(housing);
            self.next_contact = Some(Contact::of(
                design,
                &[Item::Part(part.id), Item::Part(other.id)],
                &[],
            ));
            self.report(
                Severity::Warning,
                Code::Grain,
                format!(
                    "{}: {} is glued across its grain to {} over {} in the {noun}, and wood held that wide splits as it moves with the seasons",
                    glued.name,
                    self.part_name(part.id),
                    self.part_name(other.id),
                    length_text(across)
                ),
                self.joint_place(glued.joint),
                Vec::new(),
                Some(format!(
                    "leave the glue out of {} (glue: false) so {} can move in the {noun}, or make the {noun} no wider than {} across the grain of {} ({})",
                    glued.name,
                    self.part_name(part.id),
                    length_text(limit(part)),
                    self.part_name(part.id),
                    limit_source(part)
                )),
            );
            warned.insert(part.id);
        }
    }

    pub(crate) fn grain(&mut self) {
        let design = self.design;
        let cut_pairs: BTreeSet<(JointId, usize)> = design
            .parts()
            .iter()
            .flat_map(|part| &part.cuts)
            .filter_map(|cut| match cut.source {
                CutSource::Joint { joint, pair } => Some((joint, pair.index())),
                CutSource::Hardware { .. }
                | CutSource::Boards { .. }
                | CutSource::Edges { .. }
                | CutSource::Shape { .. } => None,
            })
            .collect();
        let fastened: BTreeSet<JointId> = design
            .fasteners
            .iter()
            .filter(|use_| use_.count > 0)
            .map(|use_| use_.joint)
            .collect();
        let mut fixings: BTreeMap<PartId, Vec<Fixing>> = BTreeMap::new();
        let mut warned: BTreeSet<PartId> = BTreeSet::new();

        for joint in design.joints() {
            if !joint.glue {
                continue;
            }
            for (index, pair) in joint.indexed_pairs() {
                let parts: Vec<&Part> = pair
                    .roles
                    .iter()
                    .filter_map(|role| design.part(role.part))
                    .collect();
                let [a, b] = parts.as_slice() else {
                    continue;
                };
                // Glue in a housing the joint extends a part into, across
                // its grain, holds it at that edge.
                for extension in &pair.extensions {
                    let Some(part) = design.part(extension.part) else {
                        continue;
                    };
                    if !moves(part) {
                        continue;
                    }
                    let direction = part.pose.orientation.direction(extension.axis);
                    if direction.axis() != movement_axis(part) {
                        continue;
                    }
                    let drawn = part.drawn_box();
                    let at_max = matches!(
                        (extension.end, direction.is_positive()),
                        (End::End, true) | (End::Start, false)
                    );
                    let at = if at_max {
                        drawn.max().get(direction.axis())
                    } else {
                        drawn.min().get(direction.axis())
                    };
                    let other = if a.id == part.id { b.id } else { a.id };
                    fixings.entry(part.id).or_default().push(Fixing {
                        other,
                        low: at,
                        high: at,
                        by: joint.name.clone(),
                        turned: false,
                    });
                }
                // Glue spread on the patch where the parts touch, or, for
                // a joint that cuts, places, makes or adds something, in
                // that: in the cuts where one part houses the other.
                let on_patch = pair.members.is_empty()
                    && pair.made_parts.is_empty()
                    && pair.extensions.is_empty()
                    && !fastened.contains(&joint.id)
                    && !cut_pairs.contains(&(joint.id, index.index()));
                if !on_patch {
                    let glued = InCut {
                        joint: joint.id,
                        pair: index.index(),
                        name: &joint.name,
                    };
                    self.glue_in_cuts(&glued, a, b, &mut warned);
                    continue;
                }
                let PairContact::Butt { axis: normal } = pair.contact else {
                    continue;
                };
                let Some(patch) = shared_box(&a.drawn_box(), &b.drawn_box()) else {
                    continue;
                };
                // End grain first: it is the mistake, and the glue across
                // the grain only what it leaves.
                if let Some(end) = [*a, *b]
                    .into_iter()
                    .find(|part| has_grain(part) && grain_axis(part) == normal)
                {
                    let other = if a.id == end.id { *b } else { *a };
                    self.next_contact = Some(Contact::of(
                        design,
                        &[Item::Part(a.id), Item::Part(b.id)],
                        &[],
                    ));
                    self.report(
                        Severity::Warning,
                        Code::Grain,
                        format!(
                            "{}: {} meets {} with its end grain, and glue on end grain holds almost nothing",
                            joint.name,
                            self.part_name(end.id),
                            self.part_name(other.id)
                        ),
                        self.joint_place(joint.id),
                        Vec::new(),
                        Some(format!(
                            "join them with a kind that cuts the parts or places fasteners in them, so the glue holds on long grain, or turn {} so it meets {} with a face or an edge",
                            self.part_name(end.id),
                            self.part_name(other.id)
                        )),
                    );
                    warned.insert(a.id);
                    warned.insert(b.id);
                    continue;
                }
                for (part, other) in [(*a, *b), (*b, *a)] {
                    if !moves(part) {
                        continue;
                    }
                    let axis = movement_axis(part);
                    fixings.entry(part.id).or_default().push(Fixing {
                        other: other.id,
                        low: patch.min().get(axis),
                        high: patch.max().get(axis),
                        by: joint.name.clone(),
                        turned: false,
                    });
                    let across = patch.size(axis);
                    if holds_still(other, axis)
                        && across > limit(part)
                        && !warned.contains(&part.id)
                    {
                        self.next_contact = Some(Contact::of(
                            design,
                            &[Item::Part(part.id), Item::Part(other.id)],
                            &[],
                        ));
                        self.report(
                            Severity::Warning,
                            Code::Grain,
                            format!(
                                "{}: {} is glued across its grain to {} over {}, and wood held that wide splits as it moves with the seasons",
                                joint.name,
                                self.part_name(part.id),
                                self.part_name(other.id),
                                length_text(across)
                            ),
                            self.joint_place(joint.id),
                            Vec::new(),
                            Some(format!(
                                "glue only a strip no wider than {} across the grain of {} ({}), or hold it with fasteners that let it move",
                                length_text(limit(part)),
                                self.part_name(part.id),
                                limit_source(part)
                            )),
                        );
                        warned.insert(part.id);
                    }
                }
            }
        }

        self.short_grain();
        self.member_fixings(&mut fixings);
        let links = self.links();

        for (part_id, held) in fixings {
            if warned.contains(&part_id) {
                continue;
            }
            let Some(part) = design.part(part_id) else {
                continue;
            };
            if let Some(HeldSpan {
                width: span,
                notes,
                turned,
            }) = self.part_span(part, &held, &links)
                && span > limit(part)
            {
                let help = if turned.is_empty() {
                    format!(
                        "hold {} fast along one line only and let it slide everywhere else: fasten it through slots, use fittings made to let it move, or keep its fixings within {} of each other across its grain ({})",
                        self.part_name(part_id),
                        length_text(limit(part)),
                        limit_source(part)
                    )
                } else {
                    format!(
                        "turn the slots of {} a quarter turn so they run across the grain of {}, the way it moves: a slot that runs along the grain gives it no room",
                        crate::names::list(&turned),
                        self.part_name(part_id)
                    )
                };
                self.next_contact = Some(Contact::of(design, &[Item::Part(part_id)], &[]));
                self.report(
                    Severity::Warning,
                    Code::Grain,
                    format!(
                        "{} is held fast at places {} apart across its grain, so it can't swell and shrink with the seasons",
                        self.part_name(part_id),
                        length_text(span)
                    ),
                    self.part_place(part_id),
                    notes,
                    Some(help),
                );
            }
        }
    }

    /// A tenon or round tenon on the end of a solid part whose grain runs
    /// along its width: across the tenon.
    fn short_grain(&mut self) {
        let design = self.design;
        for part in design.parts() {
            if part.stock.form != StockForm::Solid || part.grain != Grain::Width {
                continue;
            }
            for cut in &part.cuts {
                let CutPlace::End { toward } = cut.place else {
                    continue;
                };
                let (CutKind::Tenon { .. } | CutKind::Spigot { .. }) = cut.kind else {
                    continue;
                };
                let CutSource::Joint { joint, .. } = cut.source else {
                    continue;
                };
                self.next_contact = Some(Contact::of(
                    design,
                    &[Item::Part(part.id), Item::Part(toward)],
                    &[],
                ));
                let noun = cut_noun(cut);
                self.report(
                    Severity::Warning,
                    Code::Grain,
                    format!(
                        "{}: the grain of {} runs across its {noun}, so the {noun} can snap off along the grain",
                        self.joint_name(joint),
                        self.part_name(part.id)
                    ),
                    self.joint_place(joint),
                    Vec::new(),
                    Some(format!(
                        "make {} with its grain along its length (grain: length), so the grain runs along the {noun}",
                        self.part_name(part.id)
                    )),
                );
            }
        }
    }

    /// Every member with a hole in two parts: each solid board is held to
    /// the other part where it is, unless a slot in either part runs the
    /// way the board moves (across its grain). A slot that runs another
    /// way still holds it, and is marked so the help can say to turn it.
    fn member_fixings(&self, fixings: &mut BTreeMap<PartId, Vec<Fixing>>) {
        let design = self.design;
        // A member's copy is known by its shape's whole box, which is the
        // same in every part it is cut into.
        let mut holes: BTreeMap<HoleKey, Vec<Hole>> = BTreeMap::new();
        for part in design.parts() {
            for cut in &part.cuts {
                let CutPlace::Member {
                    owner, body, shape, ..
                } = &cut.place
                else {
                    continue;
                };
                let slot = match cut.kind {
                    CutKind::Drill { .. } => None,
                    CutKind::Pocket { .. } => slot_axis(part, cut),
                    CutKind::Notch { .. }
                    | CutKind::Slot { .. }
                    | CutKind::Tenon { .. }
                    | CutKind::Spigot { .. }
                    | CutKind::Round { .. }
                    | CutKind::DovetailSocket { .. }
                    | CutKind::DovetailTail { .. }
                    | CutKind::Outline { .. }
                    | CutKind::Diagonal
                    | CutKind::Chamfer { .. }
                    | CutKind::Roundover { .. } => continue,
                };
                let size = Size3 {
                    length: difference(shape.max.length, shape.min.length),
                    width: difference(shape.max.width, shape.min.width),
                    thickness: difference(shape.max.thickness, shape.min.thickness),
                };
                let offset = Size3 {
                    length: shape.min.length,
                    width: shape.min.width,
                    thickness: shape.min.thickness,
                };
                let world = part.pose.world_box_at(offset, size);
                let key = [
                    world.min().across.nm(),
                    world.min().up.nm(),
                    world.min().deep.nm(),
                    world.max().across.nm(),
                    world.max().up.nm(),
                    world.max().deep.nm(),
                ];
                let id = BodyRef {
                    owner: *owner,
                    body: *body,
                };
                let entry = holes.entry((id, key)).or_default();
                let index = match entry.iter().position(|hole| hole.part == part.id) {
                    Some(index) => index,
                    None => {
                        entry.push(Hole {
                            part: part.id,
                            world,
                            slots: Vec::new(),
                        });
                        entry.len().saturating_sub(1)
                    }
                };
                if let (Some(axis), Some(hole)) = (slot, entry.get_mut(index)) {
                    hole.slots.push(axis);
                }
            }
        }
        for ((id, _), parts) in holes {
            if parts.len() < 2 {
                continue;
            }
            let by = match id.owner {
                BodyOwner::Joint { joint, .. } => self.joint_name(joint),
                BodyOwner::Hardware { hardware } => self.hardware_name(hardware),
            };
            for hole in &parts {
                let Some(part) = design.part(hole.part) else {
                    continue;
                };
                if !moves(part) {
                    continue;
                }
                let axis = movement_axis(part);
                let middle = hole
                    .world
                    .min()
                    .get(axis)
                    .nm()
                    .saturating_add(hole.world.max().get(axis).nm())
                    / 2;
                for other in parts.iter().filter(|other| other.part != hole.part) {
                    let room = hole.slots.contains(&axis) || other.slots.contains(&axis);
                    if room {
                        continue;
                    }
                    let turned = !hole.slots.is_empty() || !other.slots.is_empty();
                    fixings.entry(hole.part).or_default().push(Fixing {
                        other: other.part,
                        low: Length::from_nm(middle),
                        high: Length::from_nm(middle),
                        by: by.clone(),
                        turned,
                    });
                }
            }
        }
    }

    /// The groups of parts that hold each other, one for every joint pair
    /// (with the parts it makes), part made of boards and piece of
    /// hardware, as the held check binds them.
    fn links(&self) -> Vec<Vec<PartId>> {
        let design = self.design;
        let mut links: Vec<Vec<PartId>> = Vec::new();
        for joint in design.joints() {
            for pair in &joint.pairs {
                let mut parts: Vec<PartId> = pair.roles.iter().map(|role| role.part).collect();
                parts.extend(pair.made_parts.iter().copied());
                links.push(parts);
            }
        }
        for part in design.parts() {
            if let PartOrigin::Boards { part: whole, .. } = part.origin {
                links.push(vec![whole, part.id]);
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
                if let Some(part) = member.carried_by() {
                    parts.push(part);
                }
            }
            links.push(parts);
        }
        links
    }

    /// How far apart across its grain a part is held fast. A part made of
    /// boards whose seams are not glued moves board by board, so each
    /// board is measured on its own.
    fn part_span(&self, part: &Part, held: &[Fixing], links: &[Vec<PartId>]) -> Option<HeldSpan> {
        let joined = joined_without(part.id, links, self.design.parts().len());
        let loose_boards = part
            .boards
            .as_ref()
            .filter(|boards| boards.methods.iter().all(|method| !method.glue));
        let Some(boards) = loose_boards else {
            return self.held_span(part, held, &joined);
        };
        let axis = part.pose.orientation.direction(boards.axis);
        let origin = part.pose.origin.get(axis.axis()).nm();
        let mut best: Option<HeldSpan> = None;
        for board in boards.boards.iter() {
            let near = board.offset.nm();
            let far = near.saturating_add(board.width.nm());
            let (low, high) = if axis.is_positive() {
                (origin.saturating_add(near), origin.saturating_add(far))
            } else {
                (origin.saturating_sub(far), origin.saturating_sub(near))
            };
            let on_board: Vec<Fixing> = held
                .iter()
                .filter(|fixing| {
                    let middle = fixing.low.nm().saturating_add(fixing.high.nm()) / 2;
                    (low..=high).contains(&middle)
                })
                .cloned()
                .collect();
            if let Some(found) = self.held_span(part, &on_board, &joined)
                && best.as_ref().is_none_or(|span| found.width > span.width)
            {
                best = Some(found);
            }
        }
        best
    }

    /// How far apart across its grain a part is held fast, with notes
    /// naming what holds it at the two places furthest apart. Fixings in
    /// one other part count with their spread only when that part holds
    /// still along the way this one moves; fixings in different parts
    /// with the distance between where each part holds it.
    fn held_span(&self, part: &Part, held: &[Fixing], joined: &[usize]) -> Option<HeldSpan> {
        let axis = movement_axis(part);
        // Each other part: where it holds this one, from and to.
        let mut groups: BTreeMap<PartId, (i64, i64, Vec<String>)> = BTreeMap::new();
        for fixing in held {
            let group = groups.entry(fixing.other).or_insert((
                fixing.low.nm(),
                fixing.high.nm(),
                Vec::new(),
            ));
            group.0 = group.0.min(fixing.low.nm());
            group.1 = group.1.max(fixing.high.nm());
            if !group.2.contains(&fixing.by) {
                group.2.push(fixing.by.clone());
            }
        }
        let held_to = |other: PartId, by: &[String]| {
            format!(
                "held to {} by {}",
                self.part_name(other),
                crate::names::list(by)
            )
        };
        let mut best: Option<(i64, Vec<String>, Vec<PartId>)> = None;
        for (other, (low, high, by)) in &groups {
            let still = self
                .design
                .part(*other)
                .is_some_and(|other| holds_still(other, axis));
            let spread = high.saturating_sub(*low);
            if still && best.as_ref().is_none_or(|(span, _, _)| spread > *span) {
                best = Some((spread, vec![held_to(*other, by)], vec![*other]));
            }
        }
        // Between parts joined to each other other than through this one:
        // the middles of where each holds it, doubled to stay whole.
        let middles: Vec<(i64, PartId, &Vec<String>)> = groups
            .iter()
            .map(|(other, (low, high, by))| (low.saturating_add(*high), *other, by))
            .collect();
        let group = |part: PartId| joined.get(part.index()).copied();
        for (index, first) in middles.iter().enumerate() {
            for second in middles.iter().skip(index + 1) {
                if group(first.1) != group(second.1) {
                    continue;
                }
                let spread = first.0.abs_diff(second.0) / 2;
                let spread = i64::try_from(spread).unwrap_or(i64::MAX);
                if best.as_ref().is_none_or(|(span, _, _)| spread > *span) {
                    let (low, high) = if first.0 <= second.0 {
                        (first, second)
                    } else {
                        (second, first)
                    };
                    best = Some((
                        spread,
                        vec![held_to(low.1, low.2), held_to(high.1, high.2)],
                        vec![low.1, high.1],
                    ));
                }
            }
        }
        best.map(|(span, notes, others)| {
            // The members that hold it there through slots turned the
            // wrong way.
            let mut turned: Vec<String> = held
                .iter()
                .filter(|fixing| fixing.turned && others.contains(&fixing.other))
                .map(|fixing| fixing.by.clone())
                .collect();
            turned.sort();
            turned.dedup();
            HeldSpan {
                width: Length::from_nm(span),
                notes,
                turned,
            }
        })
    }
}
