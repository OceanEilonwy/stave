//! Cut solids: each part as it is cut from stock, with its cuts made.
//!
//! A part's [`Solid`] lives in the part's own frame (see
//! [`stave_model::solved`]): local coordinates run from `0` to the cut size
//! along `length`, `width` and `thickness`.
//!
//! - A **board** is a set of disjoint, axis-aligned [`Block`]s in whole
//!   nanometres ([`Blocks`]): the cut box minus every axis-aligned cut
//!   (`notch`, `pocket`, `tenon`, a `slot` along one of the part's axes,
//!   the square ends a `spigot` or `round` leaves). Box minus box is exact,
//!   so these cuts are exact.
//! - **Angled cuts** (`dovetail_socket`, `dovetail_tail`, `outline` and a
//!   `slot` turned by an angle that isn't a multiple of 90deg) are
//!   [`Prism`]s: a polygon on a face cut straight down from it (grammar,
//!   "Cut operations"). Each is split once into convex pieces; the pieces
//!   that are axis-aligned rectangles are taken out of the blocks like any
//!   box, and the others, the slanted pieces, are kept in
//!   [`Solid::prisms`]: the wood is the blocks less those pieces.
//! - **Round features** are [`Rod`]s: cylinders along one of the part's
//!   axes. A `spigot` or `round` adds one to a board; round stock and turned
//!   parts are a lathe [`Profile`] along the length; a `drill` removes one
//!   (a hole). Their sizes and positions are exact; only the collision
//!   tests and the meshes that use them work in floats.
//! - Cuts on round stock and turned parts other than `drill` and `spigot`
//!   can't be made yet: they are listed in [`Solid::skipped`] and the
//!   checker warns.
//!
//! A polygon's points are exact nanometres. Where a shape needs an
//! irrational number (a dovetail's offset `depth × tan(angle)`, the
//! corners of a slot turned by an angle), it is computed once from the
//! exact angle and length and rounded to the nanometre when the prism is
//! built, the same rule as a hole's position. A dovetail given as a slope
//! `1 in n` spreads by `depth / n`, exact whenever that is a whole number
//! of nanometres.
//!
//! Simplifications (recorded here, as the plan asks): the `flats` of a
//! `round` are kept on its rod for drawing but collision tests use the full
//! round (a little more wood than there is); a smooth turned profile is
//! checked as straight spans between its stations, each as wide as its wider
//! end; a hole's position is rounded to the nanometre when it falls between
//! two.

use stave_lang::ast::CutOperation;
use stave_lang::placement::PartAxis;
use stave_lang::units::{Angle, Length};
use stave_model::solved::{
    Boards, Cut, CutKind, CutPlace, Depth, End, LocalFace, Mark, Part, Shape, Size3, Slope,
    SolvedDesign, Tilt,
};

pub use crate::prism::{ConvexPolygon, DepthRange, Point2, Polygon, PolygonError, Prism};
use crate::render;

/// A point in a part's own frame, in nanometres.
pub type Local = Size3;

/// A box in a part's own frame with a positive size along every axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Block {
    min: Local,
    max: Local,
}

impl Block {
    /// The box between two corners; `None` unless `min < max` on every
    /// axis.
    pub fn new(min: Local, max: Local) -> Option<Block> {
        PartAxis::ALL
            .into_iter()
            .all(|axis| min.get(axis) < max.get(axis))
            .then_some(Block { min, max })
    }

    /// The corner nearest the local origin.
    pub fn min(&self) -> Local {
        self.min
    }

    /// The opposite corner.
    pub fn max(&self) -> Local {
        self.max
    }

    /// The size along an axis (always positive).
    pub fn size(&self, axis: PartAxis) -> Length {
        Length::from_nm(
            self.max
                .get(axis)
                .nm()
                .saturating_sub(self.min.get(axis).nm()),
        )
    }

    /// The box both share, if it has a volume.
    pub fn intersection(&self, other: &Block) -> Option<Block> {
        let mut min = self.min;
        let mut max = self.max;
        for axis in PartAxis::ALL {
            min = min.with(axis, min.get(axis).max(other.min.get(axis)));
            max = max.with(axis, max.get(axis).min(other.max.get(axis)));
        }
        Block::new(min, max)
    }

    /// This box minus `cut`: at most six disjoint boxes. Slabs are taken
    /// along `length`, then `width`, then `thickness`.
    fn minus(&self, cut: &Block) -> Vec<Block> {
        let Some(inner) = self.intersection(cut) else {
            return vec![*self];
        };
        let mut pieces = Vec::new();
        let mut rest = *self;
        for axis in PartAxis::ALL {
            // Below the cut along this axis.
            if let Some(below) = Block::new(rest.min, rest.max.with(axis, inner.min.get(axis))) {
                pieces.push(below);
            }
            // Above it.
            if let Some(above) = Block::new(rest.min.with(axis, inner.max.get(axis)), rest.max) {
                pieces.push(above);
            }
            rest = Block {
                min: rest.min.with(axis, inner.min.get(axis)),
                max: rest.max.with(axis, inner.max.get(axis)),
            };
        }
        pieces
    }
}

/// Disjoint blocks: boxes added and removed. The only ways to make one are
/// a single block, adding a block (only what isn't there yet is added) and
/// subtraction, so the blocks never overlap.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Blocks {
    blocks: Vec<Block>,
}

impl Blocks {
    /// One block.
    pub fn new(block: Block) -> Blocks {
        Blocks {
            blocks: vec![block],
        }
    }

    /// No wood at all.
    pub fn empty() -> Blocks {
        Blocks::default()
    }

    /// Removes a box.
    pub fn subtract(&mut self, cut: &Block) {
        if !self.blocks.iter().any(|b| b.intersection(cut).is_some()) {
            return;
        }
        self.blocks = self.blocks.iter().flat_map(|b| b.minus(cut)).collect();
    }

    /// Adds a box: the part of it not already covered.
    pub fn add(&mut self, block: Block) {
        let mut pieces = vec![block];
        for existing in &self.blocks {
            if pieces.iter().any(|p| p.intersection(existing).is_some()) {
                pieces = pieces.iter().flat_map(|p| p.minus(existing)).collect();
            }
        }
        self.blocks.extend(pieces);
    }

    /// The blocks moved `by` along `axis`.
    fn shifted(&self, axis: PartAxis, by: Length) -> Blocks {
        let shift = |point: Local| {
            point.with(
                axis,
                point.get(axis).checked_add(by).unwrap_or(point.get(axis)),
            )
        };
        Blocks {
            blocks: self
                .blocks
                .iter()
                .map(|b| Block {
                    min: shift(b.min),
                    max: shift(b.max),
                })
                .collect(),
        }
    }

    /// The blocks.
    pub fn iter(&self) -> impl Iterator<Item = &Block> {
        self.blocks.iter()
    }

    /// The number of blocks.
    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    /// `true` when nothing is left.
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// The total volume in cubic nanometres.
    pub fn volume(&self) -> i128 {
        self.blocks
            .iter()
            .map(|b| {
                PartAxis::ALL
                    .into_iter()
                    .map(|axis| i128::from(b.size(axis).nm()))
                    .product::<i128>()
            })
            .sum()
    }
}

/// The two axes across a part axis, in [`PartAxis::ALL`] order.
pub fn across(axis: PartAxis) -> [PartAxis; 2] {
    match axis {
        PartAxis::Length => [PartAxis::Width, PartAxis::Thickness],
        PartAxis::Width => [PartAxis::Length, PartAxis::Thickness],
        PartAxis::Thickness => [PartAxis::Length, PartAxis::Width],
    }
}

/// Two flat faces kept on a round section, `across` apart, their normal
/// turned `angle` from the part's width axis toward its thickness axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Flats {
    /// The distance between the flats.
    pub across: Length,
    /// The flats' angle.
    pub angle: Angle,
}

/// A cylinder along one of a part's axes: a round tenon, a round section,
/// or (in [`Solid::holes`]) a hole.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rod {
    /// The axis it runs along.
    pub axis: PartAxis,
    /// Its centre on the two axes [`across`] its own, in that order.
    pub centre: [Length; 2],
    /// Its diameter.
    pub diameter: Length,
    /// Where it starts along its axis.
    pub start: Length,
    /// Where it ends along its axis (after `start`).
    pub end: Length,
    /// Flats kept on a round section, drawn but not used by collision
    /// tests.
    pub flats: Option<Flats>,
}

