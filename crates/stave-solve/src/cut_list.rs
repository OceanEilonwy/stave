//! The cut list (grammar, "Cut list"): every piece of wood to cut, at the
//! size to cut it and the rough size to cut it from the board at, how many
//! boards of each stock to buy, and how much edge banding.
//!
//! It is derived from a [`SolvedDesign`], never stored. Every size is
//! worked out exactly and rounded once, here, to the project's
//! `resolution:`.

use std::collections::BTreeMap;

use stave_lang::placement::PartAxis;
use stave_lang::units::{ExactLength, Length, Resolution, Rounded};
use stave_model::ids::PartId;
use stave_model::solved::{Banding, Part, PartOrigin, Size3, SolvedDesign, StockSection};
use stave_schema::words::{Grain, StockForm};

/// Why a cut list can't be made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CutListError {
    /// A size too large to work out.
    #[error("a size in the cut list is too large to work out")]
    Overflow,
}

/// A size, length by width by thickness in the part's own axes, each
/// rounded once to the project's resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoundedSize {
    /// Along the part's length.
    pub length: Rounded,
    /// Along the part's width.
    pub width: Rounded,
    /// Through the part's thickness.
    pub thickness: Rounded,
}

impl RoundedSize {
    /// The three sizes, length first.
    pub fn each(&self) -> [Rounded; 3] {
        [self.length, self.width, self.thickness]
    }
}

/// One piece of wood to cut.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The part it is (the part made of boards, for a board's row).
    pub part: PartId,
    /// Its name: the part's, or `top.board[2]` for a board of a part made
    /// of boards.
    pub name: String,
    /// The stock it is cut from.
    pub stock: String,
    /// The stock's form.
    pub form: StockForm,
    /// The size to cut it to: its cut size, less its edge banding.
    pub cut: RoundedSize,
    /// The size to first cut it from the board at, before milling: the cut
    /// size with the rough allowances and the bought thickness for solid
    /// stock, the cut size for a sheet or a dowel.
    pub rough: RoundedSize,
    /// Its edge banding.
    pub banding: Option<Banding>,
    /// The part it is the mirror image of.
    pub mirror_of: Option<PartId>,
}

/// How many boards of a stock to buy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StockToBuy {
    /// The stock.
    pub stock: String,
    /// Its form.
    pub form: StockForm,
    /// The rows cut from it.
    pub pieces: usize,
    /// Its `max_length`, when it says.
    pub max_length: Option<Length>,
    /// The kerf left between pieces.
    pub kerf: Length,
    /// How many boards to buy: `None` for a sheet (not laid out), or a
    /// stock with no `max_length`.
    pub boards: Option<u32>,
}

/// How much edge banding of one thickness to buy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BandingToBuy {
    /// The band's thickness.
    pub thickness: Length,
    /// The length of every edge banded with it.
    pub length: Rounded,
}

/// The cut list of a design.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CutList {
    /// What the sizes are rounded to.
    pub resolution: Resolution,
    /// Every piece to cut, in the order of the design's parts.
    pub rows: Vec<Row>,
    /// What to buy of each stock, by stock name.
    pub stocks: Vec<StockToBuy>,
    /// The edge banding to buy, thinnest first.
    pub banding: Vec<BandingToBuy>,
}

/// The exact size of a piece, by the part's own axes.
#[derive(Debug, Clone, Copy)]
struct ExactSize([ExactLength; 3]);

impl ExactSize {
    fn of(size: Size3) -> ExactSize {
        ExactSize([
            size.length.to_exact(),
            size.width.to_exact(),
            size.thickness.to_exact(),
        ])
    }

    fn get(&self, axis: PartAxis) -> ExactLength {
        let [length, width, thickness] = self.0;
        match axis {
            PartAxis::Length => length,
            PartAxis::Width => width,
            PartAxis::Thickness => thickness,
        }
    }

    fn set(&mut self, axis: PartAxis, value: ExactLength) {
        let [length, width, thickness] = &mut self.0;
        *match axis {
            PartAxis::Length => length,
            PartAxis::Width => width,
            PartAxis::Thickness => thickness,
        } = value;
    }

    fn add(&mut self, axis: PartAxis, extra: ExactLength) -> Result<(), CutListError> {
        let value = self
            .get(axis)
            .checked_add(extra)
            .ok_or(CutListError::Overflow)?;
        self.set(axis, value);
        Ok(())
    }

