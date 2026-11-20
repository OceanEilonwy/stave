//! `stave cut-list`: the cut list as a plain table (grammar, "Cut list").

use std::fmt::Write as _;

use stave_lang::units::{ExactLength, Rounded};
use stave_model::solved::{Banding, End, SolvedDesign};
use stave_model::value::format_length;
use stave_schema::words::StockForm;
use stave_solve::cut_list::{CutList, RoundedSize};

use crate::dump::mm;

/// A rounded size, marked `*` when rounding moved it.
fn rounded(value: Rounded) -> String {
    if value.moved_by == ExactLength::ZERO {
        mm(value.length)
    } else {
        format!("{}*", mm(value.length))
    }
}

fn size(size: &RoundedSize) -> String {
    let [length, width, thickness] = size.each().map(rounded);
    format!("{length} x {width} x {thickness}")
}

/// The banded faces as they face once the part is placed.
fn banding(design: &SolvedDesign, part: stave_model::ids::PartId, band: &Banding) -> String {
    let Some(part) = design.part(part) else {
        return String::new();
    };
    let faces: Vec<&str> = band
        .faces
        .iter()
        .map(|face| {
            let direction = part.pose.orientation.direction(face.axis);
            let direction = match face.end {
                End::End => direction,
                End::Start => direction.opposite(),
            };
            direction.face().word()
        })
        .collect();
    format!("{} on {}", band.thickness, faces.join(", "))
}

/// The cut list as text: the rows, then what to buy.
pub fn table(design: &SolvedDesign, list: &CutList) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "Cut list: sizes in mm, length x width x thickness, rounded to {}mm (* where rounding moved a size)",
        mm(list.resolution.step())
    );
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "{:<28} {:<14} {:<26} {:<26} banding",
        "part", "stock", "cut to", "rough"
    );
    let moved: Vec<String> = list
        .rows
        .iter()
        .flat_map(|row| {
            [("cut", &row.cut), ("rough", &row.rough)]
                .into_iter()
                .filter(|(_, size)| size.each().iter().any(|v| v.moved_by != ExactLength::ZERO))
                .map(|(which, size)| {
                    let by: Vec<String> = size
                        .each()
                        .iter()
                        .map(|v| format_length(v.moved_by))
                        .collect();
                    format!("{} {which}: moved by {}", row.name, by.join(", "))
                })
                .collect::<Vec<_>>()
        })
        .collect();
    for row in &list.rows {
        let mut name = row.name.clone();
        if let Some(source) = row.mirror_of.and_then(|id| design.part(id)) {
            let _ = write!(name, " (mirror of {})", source.name);
        }
        let band = row
            .banding
            .as_ref()
            .map(|band| banding(design, row.part, band))
            .unwrap_or_default();
        let line = format!(
            "{:<28} {:<14} {:<26} {:<26} {band}",
            name,
            row.stock,
            size(&row.cut),
            size(&row.rough)
        );
        let _ = writeln!(out, "{}", line.trim_end());
    }
    if !moved.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "Rounded:");
        for line in moved {
            let _ = writeln!(out, "  {line}");
        }
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "To buy:");
    for stock in &list.stocks {
        let what = match (stock.form, stock.boards, stock.max_length) {
            (StockForm::Plywood | StockForm::Mdf, _, _) => {
                "sheets are not laid out yet: cut the parts above".to_owned()
            }
            (StockForm::Solid | StockForm::Dowel, Some(boards), Some(max_length)) => format!(
                "{boards} {} {} long, with {} kerf between pieces, one piece to a board's width (ripping narrow parts side by side can save boards)",
                if boards == 1 { "board" } else { "boards" },
                max_length,
                stock.kerf
            ),
            (StockForm::Solid | StockForm::Dowel, _, _) => {
                "no count: the stock doesn't say how long its boards are (max_length)".to_owned()
            }
        };
        let pieces = if stock.pieces == 1 { "piece" } else { "pieces" };
        let _ = writeln!(
            out,
            "  {:<14} {} {pieces}: {what}",
            stock.stock, stock.pieces
        );
    }
    if !list.banding.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "Edge banding:");
        for band in &list.banding {
            let _ = writeln!(
                out,
                "  {} thick: {}mm",
                band.thickness,
                rounded(band.length)
            );
        }
    }
    out
}