/// A slanted hole (grammar, "Slanted members"): drilled from `face`, its
/// axis meeting the face's plane at `entry` and leaning, going deeper,
/// toward `tilt.toward` by `tilt.angle`. It runs from `start` to `end`
/// along its axis, measured from `entry` into the part. Exact; only the
/// tests and meshes that use it work in floats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Slant {
    /// The face it is drilled from.
    pub face: LocalFace,
    /// Where its axis meets the face's plane, in the part's frame.
    pub entry: Local,
    /// Which way and how far it leans.
    pub tilt: Tilt,
    /// Its diameter.
    pub diameter: Length,
    /// Where it starts along its axis, from `entry`: a little before the
    /// face (negative) for a hole that opens onto it.
    pub start: Length,
    /// Where it ends along its axis (after `start`).
    pub end: Length,
}

impl Slant {
    /// The unit vector along its axis, going deeper, in the part's frame
    /// (`x` length, `y` width, `z` thickness).
    pub fn direction(&self) -> glam::DVec3 {
        let angle = self.tilt.angle.to_radians_f64();
        let mut direction = glam::DVec3::ZERO;
        let inward = match self.face.end {
            End::Start => 1.0,
            End::End => -1.0,
        };
        let lean = match self.tilt.toward.end {
            End::Start => -1.0,
            End::End => 1.0,
        };
        direction[local_slot(self.face.axis)] = inward * angle.cos();
        direction[local_slot(self.tilt.toward.axis)] = lean * angle.sin();
        direction
    }

    /// The two ends of its axis, in the part's frame, nanometres.
    pub fn ends(&self) -> (glam::DVec3, glam::DVec3) {
        let entry = glam::DVec3::new(
            render::nm(self.entry.length),
            render::nm(self.entry.width),
            render::nm(self.entry.thickness),
        );
        let direction = self.direction();
        (
            entry + direction * render::nm(self.start),
            entry + direction * render::nm(self.end),
        )
    }

    /// The two ends of its axis, in the part's frame, each rounded to the
    /// nanometre.
    pub fn exact_ends(&self) -> (Local, Local) {
        let (a, b) = self.ends();
        let exact = |point: glam::DVec3| {
            let mut local = Local::default();
            for axis in PartAxis::ALL {
                let value =
                    render::length_from_nm(point[local_slot(axis)].round()).unwrap_or(Length::ZERO);
                local = local.with(axis, value);
            }
            local
        };
        (exact(a), exact(b))
    }
}

/// The slot of a part axis in a local vector.
pub(crate) fn local_slot(axis: PartAxis) -> usize {
    match axis {
        PartAxis::Length => 0,
        PartAxis::Width => 1,
        PartAxis::Thickness => 2,
    }
}

/// A station of a lathe profile: a diameter at a distance along the
/// length, in local coordinates. Two stations at the same distance are a
/// shoulder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProfileStation {
    /// Local length coordinate.
    pub at: Length,
    /// The diameter there.
    pub diameter: Length,
}

/// The lathe profile of round stock or a turned part, over its whole cut
/// length, with what joints add and the round tenons they cut.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Profile {
    /// The stations, in order along the length.
    pub stations: Vec<ProfileStation>,
    /// Draw one smooth curve through them (a turned profile's `smooth:`).
    pub smooth: bool,
    /// The zones along the length a round tenon narrowed (local length
    /// coordinates): their surface is a cut face.
    pub narrowed: Vec<(Length, Length)>,
}

impl Profile {
    /// The diameter at a local length coordinate, on straight lines
    /// between stations.
    pub(crate) fn diameter_at(&self, at: Length) -> Length {
        let mut previous: Option<ProfileStation> = None;
        for station in &self.stations {
            if station.at >= at {
                return match previous {
                    Some(before) if station.at > before.at => {
                        let span = i128::from(station.at.nm() - before.at.nm());
                        let into = i128::from(at.nm() - before.at.nm());
                        let change = i128::from(station.diameter.nm() - before.diameter.nm());
                        let value = i128::from(before.diameter.nm()) + change * into / span;
                        Length::from_nm(i64::try_from(value).unwrap_or(station.diameter.nm()))
                    }
                    _ => station.diameter,
                };
            }
            previous = Some(*station);
        }
        previous.map_or(Length::ZERO, |last| last.diameter)
    }

    /// Narrows the profile to `diameter` over `start..end` (a round
    /// tenon), with shoulders at both ends of that zone.
    fn narrow(&mut self, start: Length, end: Length, diameter: Length) {
        let at_start = self.diameter_at(start);
        let at_end = self.diameter_at(end);
        let first = self.stations.first().map_or(start, |s| s.at);
        let last = self.stations.last().map_or(end, |s| s.at);
        let zone = Zone {
            start,
            end,
            at_start,
            at_end,
            diameter,
            outer_start: start > first,
            outer_end: end < last,
        };
        let mut stations: Vec<ProfileStation> = Vec::new();
        let mut pushed_zone = false;
        for station in &self.stations {
            if station.at < start || station.at > end {
                if station.at > end && !pushed_zone {
                    push_zone(&mut stations, &zone);
                    pushed_zone = true;
                }
                stations.push(*station);
            }
        }
        if !pushed_zone {
            push_zone(&mut stations, &zone);
        }
        self.stations = stations;
        self.narrowed.push((start, end));
    }

    /// The profile as cylinders along the length, centred at `centre`, each
    /// span as wide as its wider end: what collision tests use.
    pub fn rods(&self, centre: [Length; 2]) -> Vec<Rod> {
        self.stations
            .windows(2)
            .filter_map(|pair| {
                let &[from, to] = pair else { return None };
                (to.at > from.at).then_some(Rod {
                    axis: PartAxis::Length,
                    centre,
                    diameter: from.diameter.max(to.diameter),
                    start: from.at,
                    end: to.at,
                    flats: None,
                })
            })
            .filter(|rod| rod.diameter > Length::ZERO)
            .collect()
    }
}

/// The zone a round tenon narrows.
struct Zone {
    start: Length,
    end: Length,
    at_start: Length,
    at_end: Length,
    diameter: Length,
    /// The profile goes on before the zone (a shoulder there).
    outer_start: bool,
    /// The profile goes on after the zone.
    outer_end: bool,
}

fn push_zone(stations: &mut Vec<ProfileStation>, zone: &Zone) {
    if zone.outer_start {
        stations.push(ProfileStation {
            at: zone.start,
            diameter: zone.at_start,
        });
    }
    stations.push(ProfileStation {
        at: zone.start,
        diameter: zone.diameter.min(zone.at_start),
    });
    stations.push(ProfileStation {
        at: zone.end,
        diameter: zone.diameter.min(zone.at_end),
    });
    if zone.outer_end {
        stations.push(ProfileStation {
            at: zone.end,
            diameter: zone.at_end,
        });
    }
}

/// Why a cut was left unmade.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Unsupported {
    /// A cut other than `drill` and `spigot` on round stock or a turned
    /// part.
    OnRoundPart(CutOperation),
}

impl Unsupported {
    /// What can't be cut, with an article: "a notch in a round or turned
    /// part". A `not-supported` warning reads "cutting <this> is not
    /// supported yet".
    pub fn description(&self) -> String {
        match self {
            Unsupported::OnRoundPart(operation) => {
                format!("{} in a round or turned part", operation_noun(*operation))
            }
        }
    }
}

/// An operation with an article: "a dovetail tail".
fn operation_noun(operation: CutOperation) -> &'static str {
    match operation {
        CutOperation::Notch => "a notch",
        CutOperation::Slot => "a slot",
        CutOperation::Pocket => "a mortise",
        CutOperation::Tenon => "a tenon",
        CutOperation::Spigot => "a round tenon",
        CutOperation::Round => "a round section",
        CutOperation::Drill => "a hole",
        CutOperation::DovetailSocket => "a dovetail socket",
        CutOperation::DovetailTail => "a dovetail tail",
        CutOperation::Outline => "an outline",
        CutOperation::Diagonal => "a diagonal",
        CutOperation::Chamfer => "a chamfer",
        CutOperation::Roundover => "a roundover",
    }
}

/// A cut that was left unmade because the geometry can't make it yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Skipped {
    /// Its index in [`Part::cuts`].
    pub cut: usize,
    /// Why.
    pub reason: Unsupported,
}

