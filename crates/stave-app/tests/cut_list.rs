//! Executable spec: the cut list and its allowances (grammar, "Cut list"
//! and "Edge banding"): kerf, rough sizes, the bought thickness and edge
//! banding, each size rounded once.

mod common;

use std::path::Path;

use common::{design, mm, open_files};
use stave_lang::units::{ExactLength, Length};
use stave_solve::cut_list::{CutList, Row, cut_list};

fn list_of(text: &str) -> CutList {
    let opened = open_files(&[("project.stave", text)]);
    cut_list(design(&opened)).unwrap()
}

fn row<'l>(list: &'l CutList, name: &str) -> &'l Row {
    list.rows
        .iter()
        .find(|row| row.name == name)
        .unwrap_or_else(|| panic!("no row '{name}'"))
}

/// A row's sizes, length by width by thickness, as rounded.
fn sizes(size: &stave_solve::cut_list::RoundedSize) -> [Length; 3] {
    size.each().map(|value| value.length)
}

fn lengths(values: [&str; 3]) -> [Length; 3] {
    values.map(mm)
}

fn boards(list: &CutList, stock: &str) -> Option<u32> {
    list.stocks
        .iter()
        .find(|entry| entry.stock == stock)
        .unwrap_or_else(|| panic!("no stock '{stock}'"))
        .boards
}

#[test]
fn the_shelf_unit_example_lists_rough_sizes_banding_and_boards() {
    let path = Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/shelf_unit"
    ));
    let opened = stave_app::open(path).unwrap();
    assert!(opened.diagnostics.is_empty(), "{:?}", opened.diagnostics);
    let list = cut_list(design(&opened)).unwrap();
    // The oak sides: cut 760 x 280 x 19, first cut 25mm long and 5mm wide
    // from 1in (25.4mm) boards, rounded to the half millimetre once.
    let side = row(&list, "side_l");
    assert_eq!(sizes(&side.cut), lengths(["760", "280", "19"]));
    assert_eq!(sizes(&side.rough), lengths(["785", "285", "25.5"]));
    assert_eq!(side.rough.thickness.moved_by, mm("0.1").to_exact());
    assert_eq!(side.cut.length.moved_by, ExactLength::ZERO);
    // A shelf is drawn 280mm deep, banded 1mm on its front: cut 279mm.
    // Plywood is cut straight to size, so its rough size is its cut size.
    let shelf = row(&list, "shelf[0]");
    assert_eq!(sizes(&shelf.cut)[1], mm("279"));
    assert_eq!(shelf.rough, shelf.cut);
    assert!(shelf.banding.is_some());
    // Two 785mm rough sides and a 3mm kerf fit one 1800mm board; the
    // plywood is not laid out.
    assert_eq!(boards(&list, "oak_19"), Some(1));
    assert_eq!(boards(&list, "ply_18"), None);
    // Three shelves of banding along their fronts.
    let shelf_length = sizes(&shelf.cut)[0];
    assert_eq!(list.banding.len(), 1);
    assert_eq!(list.banding[0].thickness, mm("1"));
    assert_eq!(
        list.banding[0].length.length,
        shelf_length.checked_mul(3).unwrap()
    );
}

/// Four 450mm rails on a stock of 1800mm boards, with a project kerf.
fn rails(kerf: &str) -> String {
    format!(
        "stave 1

project \"Rails\" {{
  units: mm
  {kerf}
}}

stock oak_19 {{ species: white_oak, form: solid, thickness: 19mm, max_width: 200mm, max_length: 1800mm }}

part base : oak_19 {{
  length: 450mm
  width:  90mm
  place {{
    ground
  }}
}}

part rail_a : oak_19 {{
  length: 450mm
  width:  90mm
  place {{
    against(base.top)
    flush(base.left)
    flush(base.front)
  }}
}}

part rail_b : oak_19 {{
  length: 450mm
  width:  90mm
  place {{
    against(rail_a.top)
    flush(base.left)
    flush(base.front)
  }}
}}

part rail_c : oak_19 {{
  length: 450mm
  width:  90mm
  place {{
    against(rail_b.top)
    flush(base.left)
    flush(base.front)
  }}
}}
"
    )
}

#[test]
fn the_kerf_decides_whether_four_450mm_parts_fit_an_1800mm_board() {
    // The default kerf, 3mm: 4 x 450 + 3 x 3 = 1809mm, two boards.
    let list = list_of(&rails(""));
    assert_eq!(boards(&list, "oak_19"), Some(2));
    assert_eq!(
        list.stocks[0].kerf,
        Length::from_nm(stave_schema::fields::DEFAULT_KERF_NM)
    );
    // No kerf at all: exactly one board.
    let list = list_of(&rails("kerf: 0mm"));
    assert_eq!(boards(&list, "oak_19"), Some(1));
}

