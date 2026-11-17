//! "Did you mean ...?" help lines, snapshot-tested.

mod common;

use common::{HEADER, open, open_files};
use stave_lang::render_diagnostics;

fn render(opened: &stave_app::Opened) -> String {
    render_diagnostics(&opened.sources, &opened.diagnostics)
}

#[test]
fn typos_get_the_closest_names() {
    let opened = open(
        "part shelf : pine_18 {
  length: 100mm
  widht:  100mm
  place {
    aginst(base.tp)
    flush(base.left)
    flush(base.front)
  }
}
part rail[left, right] : pine_19 {
  length: 100mm
  width:  20mm
  place {
    against(base.top)
    spaced(ends, from: base.left, to: base.right)
    flush(base.front)
  }
}
let a = rail[rihgt].length
let b = side_mount_slide(lenght: 450mm)
joint housing : dado(rail[left], int: base, depth: 5mm)
component Box {
  param height = 50mm
}
assembly box : Box {
  hieght: 60mm
}
region space = opening(left: base.left, rigth: base.right, bottom: base.bottom, top: base.top, front: base.front, back: base.back)
let far_away = completely_unrelated + 1mm",
    );
    insta::assert_snapshot!(render(&opened));
}

#[test]
fn qualified_names_suggest_the_module_s_items() {
    let library = "stave 1\n\nlibrary \"Parts\"\n\nlet width = 120mm\n";
    let project = format!(
        "stave 1\n\nimport \"./parts\"\n{}\nlet a = parts.widht\n",
        &HEADER["stave 1\n".len()..]
    );
    let opened = open_files(&[
        ("project.stave", &project),
        ("parts/library.stave", library),
    ]);
    insta::assert_snapshot!(render(&opened));
}

/// The rendered diagnostics of a broken design, with paths relative to the
/// workspace.
fn broken(folder: &str) -> String {
    let workspace = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
    let opened = stave_app::open(&workspace.join("tests/broken").join(folder)).unwrap();
    render(&opened).replace(&format!("{}/", workspace.display()), "")
}

#[test]
fn a_failed_kind_rule_offers_a_value_from_the_kinds_list() {
    insta::assert_snapshot!(broken("require_help"));
}

#[test]
fn a_failed_kind_rule_says_when_no_listed_value_fits() {
    insta::assert_snapshot!(broken("require_no_fit"));
}