/// Why a cut's shape is not a shape: a mistake in the design, reported as
/// `error[type-mismatch]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
pub enum ShapeError {
    /// Its polygon is not a simple polygon (an `outline` that crosses
    /// itself, or a dovetail with no width or depth).
    #[error(transparent)]
    Polygon(#[from] PolygonError),
    /// A `slot` whose angle turns it into the face it is cut from.
    #[error("the slot's angle turns it into the face it is cut from")]
    SlotIntoFace,
    /// A dovetail whose angle is 90deg or more.
    #[error("a dovetail's angle must be less than 90deg")]
    TooSteep,
    /// A `diagonal` between two parts that run the same way.
    #[error("its parts run the same way, so they make no corner to cut on the diagonal")]
    DiagonalParallel,
    /// A `diagonal` between two parts that don't cross at an end of each.
    #[error(
        "its parts don't cross at an end of each, so they make no corner to cut on the diagonal"
    )]
    DiagonalNotAtCorner,
}

impl ShapeError {
    /// A concrete help line for the diagnostic.
    pub fn help(&self) -> String {
        match self {
            ShapeError::Polygon(error) => error.help(),
            ShapeError::SlotIntoFace => {
                "give the slot an angle that keeps it on the face, such as 0deg or 45deg".to_owned()
            }
            ShapeError::TooSteep => {
                "dovetails slope between about 7deg and 14deg; try 10deg".to_owned()
            }
            ShapeError::DiagonalParallel | ShapeError::DiagonalNotAtCorner => {
                "draw the two parts square to each other and crossing at the corner, each running right to the outside of it, as a frame's corner is drawn before it is mitred".to_owned()
            }
        }
    }
}

/// A cut that was left unmade because its shape is a mistake.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Misshapen {
    /// Its index among the part's cuts (see [`cut_at`]).
    pub cut: usize,
    /// What is wrong with it.
    pub error: ShapeError,
}

/// Why [`Solid::make`] left a cut unmade.
enum Unmade {
    NotSupported(Unsupported),
    Misshapen(ShapeError),
}

impl From<ShapeError> for Unmade {
    fn from(error: ShapeError) -> Unmade {
        Unmade::Misshapen(error)
    }
}

impl From<PolygonError> for Unmade {
    fn from(error: PolygonError) -> Unmade {
        Unmade::Misshapen(ShapeError::Polygon(error))
    }
}

/// A part with its cuts made, in its own frame.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Solid {
    blocks: Blocks,
    rods: Vec<Rod>,
    profile: Option<Profile>,
    holes: Vec<Rod>,
    slants: Vec<Slant>,
    prisms: Vec<Prism>,
    skipped: Vec<Skipped>,
    misshapen: Vec<Misshapen>,
}

impl Solid {
    /// Makes every cut of a part. `design` gives the other parts a cut is
    /// measured against (the entering part of a mortise or a slot).
    pub fn of(part: &Part, design: &SolvedDesign) -> Solid {
        let size = part.cut;
        let centre = [half(size.width), half(size.thickness)];
        let mut from_boards = BoardWood::default();
        let (blocks, profile) = match &part.shape {
            Shape::Board => (
                match &part.boards {
                    Some(boards) if boards_change_the_wood(part, boards) => {
                        from_boards = board_wood(part, boards, design);
                        std::mem::take(&mut from_boards.blocks)
                    }
                    _ => Block::new(Local::default(), size).map_or_else(Blocks::empty, Blocks::new),
                },
                None,
            ),
            Shape::Round { diameter } => (
                Blocks::empty(),
                Some(Profile {
                    stations: vec![
                        ProfileStation {
                            at: Length::ZERO,
                            diameter: *diameter,
                        },
                        ProfileStation {
                            at: size.length,
                            diameter: *diameter,
                        },
                    ],
                    smooth: false,
                    narrowed: Vec::new(),
                }),
            ),
            Shape::Turned {
                stations, smooth, ..
            } => {
                let start = part.drawn_offset.length;
                let mut profile: Vec<ProfileStation> = stations
                    .iter()
                    .map(|station| ProfileStation {
                        at: start.checked_add(station.at).unwrap_or(start),
                        diameter: station.diameter,
                    })
                    .collect();
                // What joints add at either end is drawn at the end
                // station's diameter.
                if let Some(first) = profile.first().copied()
                    && first.at > Length::ZERO
                {
                    profile.insert(
                        0,
                        ProfileStation {
                            at: Length::ZERO,
                            diameter: first.diameter,
                        },
                    );
                }
                if let Some(last) = profile.last().copied()
                    && last.at < size.length
                {
                    profile.push(ProfileStation {
                        at: size.length,
                        diameter: last.diameter,
                    });
                }
                (
                    Blocks::empty(),
                    Some(Profile {
                        stations: profile,
                        smooth: *smooth,
                        narrowed: Vec::new(),
                    }),
                )
            }
        };
        let mut solid = Solid {
            blocks,
            rods: Vec::new(),
            profile,
            holes: Vec::new(),
            slants: Vec::new(),
            prisms: from_boards.prisms,
            skipped: Vec::new(),
            misshapen: from_boards.misshapen,
        };
        for (index, cut) in part.cuts.iter().enumerate() {
            match solid.make(part, cut, design, centre) {
                Ok(()) => {}
                Err(Unmade::NotSupported(reason)) => {
                    solid.skipped.push(Skipped { cut: index, reason });
                }
                Err(Unmade::Misshapen(error)) => {
                    solid.misshapen.push(Misshapen { cut: index, error });
                }
            }
        }
        solid
    }

    /// The blocks of a board (empty for round and turned parts).
    pub fn blocks(&self) -> &Blocks {
        &self.blocks
    }

    /// Round tenons and round sections on a board.
    pub fn rods(&self) -> &[Rod] {
        &self.rods
    }

    /// The lathe profile of round stock or a turned part.
    pub fn profile(&self) -> Option<&Profile> {
        self.profile.as_ref()
    }

    /// The holes drilled in the part.
    pub fn holes(&self) -> &[Rod] {
        &self.holes
    }

    /// The slanted holes drilled in the part (grammar, "Slanted
    /// members").
    pub fn slants(&self) -> &[Slant] {
        &self.slants
    }

    /// The angled cuts with a slanted piece: the wood is the blocks less
    /// their [`Prism::slanted_pieces`] (their rectangular pieces are
    /// already taken out of the blocks).
    pub fn prisms(&self) -> &[Prism] {
        &self.prisms
    }

    /// The cuts the geometry can't make yet.
    pub fn skipped(&self) -> &[Skipped] {
        &self.skipped
    }

    /// The cuts whose shape is a mistake, left unmade.
    pub fn misshapen(&self) -> &[Misshapen] {
        &self.misshapen
    }

    /// Every cylinder of wood: the board's rods and the lathe profile as
    /// spans. `centre` is the lathe axis (the middle of the cut width and
    /// thickness).
    pub fn all_rods(&self, centre: [Length; 2]) -> Vec<Rod> {
        let mut rods = self.rods.clone();
        if let Some(profile) = &self.profile {
            rods.extend(profile.rods(centre));
        }
        rods
    }