    fn round(&self, resolution: Resolution) -> Result<RoundedSize, CutListError> {
        let round = |value: ExactLength| value.round_to(resolution).ok_or(CutListError::Overflow);
        let [length, width, thickness] = self.0;
        Ok(RoundedSize {
            length: round(length)?,
            width: round(width)?,
            thickness: round(thickness)?,
        })
    }
}

/// The axis a part's grain runs along.
fn grain_axis(grain: Grain) -> PartAxis {
    match grain {
        Grain::Length => PartAxis::Length,
        Grain::Width => PartAxis::Width,
    }
}

/// The axis across the grain in the part's face.
fn across_axis(grain: Grain) -> PartAxis {
    match grain {
        Grain::Length => PartAxis::Width,
        Grain::Width => PartAxis::Length,
    }
}

/// A row's piece before rounding: its name, cut and rough sizes.
struct Piece {
    name: String,
    cut: ExactSize,
    rough: ExactSize,
}

/// The pieces a part is cut as: itself, or one per board.
fn pieces(part: &Part) -> Result<Vec<Piece>, CutListError> {
    let mut cut = ExactSize::of(part.cut);
    // The band is glued on after cutting: take it off once per banded
    // face (grammar, "Edge banding").
    if let Some(banding) = &part.banding {
        for face in &banding.faces {
            let value = cut
                .get(face.axis)
                .checked_sub(banding.thickness.to_exact())
                .ok_or(CutListError::Overflow)?;
            cut.set(face.axis, value);
        }
    }
    let mut out = Vec::new();
    match &part.boards {
        Some(boards) => {
            for board in boards.boards.iter() {
                let mut size = cut;
                size.set(boards.axis, board.width.to_exact());
                out.push((format!("{}.board[{}]", part.name, board.index.get()), size));
            }
        }
        None => out.push((part.name.clone(), cut)),
    }
    let allowances = part.stock.allowances;
    out.into_iter()
        .map(|(name, cut)| {
            let mut rough = cut;
            if part.stock.form == StockForm::Solid {
                rough.add(grain_axis(part.grain), allowances.rough_length.to_exact())?;
                rough.add(across_axis(part.grain), allowances.rough_width.to_exact())?;
                let bought = match (allowances.sold, part.stock.section) {
                    (Some(sold), _) => Some(sold),
                    (None, StockSection::Board { thickness, .. }) => Some(thickness),
                    (None, StockSection::Dowel { .. }) => None,
                };
                if let Some(bought) = bought {
                    let bought = bought.to_exact();
                    if bought > rough.get(PartAxis::Thickness) {
                        rough.set(PartAxis::Thickness, bought);
                    }
                }
            }
            Ok(Piece { name, cut, rough })
        })
        .collect()
}

/// How many boards `max_length` long the pieces take, laid end to end,
/// longest first, each into the first board it still fits in, with `kerf`
/// between neighbours. `None` when a piece is longer than a board.
fn boards_needed(
    mut lengths: Vec<ExactLength>,
    max_length: ExactLength,
    kerf: ExactLength,
) -> Result<Option<u32>, CutListError> {
    lengths.sort_by(|a, b| b.cmp(a));
    // How much of each board is taken, kerfs included.
    let mut boards: Vec<ExactLength> = Vec::new();
    for length in lengths {
        if length > max_length {
            return Ok(None);
        }
        let mut placed = false;
        for used in &mut boards {
            let after = used
                .checked_add(kerf)
                .and_then(|value| value.checked_add(length))
                .ok_or(CutListError::Overflow)?;
            if after <= max_length {
                *used = after;
                placed = true;
                break;
            }
        }
        if !placed {
            boards.push(length);
        }
    }
    u32::try_from(boards.len())
        .map(Some)
        .map_err(|_| CutListError::Overflow)
}

/// What is gathered for one stock while the rows are made.
struct Tally {
    form: StockForm,
    max_length: Option<Length>,
    kerf: Length,
    lengths: Vec<ExactLength>,
}

