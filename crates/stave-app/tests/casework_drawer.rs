//! The standard `casework.Drawer` with its default pull fits a cabinet of
//! any common width. Its front screws (through the box front into the
//! drawer front) and the pull's screws (through the drawer front, centred)
//! used to meet in cabinets about 575mm to 635mm wide, 600mm the most
//! common width of all: the front screws were spread evenly, a third of
//! the way in from each end, right where a 160mm pull's screws go.

use std::path::Path;

use stave_lang::render_diagnostics;
use stave_modules::MemoryFiles;

/// A case `width` wide and 250mm high, filled by one drawer with the
/// default pull.
fn cabinet(width: u32) -> String {
    format!(
        r#"stave 1

import "std:casework"

project "Drawer test" {{
  units: mm
}}

param overall_width  = {width}mm
param overall_height = 250mm
param overall_depth  = 450mm

region envelope = box(across: overall_width, up: overall_height, deep: overall_depth)

stock oak_19   {{ species: white_oak, form: solid, thickness: 19mm, max_width: 200mm, max_length: 2700mm }}
stock maple_12 {{ species: hard_maple, form: solid, thickness: 12mm, max_width: 150mm, max_length: 2400mm }}
stock ply_6    {{ species: birch, form: plywood, thickness: 6mm, max_length: 2440mm }}

assembly case : casework.Case {{
  space:       envelope
  panel_stock: oak_19
}}

assembly drawer : casework.Drawer {{
  opening:      case.inside
  mount:        overlay(left: 18mm, right: 18mm, top: -1.5mm, bottom: 8mm, meeting: -0.75mm)
  front_stock:  oak_19
  box_stock:    maple_12
  bottom_stock: ply_6
  slide:        side_mount_slide(length: 400mm)
}}
"#
    )
}

#[test]
fn a_drawer_with_the_default_pull_fits_the_common_cabinet_widths() {
    for width in [400, 450, 500, 550, 580, 600, 620, 635, 700, 800, 900, 1000] {
        let folder = Path::new("drawer_test");
        let mut files = MemoryFiles::new();
        files.insert(folder.join("drawer_test.stave"), cabinet(width));
        let opened = stave_app::open_with(folder, &files).unwrap();
        assert_eq!(
            opened.error_count(),
            0,
            "{width}mm:\n{}",
            render_diagnostics(&opened.sources, &opened.diagnostics)
        );
    }
}