    fn make(
        &mut self,
        part: &Part,
        cut: &Cut,
        design: &SolvedDesign,
        centre: [Length; 2],
    ) -> Result<(), Unmade> {
        let size = part.cut;
        let operation = cut.kind.operation();
        let round_part = self.profile.is_some();
        match &cut.kind {
            CutKind::Drill {
                diameter,
                depth,
                from_depth,
                tilt,
            } => {
                if let Some(tilt) = tilt {
                    self.slants
                        .push(slant_of(part, cut, *diameter, *depth, *from_depth, *tilt));
                    return Ok(());
                }
                let range = drill_range(size, cut.face, *depth, *from_depth);
                self.holes.push(Rod {
                    axis: cut.face.axis,
                    centre: hole_centre(part, cut),
                    diameter: *diameter,
                    start: range.0,
                    end: range.1,
                    flats: None,
                });
                return Ok(());
            }
            CutKind::Spigot { length, diameter } => {
                let (start, end) = depth_range(size, cut.face, Depth::Length(*length));
                if let Some(profile) = &mut self.profile {
                    profile.narrow(start, end, *diameter);
                } else {
                    self.remove_section(size, start, end);
                    self.rods.push(Rod {
                        axis: PartAxis::Length,
                        centre,
                        diameter: *diameter,
                        start,
                        end,
                        flats: None,
                    });
                }
                return Ok(());
            }
            CutKind::Notch { .. }
            | CutKind::Slot { .. }
            | CutKind::Pocket { .. }
            | CutKind::Tenon { .. }
            | CutKind::Round { .. }
            | CutKind::DovetailSocket { .. }
            | CutKind::DovetailTail { .. }
            | CutKind::Outline { .. }
            | CutKind::Diagonal
            | CutKind::Chamfer { .. }
            | CutKind::Roundover { .. } => {}
        }
        if round_part {
            return Err(Unmade::NotSupported(Unsupported::OnRoundPart(operation)));
        }
        match &cut.kind {
            CutKind::Notch { depth } => {
                let mut min = Local::default();
                let mut max = size;
                for axis in across(cut.face.axis) {
                    let (low, high) = face_extent(part, &cut.place, axis);
                    min = min.with(axis, low);
                    max = max.with(axis, high);
                }
                let (low, high) = depth_range(size, cut.face, *depth);
                min = min.with(cut.face.axis, low);
                max = max.with(cut.face.axis, high);
                if let Some(block) = Block::new(min, max) {
                    self.blocks.subtract(&block);
                }
            }
            CutKind::Pocket {
                width,
                thickness,
                depth,
            } => {
                let [first, second] = across(cut.face.axis);
                let (along_width, along_thickness) =
                    entering_axes(part, &cut.place, design, cut.face.axis)
                        .unwrap_or((first, second));
                let middle = place_centre(part, &cut.place, cut.face.axis);
                let mut min = Local::default();
                let mut max = size;
                for (axis, extent) in [(along_width, *width), (along_thickness, *thickness)] {
                    let centre = middle.get(axis);
                    let low = centred_low(centre, extent);
                    min = min.with(axis, low);
                    max = max.with(axis, low.checked_add(extent).unwrap_or(low));
                }
                let (low, high) = depth_range(size, cut.face, *depth);
                min = min.with(cut.face.axis, low);
                max = max.with(cut.face.axis, high);
                if let Some(block) = Block::new(min, max) {
                    self.blocks.subtract(&block);
                }
            }
            CutKind::Tenon {
                length,
                width,
                thickness,
            } => {
                // On an end, the tenon's width and thickness are across the
                // part's; on an edge (a tongue, from `crossing`), its width
                // runs along the part's length. Either way it is centred on
                // the place: the end, or the footprint.
                let face_axis = cut.face.axis;
                let (width_axis, thickness_axis) = match face_axis {
                    PartAxis::Length => (PartAxis::Width, PartAxis::Thickness),
                    PartAxis::Width => (PartAxis::Length, PartAxis::Thickness),
                    PartAxis::Thickness => (PartAxis::Length, PartAxis::Width),
                };
                let (start, end) = depth_range(size, cut.face, Depth::Length(*length));
                let zone = |min: Local, max: Local| {
                    Block::new(min.with(face_axis, start), max.with(face_axis, end))
                };
                let middle = place_centre(part, &cut.place, face_axis);
                let span = |axis: PartAxis, extent: Length| {
                    let full = size.get(axis);
                    let low = centred_low(middle.get(axis), extent).max(Length::ZERO);
                    (low, low.checked_add(extent).unwrap_or(full).min(full))
                };
                let (w0, w1) = span(width_axis, *width);
                let (t0, t1) = span(thickness_axis, *thickness);
                let around = [
                    zone(Local::default(), size.with(width_axis, w0)),
                    zone(Local::default().with(width_axis, w1), size),
                    zone(
                        Local::default().with(width_axis, w0),
                        size.with(width_axis, w1).with(thickness_axis, t0),
                    ),
                    zone(
                        Local::default()
                            .with(width_axis, w0)
                            .with(thickness_axis, t1),
                        size.with(width_axis, w1),
                    ),
                ];
                for block in around.into_iter().flatten() {
                    self.blocks.subtract(&block);
                }
            }
            CutKind::Round {
                diameter,
                flats,
                flats_angle,
            } => {
                let (start, end) = face_extent(part, &cut.place, PartAxis::Length);
                self.remove_section(size, start, end);
                self.rods.push(Rod {
                    axis: PartAxis::Length,
                    centre,
                    diameter: *diameter,
                    start,
                    end,
                    flats: flats.map(|across| Flats {
                        across,
                        angle: *flats_angle,
                    }),
                });
            }
            CutKind::Slot {
                width,
                depth,
                angle,
                from_centre,
            } => {
                let quarter = Angle::from_deg(90).microdeg();
                if angle.microdeg().rem_euclid(quarter) != 0 {
                    let (base, next) = slot_axes(part, &cut.place, design, cut.face.axis)
                        .ok_or(ShapeError::SlotIntoFace)?;
                    let slot = TurnedSlot {
                        face_axis: cut.face.axis,
                        base,
                        next,
                        angle: *angle,
                        centre: place_centre(part, &cut.place, cut.face.axis),
                        width: *width,
                        from_centre: *from_centre,
                    };
                    let polygon = slot.polygon(size)?;
                    self.add_prism(cut.face.axis, polygon, depth_range(size, cut.face, *depth))?;
                    return Ok(());
                }
                let turns = angle.microdeg().div_euclid(quarter).rem_euclid(4);
                let (run, sign) = slot_run(part, &cut.place, design, cut.face.axis, turns)
                    .ok_or(ShapeError::SlotIntoFace)?;
                let [first, second] = across(cut.face.axis);
                let wide = if run == first { second } else { first };
                let middle = place_centre(part, &cut.place, cut.face.axis);
                let mut min = Local::default();
                let mut max = size;
                let low = centred_low(middle.get(wide), *width);
                min = min.with(wide, low);
                max = max.with(wide, low.checked_add(*width).unwrap_or(low));
                if *from_centre {
                    if sign {
                        min = min.with(run, middle.get(run));
                    } else {
                        max = max.with(run, middle.get(run));
                    }
                }
                let (low, high) = depth_range(size, cut.face, *depth);
                min = min.with(cut.face.axis, low);
                max = max.with(cut.face.axis, high);
                if let Some(block) = Block::new(min, max) {
                    self.blocks.subtract(&block);
                }
            }
            CutKind::DovetailSocket { neck, depth, slope } => {
                let dovetail = Dovetail::of(part, cut, design, *neck, *depth, *slope);
                let (axis, polygon, range) = dovetail.socket()?;
                self.add_prism(axis, polygon, range)?;
            }
            CutKind::DovetailTail { neck, depth, slope } => {
                let dovetail = Dovetail::of(part, cut, design, *neck, *depth, *slope);
                for (axis, polygon, range) in dovetail.wedges()? {
                    self.add_prism(axis, polygon, range)?;
                }
            }
            CutKind::Outline {
                marks,
                depth,
                from_depth,
            } => {
                let polygon = outline_polygon(part, cut, marks)?;
                let range = outline_range(size, cut.face, *depth, *from_depth);
                self.add_prism(cut.face.axis, polygon, range)?;
            }
            CutKind::Diagonal => {
                let (axis, polygon) = diagonal_polygon(part, cut, design)?;
                self.add_prism(axis, polygon, (Length::ZERO, size.get(axis)))?;
            }
            CutKind::Chamfer { size: back } => {
                let (axis, polygon, range) = edge_profile(part, cut, EdgeShape::Chamfer(*back))?;
                self.add_prism(axis, polygon, range)?;
            }
            CutKind::Roundover { radius } => {
                let (axis, polygon, range) = edge_profile(part, cut, EdgeShape::Round(*radius))?;
                self.add_prism(axis, polygon, range)?;
            }
            CutKind::Drill { .. } | CutKind::Spigot { .. } => {}
        }
        Ok(())
    }

    /// Cuts a prism: its rectangular pieces out of the blocks, exactly;
    /// the prism itself is kept when it has a slanted piece. A range with
    /// no depth cuts nothing.
    fn add_prism(
        &mut self,
        axis: PartAxis,
        polygon: Polygon,
        range: (Length, Length),
    ) -> Result<(), PolygonError> {
        let Some(depth) = DepthRange::new(range.0, range.1) else {
            return Ok(());
        };
        let prism = Prism::new(axis, polygon, depth)?;
        let [first, second] = across(axis);
        for piece in prism.pieces().iter().filter(|p| p.is_rectangle()) {
            let (low, high) = piece.bounds();
            let min = Local::default()
                .with(first, low.first)
                .with(second, low.second)
                .with(axis, depth.from());
            let max = Local::default()
                .with(first, high.first)
                .with(second, high.second)
                .with(axis, depth.to());
            if let Some(block) = Block::new(min, max) {
                self.blocks.subtract(&block);
            }
        }
        if prism.is_slanted() {
            self.prisms.push(prism);
        }
        Ok(())
    }

    /// Removes the whole cross-section over `start..end` of the length.
    fn remove_section(&mut self, size: Size3, start: Length, end: Length) {
        if let Some(block) = Block::new(
            Local::default().with(PartAxis::Length, start),
            size.with(PartAxis::Length, end),
        ) {
            self.blocks.subtract(&block);
        }
    }
}

