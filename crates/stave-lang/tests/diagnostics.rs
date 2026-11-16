//! Snapshots of rendered diagnostics: what a user sees in the terminal.

use stave_lang::{SourceMap, parse_file, render_diagnostics};

fn render(path: &str, text: &str) -> String {
    let mut sources = SourceMap::new();
    let file = sources.add(path, text).unwrap();
    render_diagnostics(&sources, &parse_file(file, text).diagnostics)
}

fn broken(folder: &str) -> String {
    let path = format!("tests/broken/{folder}/project.stave");
    let text =
        std::fs::read_to_string(format!("{}/../../{path}", env!("CARGO_MANIFEST_DIR"))).unwrap();
    render(&path, &text)
}

#[test]
fn syntax_error() {
    insta::assert_snapshot!(broken("syntax_error"));
}

#[test]
fn missing_value() {
    insta::assert_snapshot!(broken("missing_value"));
}

#[test]
fn reserved_word() {
    insta::assert_snapshot!(broken("reserved_word"));
}

#[test]
fn unknown_unit() {
    insta::assert_snapshot!(broken("unknown_unit"));
}

#[test]
fn unclosed_bracket() {
    insta::assert_snapshot!(broken("unclosed_bracket"));
}

#[test]
fn missing_brace() {
    insta::assert_snapshot!(broken("missing_brace"));
}

#[test]
fn several_errors_in_one_file() {
    insta::assert_snapshot!(render(
        "carcass.stave",
        "stave 1\n\
         \n\
         region front_opening = opening(\n\
         \x20 left: side_l.right,\n\
         \x20 top:  top.bottom              // the \")\" is missing\n\
         region bay[drawers, doors] = split(front_opening, up, at: divider)\n\
         \n\
         joint shelf_dados : dado(shelf, into: side) { depth: 9mm }\n\
         project \"Goal {name}\" { units: mm }\n\
         let x = \"never closed\n\
         part p : s {\n\
         \x20 length: 1mm\n"
    ));
}

#[test]
fn an_operator_at_the_start_of_a_line() {
    insta::assert_snapshot!(render(
        "cabinet.stave",
        "stave 1\n\
         param h = 10mm\n\
         \x20 - 2mm\n\
         part side : oak {\n\
         \x20 length: cabinet_height\n\
         \x20         - base.thickness\n\
         }\n"
    ));
}

#[test]
fn a_call_written_without_parentheses() {
    insta::assert_snapshot!(render(
        "carcass.stave",
        "stave 1\n\
         part shelf : oak {\n\
         \x20 length: between side_l\n\
         \x20 place {\n\
         \x20   on base.top\n\
         \x20   center base, across\n\
         \x20 }\n\
         }\n\
         kind k : joint {\n\
         \x20 cut crossing into {\n\
         \x20   notch(depth: 3mm)\n\
         \x20 }\n\
         }\n\
         recipe r : notch {\n\
         \x20 cut {\n\
         \x20   saw walls\n\
         \x20 }\n\
         }\n"
    ));
}
