//! Whether a part is still one piece of wood once every cut is made
//! (grammar, "Cuts stay in their part").
//!
//! Each cut on its own may leave wood behind it, yet two cuts from
//! opposite faces that meet, or one cut that goes `through` right across
//! the part, leave it in two. The wood of a board is its solid's disjoint
//! [`Blocks`]; two blocks are the same piece of wood when they share a
//! face with an area (blocks that touch only along an edge or at a corner
//! are two pieces). A round section or spigot turned on a board (a rod
//! between or beyond the blocks) joins what it touches; it is taken as
//! the box round it, which only ever finds more wood than there is.
//! Angled cuts' slanted pieces and holes are left out: they only ever
//! remove wood. So the test never finds a part in pieces that isn't.
//!
//! Round stock and turned parts are not tested (their wood is a lathe
//! profile, not blocks), and the cuts between the boards of a part made of
//! boards are left out: the part is tested as one board with its own cuts.

use stave_lang::placement::PartAxis;
use stave_model::solved::{Part, SolvedDesign};

use stave_lang::units::Length;

use crate::solid::{Block, Local, Rod, Solid, across, half};

/// A part its cuts leave in more than one piece.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Severed {
    /// How many pieces of wood are left (at least two).
    pub pieces: usize,
    /// The cut (an index into the part's cuts) that, made after those
    /// before it, first leaves the part in pieces.
    pub cut: usize,
    /// An earlier cut it meets: without this one, `cut` would leave the
    /// part whole. `None` when `cut` cuts the part in two on its own.
    pub meets: Option<usize>,
}

/// Are two boxes one piece of wood: do they share a volume, or a face
/// with an area? (Overlapping on every axis but one, where they touch.)
fn joined(a: &Block, b: &Block) -> bool {
    let mut touching = 0usize;
    for axis in PartAxis::ALL {
        let low = a.min().get(axis).max(b.min().get(axis));
        let high = a.max().get(axis).min(b.max().get(axis));
        if high < low {
            return false;
        }
        if high == low {
            touching += 1;
        }
    }
    touching <= 1
}

/// The box round a rod.
fn rod_box(rod: &Rod) -> Option<Block> {
    let radius = half(rod.diameter);
    let mut min = Local::default().with(rod.axis, rod.start);
    let mut max = Local::default().with(rod.axis, rod.end);
    for (axis, centre) in across(rod.axis).into_iter().zip(rod.centre) {
        min = min.with(axis, Length::from_nm(centre.nm().checked_sub(radius.nm())?));
        max = max.with(axis, Length::from_nm(centre.nm().checked_add(radius.nm())?));
    }
    Block::new(min, max)
}

/// How many separate pieces of wood a board's solid is (0 when there is
/// no wood at all).
pub fn pieces(solid: &Solid) -> usize {
    let boxes: Vec<Block> = solid
        .blocks()
        .iter()
        .copied()
        .chain(solid.rods().iter().filter_map(rod_box))
        .collect();
    pieces_of(&boxes)
}

/// The root of `index` in a union–find forest, halving the path.
fn root(parents: &mut [usize], mut index: usize) -> usize {
    while let Some(&parent) = parents.get(index) {
        if parent == index {
            break;
        }
        let grandparent = parents.get(parent).copied().unwrap_or(parent);
        if let Some(slot) = parents.get_mut(index) {
            *slot = grandparent;
        }
        index = grandparent;
    }
    index
}

/// How many separate pieces of wood boxes make.
fn pieces_of(blocks: &[Block]) -> usize {
    let mut parents: Vec<usize> = (0..blocks.len()).collect();
    let mut count = blocks.len();
    for (i, a) in blocks.iter().enumerate() {
        for (j, b) in blocks.iter().enumerate().skip(i + 1) {
            if joined(a, b) {
                let (ra, rb) = (root(&mut parents, i), root(&mut parents, j));
                if ra != rb {
                    if let Some(slot) = parents.get_mut(rb) {
                        *slot = ra;
                    }
                    count = count.saturating_sub(1);
                }
            }
        }
    }
    count
}

/// How many pieces the part is in with only `cuts` made.
fn pieces_with(
    trial: &mut Part,
    cuts: impl Iterator<Item = usize>,
    part: &Part,
    design: &SolvedDesign,
) -> usize {
    trial.cuts.clear();
    trial
        .cuts
        .extend(cuts.filter_map(|index| part.cuts.get(index).cloned()));
    pieces(&Solid::of(trial, design))
}

/// The part in pieces, if its own cuts leave it so; `solid` is its solid
/// with every cut made. Only when that is in pieces are the cuts made
/// again, a few at a time, to find the one that parts it and the cut it
/// meets.
///
/// A part made of boards is tested as one board: its boards are glued
/// along their seams, and their widths, each rounded to the nanometre,
/// may leave a nanometre between two of them in the solid.
pub fn severed(part: &Part, design: &SolvedDesign, solid: &Solid) -> Option<Severed> {
    if solid.profile().is_some() || pieces(solid) <= 1 {
        return None;
    }
    let mut trial = Part {
        cuts: Vec::with_capacity(part.cuts.len()),
        boards: None,
        ..part.clone()
    };
    let every = part.cuts.len();
    let count = pieces_with(&mut trial, 0..every, part, design);
    if count <= 1 {
        return None;
    }
    let cut = (0..every).find(|&last| pieces_with(&mut trial, 0..=last, part, design) > 1)?;
    let meets = (0..cut).find(|&left_out| {
        pieces_with(
            &mut trial,
            (0..=cut).filter(|index| *index != left_out),
            part,
            design,
        ) <= 1
    });
    Some(Severed {
        pieces: count,
        cut,
        meets,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::solid::Blocks;
    use stave_model::solved::Size3;

    fn block(min: [i64; 3], max: [i64; 3]) -> Block {
        let point = |[length, width, thickness]: [i64; 3]| Size3 {
            length: Length::from_mm(length),
            width: Length::from_mm(width),
            thickness: Length::from_mm(thickness),
        };
        Block::new(point(min), point(max)).unwrap()
    }

    fn count(blocks: &Blocks) -> usize {
        pieces_of(&blocks.iter().copied().collect::<Vec<_>>())
    }

    #[test]
    fn two_cuts_from_both_faces_that_meet_leave_two_pieces() {
        let mut wood = Blocks::new(block([0, 0, 0], [600, 300, 19]));
        // Cut from one face to half way, and from the other to half way,
        // at the same place along the length.
        wood.subtract(&block([300, 0, 0], [319, 300, 10]));
        assert_eq!(count(&wood), 1);
        wood.subtract(&block([300, 0, 10], [319, 300, 19]));
        assert_eq!(count(&wood), 2);
    }

    #[test]
    fn a_cut_stopped_short_of_an_edge_keeps_one_piece() {
        let mut wood = Blocks::new(block([0, 0, 0], [600, 300, 19]));
        wood.subtract(&block([300, 0, 0], [319, 290, 19]));
        assert_eq!(count(&wood), 1);
    }

    #[test]
    fn blocks_touching_only_along_an_edge_are_two_pieces() {
        let mut wood = Blocks::new(block([0, 0, 0], [10, 10, 10]));
        wood.add(block([10, 10, 0], [20, 20, 10]));
        assert_eq!(count(&wood), 2);
        assert_eq!(count(&Blocks::empty()), 0);
    }
}