/// `true` when a part's boards make its wood differ from its cut box: a
/// board has cuts, or a joint widened one (a tongue).
fn boards_change_the_wood(part: &Part, boards: &Boards) -> bool {
    let widths = boards
        .boards
        .iter()
        .try_fold(Length::ZERO, |sum, board| sum.checked_add(board.width));
    boards.boards.iter().any(|board| !board.cuts.is_empty())
        || widths != Some(part.cut.get(boards.axis))
}

/// A part's wood as its boards make it.
#[derive(Default)]
struct BoardWood {
    blocks: Blocks,
    /// The boards' angled cuts with a slanted piece, in the part's frame.
    prisms: Vec<Prism>,
    /// The boards' cuts whose shape is a mistake, numbered after the
    /// part's own cuts (see [`cut_at`]).
    misshapen: Vec<Misshapen>,
}

/// A part's wood as its boards make it (grammar, "Boards"): each board's
/// box with its edge joints' cuts made, box-shaped and angled, in the
/// part's frame, together. Holes, round tenons and round sections are not
/// made on boards; the solver warns about them.
fn board_wood(part: &Part, boards: &Boards, design: &SolvedDesign) -> BoardWood {
    let mut wood = BoardWood::default();
    let mut numbered = part.cuts.len();
    // One copy of the part, without its boards or cuts, reused for every
    // board: only the size across the boards and the cuts change.
    let mut alone = part.blank(0);
    for board in boards.boards.iter() {
        alone.cut = part.cut.with(boards.axis, board.width);
        alone.cuts.clone_from(&board.cuts);
        let solid = Solid::of(&alone, design);
        for block in solid.blocks.shifted(boards.axis, board.offset).iter() {
            wood.blocks.add(*block);
        }
        wood.prisms.extend(
            solid
                .prisms
                .iter()
                .filter_map(|prism| prism.shifted(boards.axis, board.offset)),
        );
        wood.misshapen
            .extend(solid.misshapen.iter().map(|misshapen| Misshapen {
                cut: numbered.saturating_add(misshapen.cut),
                error: misshapen.error,
            }));
        numbered = numbered.saturating_add(board.cuts.len());
    }
    wood
}

/// A part's cut by its index in [`Solid::misshapen`]: the part's own cuts
/// first, then its boards' cuts, board by board.
pub fn cut_at(part: &Part, index: usize) -> Option<&Cut> {
    part.cuts.get(index).or_else(|| {
        let boards = part.boards.as_ref()?;
        boards
            .boards
            .iter()
            .flat_map(|board| board.cuts.iter())
            .nth(index.checked_sub(part.cuts.len())?)
    })
}

/// Half a length, rounded down to the nanometre.
pub(crate) fn half(length: Length) -> Length {
    Length::from_nm(length.nm().div_euclid(2))
}

/// The low edge of something `extent` wide centred at `centre`.
pub(crate) fn centred_low(centre: Length, extent: Length) -> Length {
    Length::from_nm(centre.nm().saturating_sub(extent.nm().div_euclid(2)))
}

/// The range along a face's axis that a depth below the face covers.
pub(crate) fn depth_range(size: Size3, face: LocalFace, depth: Depth) -> (Length, Length) {
    let full = size.get(face.axis);
    let depth = match depth {
        Depth::Through => full,
        Depth::Length(depth) => depth.min(full).max(Length::ZERO),
    };
    match face.end {
        End::Start => (Length::ZERO, depth),
        End::End => (Length::from_nm(full.nm().saturating_sub(depth.nm())), full),
    }
}

/// A straight hole's range along its axis: `depth` from the face, less
/// the first `from_depth` of it.
pub(crate) fn drill_range(
    size: Size3,
    face: LocalFace,
    depth: Depth,
    from_depth: Length,
) -> (Length, Length) {
    let (start, end) = depth_range(size, face, depth);
    let from_depth = from_depth.max(Length::ZERO);
    match face.end {
        End::Start => (start.checked_add(from_depth).unwrap_or(start).min(end), end),
        End::End => (start, end.checked_sub(from_depth).unwrap_or(end).max(start)),
    }
}

/// Where a slanted `through` hole ends along its axis: where its axis
/// leaves the part, and on until the whole of its last disc is out of the
/// face it leaves by (`radius × cot` of the angle it meets that face at).
/// Rounded up to the nanometre.
fn through_end(size: Size3, face: LocalFace, entry: Local, tilt: Tilt, diameter: Length) -> Length {
    let probe = Slant {
        face,
        entry,
        tilt,
        diameter,
        start: Length::ZERO,
        end: Length::ZERO,
    };
    let direction = probe.direction();
    let origin = glam::DVec3::new(
        render::nm(entry.length),
        render::nm(entry.width),
        render::nm(entry.thickness),
    );
    let mut leave = f64::INFINITY;
    let mut leaving = 0.0_f64;
    for axis in PartAxis::ALL {
        let k = local_slot(axis);
        let d = direction[k];
        if d.abs() < 1e-12 {
            continue;
        }
        let limit = if d > 0.0 {
            render::nm(size.get(axis))
        } else {
            0.0
        };
        let t = (limit - origin[k]) / d;
        if t < leave {
            leave = t;
            leaving = d.abs();
        }
    }
    let radius = render::nm(diameter) / 2.0;
    let past = if leaving > 0.0 {
        radius * (1.0 - leaving * leaving).max(0.0).sqrt() / leaving
    } else {
        0.0
    };
    render::length_from_nm((leave.max(0.0) + past).ceil() + 1.0).unwrap_or(size.length)
}

/// The slanted hole a drill makes (grammar, "Slanted members"). On a
/// member, its axis is the shape's axis, which passes through the middle
/// of the shape's box; elsewhere, it enters at the middle of its place.
/// Where the axis meets the face's plane is rounded to the nanometre.
/// A `through` hole runs until it is surely out of the part.
pub(crate) fn slant_of(
    part: &Part,
    cut: &Cut,
    diameter: Length,
    depth: Depth,
    from_depth: Length,
    tilt: Tilt,
) -> Slant {
    let size = part.cut;
    let face = cut.face;
    let plane = match face.end {
        End::Start => Length::ZERO,
        End::End => size.get(face.axis),
    };
    let entry = match &cut.place {
        CutPlace::Member { shape, .. } => {
            let mut centre = Local::default();
            for axis in PartAxis::ALL {
                let (low, high) = (local_get(&shape.min, axis), local_get(&shape.max, axis));
                centre = centre.with(
                    axis,
                    Length::from_nm(low.nm().saturating_add(high.nm()).div_euclid(2)),
                );
            }
            // Slide along the axis from the middle to the face's plane:
            // across the face by the distance to it times tan(angle).
            let (sin, cos) = tilt.angle.sin_cos();
            let inward = match face.end {
                End::Start => 1_i128,
                End::End => -1,
            };
            let lean = match tilt.toward.end {
                End::Start => -1_i128,
                End::End => 1,
            };
            // Distance to go along the inward normal (negative: back out).
            let normal = i128::from(plane.nm()) - i128::from(centre.get(face.axis).nm());
            let along = normal * inward;
            let shift = if cos == 0 {
                0
            } else {
                along * sin / cos * lean
            };
            let across_value = i128::from(centre.get(tilt.toward.axis).nm()) + shift;
            centre.with(face.axis, plane).with(
                tilt.toward.axis,
                Length::from_nm(i64::try_from(across_value).unwrap_or(0)),
            )
        }
        CutPlace::End { .. }
        | CutPlace::Side { .. }
        | CutPlace::Crossing { .. }
        | CutPlace::Edge { .. } => place_centre(part, &cut.place, face.axis).with(face.axis, plane),
    };
    let end = match depth {
        Depth::Length(depth) => depth,
        Depth::Through => through_end(size, face, entry, tilt, diameter),
    };
    // A hole that opens onto its face starts outside it: its first disc
    // is square to its slanted axis, and the drill takes away the wedge
    // between that disc and the face too, `radius × tan(angle)` deep at
    // most.
    let start = if from_depth > Length::ZERO {
        from_depth.min(end)
    } else {
        let (sin, cos) = tilt.angle.sin_cos();
        let radius = i128::from(diameter.nm()) / 2 + 1;
        let wedge = if cos > 0 { radius * sin / cos + 1 } else { 0 };
        Length::from_nm(-i64::try_from(wedge).unwrap_or(0))
    };
    Slant {
        face,
        entry,
        tilt,
        diameter,
        start,
        end,
    }
}

