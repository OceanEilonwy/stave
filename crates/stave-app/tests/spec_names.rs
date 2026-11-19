//! Executable spec: "Names" (the four namespaces and their rules),
//! "Arrays", "Modules" and "The prelude".

mod common;

use common::{design, diagnostics, extent, holds, mm, only_error, open, open_files, part};

/// A two-element labelled array of posts standing on the base.
const POSTS: &str = "part post[left, right] : pine_19 {
  length: 100mm
  width:  19mm
  place {
    orient(length: up, width: back)
    against(base.top)
    spaced(ends, from: base.left, to: base.right)
    flush(base.front)
  }
}";

/// Three slats on the base, spaced evenly.
const SLATS: &str = "part slat[3] : pine_12 {
  length: 50mm
  width:  20mm
  place {
    against(base.top)
    spaced(evenly, from: base.left, to: base.right)
    flush(base.front)
  }
}";

// ---------------------------------------------------------------------------
// Items: scopes, shadowing and built-in functions
// ---------------------------------------------------------------------------

#[test]
fn a_name_declared_twice_in_a_component_is_a_duplicate_name() {
    only_error(
        &open("component Box {\n  let a = 1mm\n  let a = 2mm\n}"),
        "duplicate-name",
        "'a' is declared twice",
    );
}

#[test]
fn an_inner_scope_may_reuse_an_outer_name_and_wins() {
    holds(
        "let w = 1mm\ncomponent Box {\n  param w = 50mm\n  part p : pine_19 {\n    length: w\n    width:  100mm\n    place {\n      against(base.top)\n      flush(base.left)\n      flush(base.front)\n    }\n  }\n}\nassembly b : Box { }\nrequire r : b.p.length == 50mm && w == 1mm \"the component's w inside\"",
    );
}

#[test]
fn a_module_item_may_take_the_name_of_a_prelude_item_without_a_warning() {
    // Only a kind hiding a kind (or a recipe hiding a recipe) warns.
    let opened = open(
        "param knob = 3mm\npart square : pine_19 {\n  length: 100mm\n  width:  100mm\n  place {\n    against(base.top)\n    flush(base.left)\n    flush(base.front)\n  }\n}\nrequire r : knob == 3mm \"a param called knob\"",
    );
    assert!(opened.diagnostics.is_empty(), "{:?}", diagnostics(&opened));
}

#[test]
fn a_recipe_hiding_a_standard_recipe_is_a_warning() {
    let opened =
        open("recipe notch_by_hand : notch {\n  tools: [chisel]\n  cut {\n    chop(waste)\n  }\n}");
    assert_eq!(
        diagnostics(&opened),
        ["shadows-prelude: 'notch_by_hand' hides the standard recipe of the same name"]
    );
    assert!(opened.design.is_some(), "a warning doesn't stop the design");
}

#[test]
fn a_kind_or_component_can_not_take_a_builtin_functions_name() {
    let opened = open("kind between : joint { }");
    only_error(&opened, "duplicate-name", "'between' is declared twice");
    assert_eq!(
        common::help(&opened),
        "'between' is a built-in function; a kind can't take its name because both could be called"
    );
    only_error(
        &open("component mid { }"),
        "duplicate-name",
        "'mid' is declared twice",
    );
}

#[test]
fn a_builtin_is_found_only_when_called() {
    // `mid` is a let here, and still the built-in when called.
    let opened = open(
        "let mid = 3mm\npart p : pine_19 {\n  length: 100mm + mid\n  width:  100mm\n  place {\n    against(base.top)\n    center(from: base.left, to: mid(base, across))\n    flush(base.front)\n  }\n}",
    );
    let p = part(design(&opened), "p");
    assert_eq!(p.cut.length, mm("103"));
    assert_eq!(extent(p)[0], (mm("98.5"), mm("201.5")));
}

#[test]
fn members_never_clash_with_items() {
    // `top` is a part, a label and a face: `shelf[top].top` is the top face
    // of the element labelled `top`.
    holds(
        "part top : pine_19 {\n  length: 600mm\n  width:  300mm\n  place {\n    against(base.top, gap: 300mm)\n    flush(base.left)\n    flush(base.front)\n  }\n}\npart shelf[bottom, top] : pine_12 {\n  length: 100mm\n  width:  100mm\n  place {\n    against(base.top)\n    spaced(ends, from: base.left, to: base.right)\n    flush(base.front)\n  }\n}\nrequire r : between(shelf[top].top, top.bottom) == 288mm \"members\"",
    );
}