/// The cut list of a solved design.
pub fn cut_list(design: &SolvedDesign) -> Result<CutList, CutListError> {
    let resolution = Resolution::new(design.project.resolution).unwrap_or(Resolution::DEFAULT);
    let mut rows = Vec::new();
    let mut tallies: BTreeMap<String, Tally> = BTreeMap::new();
    let mut banding: BTreeMap<Length, ExactLength> = BTreeMap::new();
    for part in design.parts() {
        let mirror_of = match part.origin {
            PartOrigin::Mirror { source } | PartOrigin::MirroredAssembly { source } => Some(source),
            PartOrigin::Declared
            | PartOrigin::Made { .. }
            | PartOrigin::Piece { .. }
            | PartOrigin::Boards { .. } => None,
        };
        let tally = tallies
            .entry(part.stock.name.clone())
            .or_insert_with(|| Tally {
                form: part.stock.form,
                max_length: part.stock.max_length,
                kerf: part.stock.allowances.kerf,
                lengths: Vec::new(),
            });
        for piece in pieces(part)? {
            tally.lengths.push(piece.rough.get(grain_axis(part.grain)));
            rows.push(Row {
                part: part.id,
                name: piece.name,
                stock: part.stock.name.clone(),
                form: part.stock.form,
                cut: piece.cut.round(resolution)?,
                rough: piece.rough.round(resolution)?,
                banding: part.banding.clone(),
                mirror_of,
            });
        }
        if let Some(band) = &part.banding {
            let size = ExactSize::of(part.cut);
            let total = banding.entry(band.thickness).or_insert(ExactLength::ZERO);
            for face in &band.faces {
                // A face at an end of the length runs across the width,
                // and the other way round.
                let along = match face.axis {
                    PartAxis::Length => PartAxis::Width,
                    PartAxis::Width | PartAxis::Thickness => PartAxis::Length,
                };
                *total = total
                    .checked_add(size.get(along))
                    .ok_or(CutListError::Overflow)?;
            }
        }
    }
    let mut stocks = Vec::with_capacity(tallies.len());
    for (stock, tally) in tallies {
        let boards = match (tally.form, tally.max_length) {
            (StockForm::Solid | StockForm::Dowel, Some(max_length)) => boards_needed(
                tally.lengths.clone(),
                max_length.to_exact(),
                tally.kerf.to_exact(),
            )?,
            (StockForm::Plywood | StockForm::Mdf, _) | (_, None) => None,
        };
        stocks.push(StockToBuy {
            stock,
            form: tally.form,
            pieces: tally.lengths.len(),
            max_length: tally.max_length,
            kerf: tally.kerf,
            boards,
        });
    }
    let banding = banding
        .into_iter()
        .map(|(thickness, length)| {
            Ok(BandingToBuy {
                thickness,
                length: length.round_to(resolution).ok_or(CutListError::Overflow)?,
            })
        })
        .collect::<Result<Vec<_>, CutListError>>()?;
    Ok(CutList {
        resolution,
        rows,
        stocks,
        banding,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use stave_lang::units::Rational;

    fn mm(value: i64) -> ExactLength {
        Length::from_mm(value).to_exact()
    }

    #[test]
    fn four_450mm_parts_need_two_1800mm_boards_with_a_kerf() {
        let parts = vec![mm(450); 4];
        assert_eq!(
            boards_needed(parts.clone(), mm(1800), ExactLength::ZERO),
            Ok(Some(1))
        );
        assert_eq!(boards_needed(parts, mm(1800), mm(3)), Ok(Some(2)));
    }

    #[test]
    fn longest_first_into_the_first_board_it_fits() {
        // 1000 + 3 + 700 = 1703 in the first; 900 + 3 + 600 = 1503 in the
        // second; 200 back into the first: 1703 + 3 + 200 = 1906 > 1800,
        // into the second.
        let parts = vec![mm(600), mm(1000), mm(200), mm(900), mm(700)];
        assert_eq!(boards_needed(parts, mm(1800), mm(3)), Ok(Some(2)));
        assert_eq!(boards_needed(vec![mm(1900)], mm(1800), mm(3)), Ok(None));
        assert_eq!(boards_needed(Vec::new(), mm(1800), mm(3)), Ok(Some(0)));
    }

    #[test]
    fn a_third_is_rounded_once() {
        let third = Length::from_mm(100)
            .to_exact()
            .checked_div(Rational::from_int(3))
            .unwrap();
        let mut size = ExactSize([third, mm(10), mm(19)]);
        size.add(PartAxis::Length, third).unwrap();
        let rounded = size.round(Resolution::DEFAULT).unwrap();
        // 66.666...mm to the half millimetre: 66.5mm, not 33.5mm twice.
        assert_eq!(rounded.length.length, Length::from_nm(66_500_000));
        assert_ne!(rounded.length.moved_by, ExactLength::ZERO);
    }
}