fn local_get(point: &stave_model::solved::LocalPoint, axis: PartAxis) -> Length {
    match axis {
        PartAxis::Length => point.length,
        PartAxis::Width => point.width,
        PartAxis::Thickness => point.thickness,
    }
}

/// The extent of a cut's place along an axis of the face: a member's
/// whole shape (so a member's holes in two parts share their centres even
/// where the shape runs past the edge of one), the footprint's, or the
/// whole part's.
pub(crate) fn face_extent(part: &Part, place: &CutPlace, axis: PartAxis) -> (Length, Length) {
    let extent = match place {
        CutPlace::Member { shape, .. } => Some(shape),
        CutPlace::Crossing { footprint, .. } => Some(footprint),
        CutPlace::End { .. } | CutPlace::Side { .. } | CutPlace::Edge { .. } => None,
    };
    match extent {
        Some(extent) => (local_get(&extent.min, axis), local_get(&extent.max, axis)),
        None => (Length::ZERO, part.cut.get(axis)),
    }
}

/// The centre of a cut's place on its face (the face axis is left at 0).
pub(crate) fn place_centre(part: &Part, place: &CutPlace, face_axis: PartAxis) -> Local {
    let mut centre = Local::default();
    for axis in across(face_axis) {
        let (low, high) = face_extent(part, place, axis);
        let value = Length::from_nm(low.nm().saturating_add(high.nm()).div_euclid(2));
        centre = centre.with(axis, value);
    }
    centre
}

/// The part a cut is measured against.
fn entering_part<'d>(place: &CutPlace, design: &'d SolvedDesign) -> Option<&'d Part> {
    match place {
        CutPlace::Crossing { other, .. } => design.part(*other),
        CutPlace::End { toward } => design.part(*toward),
        CutPlace::Side { from, .. } => design.part(*from),
        CutPlace::Member { .. } | CutPlace::Edge { .. } => None,
    }
}

/// The local axes of this part that the entering part's width and
/// thickness lie along, both on the face; for a pocket.
pub(crate) fn entering_axes(
    part: &Part,
    place: &CutPlace,
    design: &SolvedDesign,
    face_axis: PartAxis,
) -> Option<(PartAxis, PartAxis)> {
    let other = entering_part(place, design)?;
    let local = |axis: PartAxis| {
        part.pose
            .orientation
            .part_axis(other.pose.orientation.direction(axis).axis())
    };
    let (width, thickness) = (local(PartAxis::Width), local(PartAxis::Thickness));
    let [first, second] = across(face_axis);
    let other_of = |axis: PartAxis| if axis == first { second } else { first };
    if width != face_axis && thickness != face_axis {
        Some((width, thickness))
    } else if width != face_axis {
        Some((width, other_of(width)))
    } else if thickness != face_axis {
        Some((other_of(thickness), thickness))
    } else {
        None
    }
}

/// The local axis a slot runs along and whether it runs toward the
/// positive end of it (for `from_centre`). `turns` is the slot's angle in
/// quarter turns.
pub(crate) fn slot_run(
    part: &Part,
    place: &CutPlace,
    design: &SolvedDesign,
    face_axis: PartAxis,
    turns: i64,
) -> Option<(PartAxis, bool)> {
    let other = entering_part(place, design)?;
    let orientation = other.pose.orientation;
    let face_world = part.pose.orientation.direction(face_axis).axis();
    let (base, next) = if orientation.length().axis() == face_world {
        (orientation.width(), orientation.thickness())
    } else if orientation.width().axis() == face_world {
        (orientation.length(), orientation.thickness())
    } else {
        (orientation.length(), orientation.width())
    };
    let direction = match turns {
        0 => base,
        1 => next,
        2 => base.opposite(),
        _ => next.opposite(),
    };
    let axis = part.pose.orientation.part_axis(direction.axis());
    if axis == face_axis {
        return None;
    }
    let along = part.pose.orientation.direction(axis);
    Some((axis, along == direction))
}

/// The local axes an angled slot's angle is measured from and toward
/// (see `slot_run`), each with whether its positive local direction is the
/// world direction the angle uses.
fn slot_axes(
    part: &Part,
    place: &CutPlace,
    design: &SolvedDesign,
    face_axis: PartAxis,
) -> Option<((PartAxis, bool), (PartAxis, bool))> {
    let other = entering_part(place, design)?;
    let orientation = other.pose.orientation;
    let face_world = part.pose.orientation.direction(face_axis).axis();
    let (base, next) = if orientation.length().axis() == face_world {
        (orientation.width(), orientation.thickness())
    } else if orientation.width().axis() == face_world {
        (orientation.length(), orientation.thickness())
    } else {
        (orientation.length(), orientation.width())
    };
    let local = |direction: stave_lang::placement::Direction| {
        let axis = part.pose.orientation.part_axis(direction.axis());
        (axis, part.pose.orientation.direction(axis) == direction)
    };
    let (base, next) = (local(base), local(next));
    (base.0 != face_axis && next.0 != face_axis).then_some((base, next))
}

fn plus(a: Length, b: Length) -> Length {
    Length::from_nm(a.nm().saturating_add(b.nm()))
}

fn minus(a: Length, b: Length) -> Length {
    Length::from_nm(a.nm().saturating_sub(b.nm()))
}

/// A slot turned in its face by an angle that isn't a multiple of 90deg
/// (the chidori's diagonal slots), before it becomes a polygon.
struct TurnedSlot {
    face_axis: PartAxis,
    /// The local axis the angle is measured from, and whether its positive
    /// direction is the one the angle starts from.
    base: (PartAxis, bool),
    /// The local axis the angle turns toward, likewise.
    next: (PartAxis, bool),
    angle: Angle,
    /// The footprint's centre (the face axis is left at 0).
    centre: Local,
    width: Length,
    /// From the centre out one side only (toward the angle).
    from_centre: bool,
}

impl TurnedSlot {
    /// The slot as a rectangle on the face, running out past the part's
    /// edges (by the sum of its sizes, which is more than any diagonal).
    /// Its corners are rounded to the nanometre.
    fn polygon(&self, size: Size3) -> Result<Polygon, ShapeError> {
        let [first, second] = across(self.face_axis);
        let unit = |(axis, positive): (PartAxis, bool)| {
            let sign = if positive { 1.0 } else { -1.0 };
            if axis == first {
                [sign, 0.0]
            } else if axis == second {
                [0.0, sign]
            } else {
                [0.0, 0.0]
            }
        };
        let (base, next) = (unit(self.base), unit(self.next));
        let radians = render::radians_f64(self.angle);
        let (cos, sin) = (radians.cos(), radians.sin());
        let u = [0, 1].map(|i| base[i] * cos + next[i] * sin);
        let v = [0, 1].map(|i| next[i] * cos - base[i] * sin);
        let centre = [
            render::nm(self.centre.get(first)),
            render::nm(self.centre.get(second)),
        ];
        let reach = PartAxis::ALL
            .into_iter()
            .map(|axis| render::nm(size.get(axis)))
            .sum::<f64>();
        let half = render::nm(self.width) / 2.0;
        let start = if self.from_centre { 0.0 } else { -reach };
        let corner = |along: f64, across: f64| {
            let at = [0, 1].map(|i| centre[i] + u[i] * along + v[i] * across);
            Some(Point2::new(
                render::length_from_nm(at[0])?,
                render::length_from_nm(at[1])?,
            ))
        };
        let corners = [
            corner(start, -half),
            corner(reach, -half),
            corner(reach, half),
            corner(start, half),
        ];
        let points: Vec<Point2> = corners
            .into_iter()
            .collect::<Option<_>>()
            .ok_or(ShapeError::Polygon(PolygonError::NoArea))?;
        Ok(Polygon::new(points)?)
    }
}

/// A dovetail on the end of a part, or a sliding dovetail on the face a
/// crossing part enters: the axis it runs into the part along, the axis
/// its sides lie across, and the axis it is cut straight through.
struct Dovetail {
    depth_axis: PartAxis,
    through: PartAxis,
    /// The face it is cut from, on the depth axis.
    face_at: Length,
    /// The bottom of the socket, or the tail's shoulder.
    inner: Length,
    /// The neck's sides on the width axis.
    neck_low: Length,
    neck_high: Length,
    /// How far each side spreads over the depth: `depth × tan(angle)`
    /// rounded to the nanometre, or `depth × ratio` for a slope `1 in n`
    /// (exact when it is a whole number of nanometres); `None` when the
    /// angle is too steep.
    offset: Option<Length>,
    /// The part's size on the width axis and through it.
    width: Length,
    thickness: Length,
}