#[test]
fn an_unknown_name_suggests_the_closest_names() {
    let opened = open("let a = baes.length");
    only_error(&opened, "unknown-name", "unknown name 'baes'");
    assert_eq!(common::help(&opened), "did you mean 'base'?");
}

#[test]
fn a_module_name_alone_is_not_a_value() {
    let opened = open_files(&[
        (
            "project.stave",
            &format!(
                "stave 1\n\nimport \"./parts\"\n{}\nlet a = parts\n",
                &common::HEADER["stave 1\n".len()..]
            ),
        ),
        (
            "parts/library.stave",
            "stave 1\n\nlibrary \"Parts\"\n\nlet width = 1mm\n",
        ),
    ]);
    only_error(&opened, "unknown-name", "unknown name 'parts'");
    assert_eq!(
        common::help(&opened),
        "'parts' is a module; name one of its items: parts.<name>"
    );
}

// ---------------------------------------------------------------------------
// Names are resolved where they are written
// ---------------------------------------------------------------------------

#[test]
fn a_library_component_sees_its_own_module_not_the_users() {
    let library = "stave 1\n\nlibrary \"Boxes\"\n\nstock ply_6 { species: birch, form: plywood, thickness: 6mm }\nlet depth = 80mm\n\ncomponent Tray {\n  param floor : Part\n  part bottom : ply_6 {\n    length: depth\n    width:  depth\n    place {\n      against(floor.top)\n      flush(floor.left)\n      flush(floor.front)\n    }\n  }\n}\n";
    let project = format!(
        "stave 1\n\nimport \"./boxes\"\n{}\nlet depth = 1mm\nassembly tray : boxes.Tray {{\n  floor: base\n}}\n",
        &common::HEADER["stave 1\n".len()..]
    );
    let opened = open_files(&[
        ("project.stave", &project),
        ("boxes/library.stave", library),
    ]);
    let bottom = part(design(&opened), "tray.bottom");
    assert_eq!(bottom.cut.length, mm("80"), "the library's depth");
    assert_eq!(bottom.cut.thickness, mm("6"), "the library's stock");
}

// ---------------------------------------------------------------------------
// Modules and the prelude
// ---------------------------------------------------------------------------

#[test]
fn every_kind_of_placed_item_is_not_in_a_library() {
    for (item, text) in [
        (
            "a part",
            "part p : s {\n  length: 1mm\n  width: 1mm\n  place {\n    ground\n  }\n}",
        ),
        (
            "a region",
            "region r = opening(left: a.left, right: a.right, bottom: a.bottom, top: a.top, front: a.front, back: a.back)",
        ),
        ("a scene", "scene v { }"),
        ("an assembly", "component C { }\nassembly c : C { }"),
        ("a joint", "joint j : dado(a, into: a)"),
        ("a hardware", "hardware h : knob(a)"),
    ] {
        let opened = open_files(&[(
            "library.stave",
            &format!(
                "stave 1\n\nlibrary \"L\"\n\nstock s {{ species: pine, form: solid, thickness: 19mm }}\n{text}\n"
            ),
        )]);
        let found = diagnostics(&opened);
        assert!(
            found.contains(&format!(
                "not-in-library: a library can't contain {item} at the top level"
            )),
            "{item}: {found:?}"
        );
    }
}

#[test]
fn a_library_may_hold_placed_items_inside_a_component() {
    let opened = open_files(&[(
        "library.stave",
        "stave 1\n\nlibrary \"L\"\n\nstock s { species: pine, form: solid, thickness: 19mm }\nparam size = 10mm\nlet half = size / 2\nprofile peg {\n  measure: radius\n  at 0mm:  5mm\n  at 10mm: 5mm\n}\nrequire positive : size > 0mm \"a size\"\ncomponent C {\n  param floor : Part\n  part p : s {\n    length: size\n    width:  half\n    place {\n      against(floor.top)\n      flush(floor.left)\n      flush(floor.front)\n    }\n  }\n  region over = opening(left: p.left, right: p.right, bottom: p.top, top: p.top, front: p.front, back: p.back)\n}\n",
    )]);
    assert!(opened.diagnostics.is_empty(), "{:?}", diagnostics(&opened));
}