#[test]
fn a_stock_overrides_the_project_and_rough_allowances_follow_the_grain() {
    let list = list_of(
        "stave 1

project \"Allowances\" {
  units:        mm
  kerf:         4mm
  rough_length: 20mm
  rough_width:  6mm
}

stock oak_19  { species: white_oak, form: solid, thickness: 19mm, max_width: 200mm, max_length: 2400mm }
stock pine_19 { species: pine, form: solid, thickness: 19mm, kerf: 2mm, rough_length: 50mm, rough_width: 0mm,
                max_width: 300mm, max_length: 2400mm }

part base : oak_19 {
  length: 600mm
  width:  100mm
  place {
    ground
  }
}

// Its grain runs across: the rough length goes on its width.
part cleat : oak_19 {
  length: 80mm
  width:  150mm
  grain:  width
  place {
    against(base.top)
    flush(base.left)
    flush(base.front)
  }
}

part slat : pine_19 {
  length: 500mm
  width:  60mm
  place {
    against(base.bottom)
    flush(base.left)
    flush(base.front)
  }
}
",
    );
    let base = row(&list, "base");
    assert_eq!(sizes(&base.rough), lengths(["620", "106", "19"]));
    let cleat = row(&list, "cleat");
    assert_eq!(sizes(&cleat.rough), lengths(["86", "170", "19"]));
    // The pine's own allowances, not the project's.
    let slat = row(&list, "slat");
    assert_eq!(sizes(&slat.rough), lengths(["550", "60", "19"]));
    let pine = list
        .stocks
        .iter()
        .find(|entry| entry.stock == "pine_19")
        .unwrap();
    assert_eq!(pine.kerf, mm("2"));
    let oak = list
        .stocks
        .iter()
        .find(|entry| entry.stock == "oak_19")
        .unwrap();
    assert_eq!(oak.kerf, mm("4"));
    assert_eq!(oak.pieces, 2);
}

#[test]
fn a_part_made_of_boards_is_one_row_per_board() {
    let list = list_of(
        "stave 1

project \"Wide top\" {
  units:       mm
  rough_width: 5mm
}

stock pine_19 { species: pine, form: solid, thickness: 19mm, max_width: 200mm, max_length: 2400mm }

part top : pine_19 {
  length: 900mm
  width:  450mm
  place {
    ground
  }
}
",
    );
    let names: Vec<&str> = list.rows.iter().map(|row| row.name.as_str()).collect();
    assert_eq!(names, ["top.board[1]", "top.board[2]", "top.board[3]"]);
    for row in &list.rows {
        assert_eq!(sizes(&row.cut)[1], mm("150"));
        assert_eq!(sizes(&row.rough)[1], mm("155"));
    }
}

#[test]
fn banding_on_both_ends_comes_off_twice_and_a_third_rounds_once() {
    let list = list_of(
        "stave 1

project \"Banded panel\" {
  units:      mm
  resolution: 1mm
}

stock ply_18 { species: birch, form: plywood, thickness: 18mm, max_width: 1220mm, max_length: 2440mm }

part panel : ply_18 {
  length:  600mm
  width:   1000mm / 3
  banding: edge_band(2mm, on: [left, right, front])
  place {
    ground
  }
}
",
    );
    let panel = row(&list, "panel");
    // 600 less two bands; 333.33... less one band is 331.33..., rounded
    // once to 331mm.
    assert_eq!(sizes(&panel.cut)[0], mm("596"));
    assert_eq!(sizes(&panel.cut)[1], mm("331"));
    assert_ne!(panel.cut.width.moved_by, ExactLength::ZERO);
}

#[test]
fn a_rough_length_within_the_board_passes() {
    // 1775mm cut and 25mm rough is exactly the 1800mm board.
    let opened = open_files(&[(
        "project.stave",
        "stave 1

project \"Just fits\" {
  units:        mm
  rough_length: 25mm
}

stock oak_19 { species: white_oak, form: solid, thickness: 19mm, max_width: 200mm, max_length: 1800mm }

part rail : oak_19 {
  length: 1775mm
  width:  90mm
  place {
    ground
  }
}
",
    )]);
    let list = cut_list(design(&opened)).unwrap();
    assert_eq!(sizes(&row(&list, "rail").rough)[0], mm("1800"));
}

#[test]
fn a_mirror_image_is_banded_on_the_mirrored_face() {
    let opened = open_files(&[(
        "project.stave",
        "stave 1

project \"Mirrored panels\" {
  units: mm
}

stock ply_18 { species: birch, form: plywood, thickness: 18mm, max_width: 1220mm, max_length: 2440mm }

part base : ply_18 {
  length: 1000mm
  width:  400mm
  place {
    ground
  }
}

part panel_l : ply_18 {
  length:  300mm
  width:   400mm
  banding: edge_band(1mm, on: left)
  place {
    against(base.top)
    flush(base.left)
    flush(base.front)
  }
}

part panel_r = mirror(panel_l, about: mid(base, across))
",
    )]);
    let design = design(&opened);
    let list = cut_list(design).unwrap();
    let mirrored = row(&list, "panel_r");
    assert_eq!(sizes(&mirrored.cut)[0], mm("299"));
    assert!(mirrored.mirror_of.is_some());
    // Placed, the band is on panel_r's right face.
    let panel = common::part(design, "panel_r");
    let band = panel.banding.as_ref().unwrap();
    let face = band.faces[0];
    let direction = panel.pose.orientation.direction(face.axis);
    let direction = match face.end {
        stave_model::solved::End::End => direction,
        stave_model::solved::End::Start => direction.opposite(),
    };
    assert_eq!(direction.face().word(), "right");
}