impl Dovetail {
    /// A socket's sides lie across the entering part's width; a tail's
    /// across its own part's width. Both are centred across the face (on
    /// the footprint, for a sliding dovetail on `crossing`) and cut
    /// through the part's third axis.
    fn of(
        part: &Part,
        cut: &Cut,
        design: &SolvedDesign,
        neck: Length,
        depth: Length,
        slope: Slope,
    ) -> Dovetail {
        let depth_axis = cut.face.axis;
        let [first, second] = across(depth_axis);
        let preferred = match &cut.kind {
            CutKind::DovetailSocket { .. } => entering_part(&cut.place, design).map(|other| {
                part.pose
                    .orientation
                    .part_axis(other.pose.orientation.direction(PartAxis::Width).axis())
            }),
            CutKind::Notch { .. }
            | CutKind::Slot { .. }
            | CutKind::Pocket { .. }
            | CutKind::Tenon { .. }
            | CutKind::Spigot { .. }
            | CutKind::Round { .. }
            | CutKind::Drill { .. }
            | CutKind::DovetailTail { .. }
            | CutKind::Outline { .. }
            | CutKind::Diagonal
            | CutKind::Chamfer { .. }
            | CutKind::Roundover { .. } => Some(PartAxis::Width),
        };
        let width_axis = preferred.filter(|axis| *axis != depth_axis).unwrap_or(
            if first == PartAxis::Width || second == PartAxis::Width {
                PartAxis::Width
            } else {
                first
            },
        );
        let through = if width_axis == first { second } else { first };
        let size = part.cut;
        let (low, high) = depth_range(size, cut.face, Depth::Length(depth));
        let (face_at, inner) = match cut.face.end {
            End::Start => (low, high),
            End::End => (high, low),
        };
        let width = size.get(width_axis);
        let middle = place_centre(part, &cut.place, depth_axis).get(width_axis);
        let neck_low = centred_low(middle, neck);
        let run = minus(high, low);
        let offset = match slope {
            Slope::Angle(angle) => {
                let spread = render::nm(run) * render::radians_f64(angle).tan();
                render::length_from_nm(spread).filter(|_| spread.is_finite() && spread >= 0.0)
            }
            Slope::Ratio(ratio) => run
                .to_exact()
                .checked_mul(ratio)
                .and_then(|spread| spread.to_nearest_length()),
        };
        Dovetail {
            depth_axis,
            through,
            face_at,
            inner,
            neck_low,
            neck_high: plus(neck_low, neck),
            offset,
            width,
            thickness: size.get(through),
        }
    }

    /// A point given on the depth and width axes, in the plane across
    /// `through`.
    fn point(&self, along_depth: Length, along_width: Length) -> Point2 {
        let [first, _] = across(self.through);
        if first == self.depth_axis {
            Point2::new(along_depth, along_width)
        } else {
            Point2::new(along_width, along_depth)
        }
    }

    fn offset(&self) -> Result<Length, ShapeError> {
        self.offset.ok_or(ShapeError::TooSteep)
    }

    fn range(&self) -> (Length, Length) {
        (Length::ZERO, self.thickness)
    }

    /// The socket: `neck` wide at the end face, wider by the offset each
    /// side at its bottom.
    fn socket(&self) -> Result<Marked, ShapeError> {
        let offset = self.offset()?;
        let polygon = Polygon::new(vec![
            self.point(self.face_at, self.neck_low),
            self.point(self.face_at, self.neck_high),
            self.point(self.inner, plus(self.neck_high, offset)),
            self.point(self.inner, minus(self.neck_low, offset)),
        ])?;
        Ok((self.through, polygon, self.range()))
    }

    /// What is cut away around the tail: the wedges either side of it,
    /// between its shoulder and the end. The tail is `neck` wide at its
    /// shoulder and wider by the offset each side at the end. A wedge
    /// runs out past the part's edge where the tail is wider than the
    /// part, which cuts nothing more.
    fn wedges(&self) -> Result<Vec<Marked>, ShapeError> {
        let offset = self.offset()?;
        let mut out = Vec::new();
        if self.neck_low > Length::ZERO {
            let flank = minus(self.neck_low, offset);
            let outer = flank.min(Length::ZERO);
            let points = vec![
                self.point(self.inner, outer),
                self.point(self.inner, self.neck_low),
                self.point(self.face_at, flank),
                self.point(self.face_at, outer),
            ];
            out.push((self.through, Polygon::new(distinct(points))?, self.range()));
        }
        if self.neck_high < self.width {
            let flank = plus(self.neck_high, offset);
            let outer = flank.max(self.width);
            let points = vec![
                self.point(self.inner, self.neck_high),
                self.point(self.inner, outer),
                self.point(self.face_at, outer),
                self.point(self.face_at, flank),
            ];
            out.push((self.through, Polygon::new(distinct(points))?, self.range()));
        }
        Ok(out)
    }
}

/// A polygon marked on the plane across `axis`, cut over `range` of it.
type Marked = (PartAxis, Polygon, (Length, Length));

/// A ring of points with each point that repeats the one before dropped.
fn distinct(points: Vec<Point2>) -> Vec<Point2> {
    let mut out: Vec<Point2> = Vec::with_capacity(points.len());
    for point in points {
        if out.last() != Some(&point) && out.first() != Some(&point) {
            out.push(point);
        }
    }
    out
}

/// An outline's marks as a polygon on its face: `along` from the end the
/// selector measures from, `across` from the reference edge (or face).
fn outline_polygon(part: &Part, cut: &Cut, marks: &[Mark]) -> Result<Polygon, PolygonError> {
    let size = part.cut;
    let [first, second] = across(cut.face.axis);
    let (along_axis, along_from, across_axis, across_from) = match &cut.place {
        CutPlace::Side {
            along_from,
            across_axis,
            across_from,
            ..
        } => (PartAxis::Length, *along_from, *across_axis, *across_from),
        CutPlace::End { .. }
        | CutPlace::Crossing { .. }
        | CutPlace::Member { .. }
        | CutPlace::Edge { .. } => (first, End::Start, second, End::Start),
    };
    let measured = |axis: PartAxis, from: End, value: Length| match from {
        End::Start => value,
        End::End => minus(size.get(axis), value),
    };
    let points = marks
        .iter()
        .map(|mark| {
            let local = Local::default()
                .with(along_axis, measured(along_axis, along_from, mark.along))
                .with(across_axis, measured(across_axis, across_from, mark.across));
            Point2::new(local.get(first), local.get(second))
        })
        .collect();
    Polygon::new(points)
}

/// The range an outline covers along its face's axis: from `from_depth`
/// below the face down to `depth`.
fn outline_range(
    size: Size3,
    face: LocalFace,
    depth: Depth,
    from_depth: Length,
) -> (Length, Length) {
    let full = size.get(face.axis);
    let to = match depth {
        Depth::Through => full,
        Depth::Length(depth) => depth.min(full).max(Length::ZERO),
    };
    let from = from_depth.min(to).max(Length::ZERO);
    match face.end {
        End::Start => (from, to),
        End::End => (minus(full, to), minus(full, from)),
    }
}

/// The third of a part's axes, besides two different ones.
fn third_axis(a: PartAxis, b: PartAxis) -> PartAxis {
    PartAxis::ALL
        .into_iter()
        .find(|axis| *axis != a && *axis != b)
        .unwrap_or(PartAxis::Thickness)
}

/// A point on the plane across `through`, from its coordinates along two
/// other axes.
fn plane_point(through: PartAxis, values: [(PartAxis, Length); 2]) -> Point2 {
    let [first, second] = across(through);
    let mut local = Local::default();
    for (axis, value) in values {
        local = local.with(axis, value);
    }
    Point2::new(local.get(first), local.get(second))
}