#[test]
fn a_hidden_prelude_item_is_reached_by_importing_its_module() {
    // A local `dado` hides the standard one; `joinery.dado` reaches it.
    let project = format!(
        "stave 1\n\nimport \"std:joinery\"\n{}\nkind dado : joint {{\n  note: \"mine\"\n}}\npart side : pine_19 {{\n  length: 400mm\n  width:  300mm\n  place {{\n    orient(length: up, width: back)\n    against(base.top)\n    flush(base.left)\n    flush(base.front)\n  }}\n}}\npart shelf : pine_19 {{\n  length: 200mm\n  width:  300mm\n  place {{\n    against(base.top, gap: 100mm)\n    against(side.right)\n    flush(base.front)\n  }}\n}}\njoint housing : joinery.dado(shelf, into: side, depth: 5mm)\n",
        &common::HEADER["stave 1\n".len()..]
    );
    let opened = open_files(&[("project.stave", &project)]);
    assert_eq!(
        diagnostics(&opened),
        ["shadows-prelude: 'dado' hides the standard kind of the same name"]
    );
    let design = opened.design.as_ref().unwrap();
    assert_eq!(part(design, "shelf").cut.length, mm("205"));
    assert_eq!(common::joint(design, "housing").kind, "joinery.dado");
}

// ---------------------------------------------------------------------------
// Arrays
// ---------------------------------------------------------------------------

#[test]
fn a_counted_array_is_numbered_from_zero() {
    let opened = open(&format!("{SLATS}\nlet x = slat[3].length"));
    only_error(&opened, "unknown-name", "unknown name '3'");
    assert_eq!(
        common::help(&opened),
        "the array has 3 elements, numbered from 0"
    );
}

#[test]
fn a_counted_array_is_indexed_by_a_count_not_a_label() {
    let opened = open(&format!("{SLATS}\nlet x = slat[top].length"));
    only_error(&opened, "unknown-name", "unknown name 'top'");
}

#[test]
fn a_counted_array_index_may_be_an_expression() {
    holds(&format!(
        "{SLATS}\nlet i = 1\nrequire r : between(slat[i + 1].left, base.right) == 162.5mm \"slat[2]\""
    ));
}

#[test]
fn only_arrays_can_be_indexed() {
    only_error(
        &open("let x = base[0].length"),
        "type-mismatch",
        "'base' is not an array",
    );
}

#[test]
fn the_same_label_twice_is_a_duplicate_name() {
    let opened = open(&POSTS.replace("[left, right]", "[left, left]"));
    only_error(&opened, "duplicate-name", "'left' is declared twice");
    assert_eq!(
        common::help(&opened),
        "every label of an array must be different"
    );
}

#[test]
fn a_single_name_in_brackets_is_a_count_not_a_label() {
    // `part post[left]` is a count expression; `left` names nothing, so it
    // is a single label, which isn't an array.
    let opened = open(&POSTS.replace("[left, right]", "[left]"));
    let found = diagnostics(&opened);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        found[0]
            .starts_with("type-mismatch: one label isn't an array: 'left' is the only label of"),
        "{found:?}"
    );
}

#[test]
fn the_count_is_a_constant_or_a_param_with_a_within_list() {
    holds(&format!(
        "param n : Count = 3 within [2, 3, 4]\n{}\nrequire r : between(slat[2].right, base.right) == 112.5mm \"three slats: gaps of (600 - 150) / 4\"",
        SLATS.replace("slat[3]", "slat[n]")
    ));
    holds(&format!(
        "let n = 1 + 2\n{}\nrequire r : slat[2].length == 50mm \"a let\"",
        SLATS.replace("slat[3]", "slat[n]")
    ));
    let opened = open(&format!(
        "param n : Count = 3 within 2..4\n{}",
        SLATS.replace("slat[3]", "slat[n]")
    ));
    only_error(
        &opened,
        "type-mismatch",
        "'slat' needs a whole number of elements known before placing",
    );
}

#[test]
fn a_count_with_a_unit_is_one_error() {
    only_error(
        &open(&SLATS.replace("slat[3]", "slat[3mm]")),
        "unit-mismatch",
        "'count' needs a count, found a length",
    );
}

#[test]
fn same_is_only_allowed_inside_an_array_declaration() {
    let opened = open(&format!("{SLATS}\nlet x = slat[same].length"));
    only_error(
        &opened,
        "type-mismatch",
        "'[same]' is only allowed inside the declaration of an array",
    );
}

#[test]
fn same_pairs_counted_arrays_of_the_same_count() {
    let cap = |count: &str| {
        format!(
            "{SLATS}\npart cap[{count}] : pine_12 {{\n  length: 50mm\n  width:  20mm\n  place {{\n    against(slat[same].top)\n    flush(slat[same].left)\n    flush(slat[same].front)\n  }}\n}}"
        )
    };
    let opened = open(&cap("3"));
    let design = design(&opened);
    assert_eq!(
        extent(part(design, "cap[2]"))[0],
        extent(part(design, "slat[2]"))[0]
    );
    assert!(
        diagnostics(&open(&cap("2"))).iter().any(|d| d
            == "type-mismatch: 'slat[same]' needs the same labels as 'cap', found different ones"),
    );
}