/// A `diagonal` (grammar, "Cut operations"): where this part and the other
/// cross at a corner, the triangle of their overlap at this part's end,
/// beyond the line from the outside corner (at the end of both) to the
/// inside corner. Cut straight through along the axis square to both
/// parts' lengths. The other part's diagonal takes the other half, so the
/// two meet on the line.
fn diagonal_polygon(
    part: &Part,
    cut: &Cut,
    design: &SolvedDesign,
) -> Result<(PartAxis, Polygon), ShapeError> {
    let CutPlace::Crossing { other, footprint } = &cut.place else {
        return Err(ShapeError::DiagonalNotAtCorner);
    };
    let other = design.part(*other).ok_or(ShapeError::DiagonalNotAtCorner)?;
    let run = part
        .pose
        .orientation
        .part_axis(other.pose.orientation.direction(PartAxis::Length).axis());
    if run == PartAxis::Length {
        return Err(ShapeError::DiagonalParallel);
    }
    let through = third_axis(PartAxis::Length, run);
    let (l0, l1) = (footprint.min.length, footprint.max.length);
    let (c0, c1) = (
        local_get(&footprint.min, run),
        local_get(&footprint.max, run),
    );
    if l0 >= l1 || c0 >= c1 {
        return Err(ShapeError::DiagonalNotAtCorner);
    }
    // This part's end: the footprint reaches one end of its length, not
    // both.
    let length = part.cut.length;
    let (outer_l, inner_l) = match (l0 == Length::ZERO, l1 == length) {
        (true, false) => (l0, l1),
        (false, true) => (l1, l0),
        (true, true) | (false, false) => return Err(ShapeError::DiagonalNotAtCorner),
    };
    // The other part's end, along `run` in this part's frame: its cut box
    // stops at one side of the footprint and runs on past the other.
    let direction = part.pose.orientation.direction(run);
    let world = direction.axis();
    let other_box = other.cut_box();
    let origin = part.pose.origin.get(world).nm();
    let local = |value: Length| {
        let offset = value.nm().saturating_sub(origin);
        Length::from_nm(if direction.is_positive() {
            offset
        } else {
            offset.saturating_neg()
        })
    };
    let (a, b) = (
        local(other_box.min().get(world)),
        local(other_box.max().get(world)),
    );
    let (qa, qb) = (a.min(b), a.max(b));
    let (outer_c, inner_c) = match (qa == c0 && qb > c1, qb == c1 && qa < c0) {
        (true, false) => (c0, c1),
        (false, true) => (c1, c0),
        (true, true) | (false, false) => return Err(ShapeError::DiagonalNotAtCorner),
    };
    let point = |l: Length, c: Length| plane_point(through, [(PartAxis::Length, l), (run, c)]);
    let polygon = Polygon::new(vec![
        point(outer_l, outer_c),
        point(outer_l, inner_c),
        point(inner_l, inner_c),
    ])?;
    Ok((through, polygon))
}

/// The cross-section of an edge profile.
#[derive(Clone, Copy)]
enum EdgeShape {
    /// A chamfer this far back along each face.
    Chamfer(Length),
    /// A roundover of this radius.
    Round(Length),
}

/// How many flats a roundover's quarter circle is drawn and checked with.
const ROUND_STEPS: i64 = 12;

/// An edge profile (grammar, "Edge profiles"): the wood between the edge
/// and the profile, on the plane across the edge, along the edge's whole
/// drawn length. Both are measured on the part's drawn box, so what joints
/// add past it is left square. A roundover's quarter circle is drawn as
/// flats that touch the circle: a little more wood than the true curve,
/// never less, and each corner is rounded once to the nanometre.
fn edge_profile(
    part: &Part,
    cut: &Cut,
    shape: EdgeShape,
) -> Result<(PartAxis, Polygon, (Length, Length)), ShapeError> {
    let CutPlace::Edge { other } = cut.place else {
        return Err(ShapeError::Polygon(PolygonError::TooFewPoints { count: 0 }));
    };
    let faces = [cut.face, other];
    let run = third_axis(cut.face.axis, other.axis);
    // The corner, and which way is into the part, along each face's axis.
    let corner = faces.map(|face| {
        let start = part.drawn_offset.get(face.axis);
        match face.end {
            End::Start => (face.axis, start, 1i64),
            End::End => (
                face.axis,
                start
                    .checked_add(part.drawn.get(face.axis))
                    .unwrap_or(start),
                -1i64,
            ),
        }
    });
    // A point `into[0]` in from the first face and `into[1]` from the
    // second.
    let point = |into: [i64; 2]| {
        let [(a, a0, a_sign), (b, b0, b_sign)] = corner;
        plane_point(
            run,
            [
                (
                    a,
                    Length::from_nm(a0.nm().saturating_add(into[0].saturating_mul(a_sign))),
                ),
                (
                    b,
                    Length::from_nm(b0.nm().saturating_add(into[1].saturating_mul(b_sign))),
                ),
            ],
        )
    };
    let mut points = vec![point([0, 0])];
    match shape {
        EdgeShape::Chamfer(back) => {
            points.push(point([back.nm(), 0]));
            points.push(point([0, back.nm()]));
        }
        EdgeShape::Round(radius) => {
            // Corners of the flats, each `radius / cos(half a step)` from
            // the centre, half a step either side of where a flat touches
            // the circle. The first and last lie on the faces.
            let quarter = i128::from(Angle::from_deg(90).microdeg());
            let steps = i128::from(ROUND_STEPS);
            let (_, half_cos) =
                Angle::from_microdeg(i64::try_from(quarter / (2 * steps)).unwrap_or(0)).sin_cos();
            let r = i128::from(radius.nm());
            for index in 0..steps {
                let angle = quarter * (2 * index + 1) / (2 * steps);
                let (sin, cos) = Angle::from_microdeg(i64::try_from(angle).unwrap_or(0)).sin_cos();
                let along = |trig: i128| {
                    let reach = r.saturating_mul(trig).checked_div(half_cos).unwrap_or(r);
                    i64::try_from(r.saturating_sub(reach)).unwrap_or(0).max(0)
                };
                points.push(point([along(cos), along(sin)]));
            }
        }
    }
    let polygon = Polygon::new(distinct(points))?;
    let start = part.drawn_offset.get(run);
    let end = start.checked_add(part.drawn.get(run)).unwrap_or(start);
    Ok((run, polygon, (start, end)))
}

/// Where a `drill`'s hole goes, on the two axes across the face's: the
/// middle of its footprint, or of the member's shape it is drilled for.
pub(crate) fn hole_centre(part: &Part, cut: &Cut) -> [Length; 2] {
    let middle = |(low, high): (Length, Length)| {
        Length::from_nm(
            i64::try_from((i128::from(low.nm()) + i128::from(high.nm())).div_euclid(2))
                .unwrap_or(low.nm()),
        )
    };
    across(cut.face.axis).map(|axis| middle(face_extent(part, &cut.place, axis)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mm(value: i64) -> Length {
        Length::from_mm(value)
    }

    fn local(l: i64, w: i64, t: i64) -> Local {
        Size3 {
            length: mm(l),
            width: mm(w),
            thickness: mm(t),
        }
    }

    #[test]
    fn a_box_minus_a_box_is_disjoint_and_exact() {
        let board = Block::new(local(0, 0, 0), local(100, 50, 20)).unwrap();
        let mut blocks = Blocks::new(board);
        let pocket = Block::new(local(40, 10, 0), local(60, 40, 12)).unwrap();
        blocks.subtract(&pocket);
        let total = 100 * 50 * 20 - 20 * 30 * 12;
        assert_eq!(blocks.volume(), i128::from(total) * 1_000_000_i128.pow(3));
        for (i, a) in blocks.iter().enumerate() {
            for b in blocks.iter().skip(i + 1) {
                assert!(a.intersection(b).is_none(), "{a:?} overlaps {b:?}");
            }
            assert!(a.intersection(&pocket).is_none());
        }
        // Cutting what is already gone changes nothing.
        let before = blocks.clone();
        blocks.subtract(&pocket);
        assert_eq!(blocks, before);
    }

    #[test]
    fn empty_and_flat_blocks_are_not_blocks() {
        assert!(Block::new(local(0, 0, 0), local(10, 0, 5)).is_none());
        assert!(Block::new(local(5, 0, 0), local(1, 1, 1)).is_none());
    }

    #[test]
    fn narrowing_a_profile_makes_shoulders() {
        let mut profile = Profile {
            stations: vec![
                ProfileStation {
                    at: mm(0),
                    diameter: mm(34),
                },
                ProfileStation {
                    at: mm(100),
                    diameter: mm(34),
                },
            ],
            smooth: false,
            narrowed: Vec::new(),
        };
        profile.narrow(mm(80), mm(100), mm(22));
        let diameters: Vec<(i64, i64)> = profile
            .stations
            .iter()
            .map(|s| (s.at.nm() / 1_000_000, s.diameter.nm() / 1_000_000))
            .collect();
        assert_eq!(diameters, vec![(0, 34), (80, 34), (80, 22), (100, 22)]);
        let rods = profile.rods([mm(17), mm(17)]);
        assert_eq!(rods.len(), 2);
        assert_eq!(rods[1].diameter, mm(22));
    }
}