#[test]
fn same_needs_the_labels_in_the_same_order() {
    let opened = open(&format!(
        "{POSTS}\npart cap[right, left] : pine_19 {{\n  length: 50mm\n  width:  19mm\n  place {{\n    against(post[same].top)\n    flush(post[same].left)\n    flush(post[same].front)\n  }}\n}}"
    ));
    assert!(
        diagnostics(&opened).iter().all(|d| d
            == "type-mismatch: 'post[same]' needs the same labels as 'cap', found different ones"),
        "{:?}",
        diagnostics(&opened)
    );
}

#[test]
fn an_unindexed_array_name_in_its_own_declaration_is_the_element() {
    holds(
        "part cap[a, b] : pine_19 {\n  length: 50mm\n  width:  cap.length\n  place {\n    against(base.top)\n    spaced(ends, from: base.left, to: base.right)\n    flush(base.front)\n  }\n}\nrequire r : cap[a].width == 50mm && cap[b].width == 50mm \"the element\"",
    );
}

#[test]
fn star_flattens_inside_a_list() {
    let opened = open(&format!(
        "{SLATS}\n{POSTS}\nscene some {{ show: [slat[*], post[*]] }}"
    ));
    let design = design(&opened);
    assert_eq!(design.scenes[1].show.as_ref().map(Vec::len), Some(5));
}

#[test]
fn a_file_in_another_format_version_is_not_supported() {
    let opened = open_files(&[(
        "project.stave",
        &common::HEADER.replacen("stave 1", "stave 2", 1),
    )]);
    let diagnostic = common::the_diagnostic(
        &opened,
        "not-supported",
        "format version 2 is not supported yet",
    );
    assert_eq!(common::line(&opened, diagnostic), 1);
    assert_eq!(
        diagnostic.help.as_deref(),
        Some("this version of Stave reads files that start with 'stave 1'")
    );
}

#[test]
fn an_import_applies_to_the_file_it_is_written_in() {
    let library = "stave 1\n\nlibrary \"Parts\"\n\nlet width = 120mm\n";
    let importing = format!(
        "stave 1\n\nimport \"./parts\"\n{}\nlet a = parts.width\n",
        &common::HEADER["stave 1\n".len()..]
    );
    let opened = open_files(&[
        ("a.stave", &importing),
        ("b.stave", "stave 1\n\nlet b = parts.width\n"),
        ("parts/library.stave", library),
    ]);
    only_error(&opened, "unknown-name", "unknown name 'parts'");
    let diagnostic = &opened.diagnostics[0];
    let file = opened
        .sources
        .get(diagnostic.primary().unwrap().file)
        .unwrap();
    assert!(file.path().ends_with("b.stave"), "{}", file.path());
}

#[test]
fn a_library_component_can_not_reach_the_parts_of_the_module_using_it() {
    // Placement stays inside a module: what a component needs from outside
    // arrives through its params.
    let library = "stave 1\n\nlibrary \"Boxes\"\n\nstock ply_6 { species: birch, form: plywood, thickness: 6mm }\n\ncomponent Tray {\n  part bottom : ply_6 {\n    length: 80mm\n    width:  80mm\n    place {\n      against(base.top)\n      flush(base.left)\n      flush(base.front)\n    }\n  }\n}\n";
    let project = format!(
        "stave 1\n\nimport \"./boxes\"\n{}\nassembly tray : boxes.Tray {{ }}\n",
        &common::HEADER["stave 1\n".len()..]
    );
    let opened = open_files(&[
        ("project.stave", &project),
        ("boxes/library.stave", library),
    ]);
    let found = diagnostics(&opened);
    assert!(
        !found.is_empty()
            && found
                .iter()
                .all(|d| d == "unknown-name: unknown name 'base'"),
        "{found:?}"
    );
}

#[test]
fn the_prelude_provides_every_item_the_grammar_lists() {
    // The "Provides" column of the prelude table, read from the grammar.
    let grammar = include_str!("../../../docs/grammar.md");
    let mut names = Vec::new();
    for row in grammar.lines().filter(|line| line.starts_with("| `std:")) {
        let provides = row.split('|').nth(3).unwrap();
        for (index, piece) in provides.split('`').enumerate() {
            if index % 2 == 1 && piece.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
                names.push(piece.to_owned());
            }
        }
    }
    assert!(names.len() >= 20, "{names:?}");
    let body: String = names
        .iter()
        .map(|name| format!("let uses_{name} = {name}\n"))
        .collect();
    let opened = open(&body);
    assert!(opened.diagnostics.is_empty(), "{:?}", diagnostics(&opened));
}
