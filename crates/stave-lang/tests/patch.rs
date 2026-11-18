//! Applying patches: every operation and path form, comments and layout
//! kept, the corpus left alone by patches that change nothing, and the
//! grammar's own example applied to `examples/clothing_cabinet`.

use std::path::{Path, PathBuf};

use stave_lang::placement::Axis;
use stave_lang::{
    ModuleFile, ModuleKey, Parse, PatchErrorKind, PatchTarget, PatchedFile, Scope, SourceMap,
    TermAddress, TermFixes, TextEdit, apply_patch, apply_text_edits, ast, format_file, parse_file,
    parse_patch_file,
};

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn parse(text: &str) -> Parse<ast::File> {
    let mut sources = SourceMap::new();
    let file = sources.add("module.stave", text).unwrap();
    parse_file(file, text)
}

fn parse_patch(text: &str) -> Parse<ast::PatchFile> {
    let mut sources = SourceMap::new();
    let file = sources.add("change.stave", text).unwrap();
    let parse = parse_patch_file(file, text);
    assert!(
        !parse.has_errors(),
        "the test's patch doesn't parse: {:?}",
        parse.diagnostics
    );
    parse
}

/// Wraps operations in a patch file.
fn patch_text(ops: &str) -> String {
    let mut text = String::from("stave 1\n\npatch \"test\" {\n");
    for line in ops.lines() {
        text.push_str("  ");
        text.push_str(line);
        text.push('\n');
    }
    text.push_str("}\n");
    text
}

/// Applies `ops` to a one-file project and returns the new text.
fn apply(source: &str, ops: &str) -> Result<String, PatchErrorKind> {
    let module = parse(source);
    let patch = parse_patch(&patch_text(ops));
    let target = PatchTarget::new(vec![ModuleFile {
        name: "main.stave",
        parse: &module,
    }]);
    let changed = apply_patch(&target, &patch).map_err(|error| error.kind)?;
    for file in &changed {
        assert_edits_rebuild(source, file);
    }
    Ok(match changed.as_slice() {
        [] => source.to_owned(),
        [only] => {
            assert_eq!(only.name, "main.stave");
            only.text.clone()
        }
        more => panic!("expected one file, got {more:?}"),
    })
}

/// Applies a changed file's edits to its old text by hand, back to front,
/// and checks they give exactly its new text; and that the edits are in
/// the promised order and as small as the line they change allows.
fn assert_edits_rebuild(old: &str, file: &PatchedFile) {
    let old = if file.created { "" } else { old };
    let mut text = old.to_owned();
    let mut limit = old.len();
    for TextEdit { range, text: new } in &file.edits {
        assert!(
            range.start <= range.end && range.end <= limit,
            "{}: edits out of order: {:?}",
            file.name,
            file.edits
        );
        assert!(
            !(range.is_empty() && new.is_empty()),
            "{}: an empty edit",
            file.name
        );
        text.replace_range(range.clone(), new);
        limit = range.start;
    }
    assert_eq!(
        text, file.text,
        "{}: the edits don't rebuild the text",
        file.name
    );
    assert_eq!(
        apply_text_edits(old, &file.edits).as_deref(),
        Some(file.text.as_str())
    );
}

fn applied(source: &str, ops: &str) -> String {
    let text = apply(source, ops).unwrap_or_else(|error| panic!("{error}"));
    let parse = parse(&text);
    assert!(
        !parse.has_errors(),
        "the patched file doesn't parse: {:?}\n{text}",
        parse.diagnostics
    );
    text
}

// ---------------------------------------------------------------------------
// set
// ---------------------------------------------------------------------------

const CABINET: &str = "\
stave 1

// Sizes.
param cabinet_width = 900mm within 600mm..1200mm   // the outside width
param depth         = 560mm
param opening       : Region
let overhang = 20mm
region bay = between(base, top)

stock oak_19 { species: white_oak, form: solid, thickness: 19mm }

/// The bottom of the box.
part base : oak_19 {
  length: cabinet_width
  width:  depth          // made of boards
  place {
    ground
  }
}

part side_l : oak_19 {
  length: 700mm
  width:  depth
  place {
    orient(length: up, width: back)
    against(base.top)            // stands on the base
    flush(base.left)
    flush(base.back)
  }
}

part side_r = mirror(side_l, about: mid(base, across))

joint dados : dado(base, into: [side_l, side_r], depth: 9mm)
hardware slides[left, right] : slide(side_l, in: bay)

scene all_open \"Open\" {
  explode: 0mm
}
";

#[test]
fn set_a_param_default_keeps_its_range_and_comment() {
    let text = applied(CABINET, "set cabinet_width = 950mm");
    assert!(
        text.contains("param cabinet_width = 950mm within 600mm..1200mm   // the outside width\n")
    );
    assert_eq!(
        text.replace("950mm within", "900mm within"),
        CABINET,
        "nothing else changed"
    );
}

#[test]
fn set_a_param_without_a_default_adds_one() {
    let text = applied(CABINET, "set opening = between(base, side_l)");
    assert!(text.contains("param opening       : Region = between(base, side_l)\n"));
}

#[test]
fn set_a_let_and_a_region() {
    let text = applied(
        CABINET,
        "set overhang = 25mm\nset bay = between(base, side_l)",
    );
    assert!(text.contains("let overhang = 25mm\n"));
    assert!(text.contains("region bay = between(base, side_l)\n"));
}

#[test]
fn set_a_field_keeps_the_comment_next_to_it() {
    let text = applied(CABINET, "set base.width = depth - 10mm");
    assert!(
        text.contains("  width:  depth - 10mm          // made of boards\n"),
        "{text}"
    );
    assert!(text.contains("/// The bottom of the box.\npart base : oak_19 {\n"));
}

#[test]
fn set_a_field_that_is_not_written_adds_it_after_the_last_field() {
    let text = applied(CABINET, "set base.note = \"glue up first\"");
    assert!(
        text.contains(
            "  width:  depth          // made of boards\n  note:   \"glue up first\"\n  place {"
        ),
        "{text}"
    );
}

#[test]
fn set_a_field_of_a_one_line_block() {
    let text = applied(
        CABINET,
        "set oak_19.thickness = 20mm\nset oak_19.max_width = 200mm",
    );
    assert!(
        text.contains(
            "stock oak_19 { species: white_oak, form: solid, thickness: 20mm, max_width: 200mm }\n"
        ),
        "{text}"
    );
}

#[test]
fn set_a_scene_field() {
    let text = applied(CABINET, "set all_open.explode = 50mm");
    assert!(text.contains("  explode: 50mm\n"));
}

#[test]
fn set_a_joint_setting_and_role() {
    let text = applied(
        CABINET,
        "set dados.depth = 8mm\nset dados.into = [side_l]\nset dados.width = 19mm",
    );
    assert!(
        text.contains("joint dados : dado(base, into: [side_l], depth: 8mm, width: 19mm)\n"),
        "{text}"
    );
}

#[test]
fn set_a_hardware_setting_of_an_array_with_every_element() {
    let text = applied(CABINET, "set slides[*].in = between(base, side_l)");
    assert!(
        text.contains("hardware slides[left, right] : slide(side_l, in: between(base, side_l))\n")
    );
}

#[test]
fn set_one_element_of_an_array_is_refused() {
    let error = apply(CABINET, "set slides[left].in = bay").unwrap_err();
    assert!(
        matches!(error, PatchErrorKind::Unaddressable { ref path, .. } if path == "slides[left].in"),
        "{error:?}"
    );
}

#[test]
fn set_a_placement_term_by_axis() {
    let text = applied(
        CABINET,
        "set side_l.place.deep = flush(base.back, inset: 6mm)\n\
         set side_l.place.up = against(base.top, gap: 2mm)\n\
         set side_l.place.across = center(base, across)\n\
         set side_l.place.orient = orient(length: up, width: right)",
    );
    assert!(
        text.contains(
            "  place {\n    orient(length: up, width: right)\n    against(base.top, gap: 2mm)            // stands on the base\n    center(base, across)\n    flush(base.back, inset: 6mm)\n  }\n"
        ),
        "{text}"
    );
}

#[test]
fn set_a_term_that_fixes_every_axis_replaces_it_whole() {
    let text = applied(CABINET, "set base.place.across = center(side_l, across)");
    assert!(
        text.contains("  place {\n    center(side_l, across)\n  }\n"),
        "{text}"
    );
}

#[test]
fn set_a_placement_axis_that_has_no_term_adds_one() {
    let source = "stave 1\n\npart a : s {\n  length: 1mm\n  place {\n    against(b.top)\n  }\n}\n";
    let text = applied(source, "set a.place.deep = flush(b.back)");
    assert_eq!(
        text,
        "stave 1\n\npart a : s {\n  length: 1mm\n  place {\n    against(b.top)\n    flush(b.back)\n  }\n}\n"
    );
    let source = "stave 1\n\npart a : s {\n  length: 1mm\n}\n";
    let text = applied(source, "set a.place.deep = flush(b.back)");
    assert_eq!(
        text,
        "stave 1\n\npart a : s {\n  length: 1mm\n\n  place {\n    flush(b.back)\n  }\n}\n"
    );
}

#[test]
fn set_a_placement_term_that_fixes_another_axis_is_refused() {
    assert_eq!(
        apply(CABINET, "set side_l.place.deep = against(base.top)").unwrap_err(),
        PatchErrorKind::WrongTerm {
            path: "side_l.place.deep".to_owned(),
            term: "against(base.top)".to_owned(),
            addressed: TermAddress::Axis(Axis::Deep),
            fixes: TermFixes::Axes(&[Axis::Up]),
        }
    );
    assert_eq!(
        apply(CABINET, "set side_l.place.orient = flush(base.left)")
            .unwrap_err()
            .to_string(),
        "'flush(base.left)' fixes across, not the orientation, so it can't be set as \
         'side_l.place.orient'"
    );
    assert!(matches!(
        apply(
            CABINET,
            "set side_l.place.up = orient(length: up, width: back)"
        )
        .unwrap_err(),
        PatchErrorKind::WrongTerm {
            addressed: TermAddress::Axis(Axis::Up),
            fixes: TermFixes::Orientation,
            ..
        }
    ));
    // With no term for the axis yet, a term for another axis used to be
    // added, setting that axis twice.
    let source = "stave 1\n\npart a : s {\n  length: 1mm\n  place {\n    against(b.top)\n  }\n}\n";
    assert!(matches!(
        apply(source, "set a.place.deep = flush(b.left)").unwrap_err(),
        PatchErrorKind::WrongTerm {
            fixes: TermFixes::Axes(&[Axis::Across]),
            ..
        }
    ));
}

#[test]
fn set_a_placement_term_that_also_fixes_a_set_axis_is_refused() {
    assert_eq!(
        apply(CABINET, "set side_l.place.across = corners(base)").unwrap_err(),
        PatchErrorKind::TermClash {
            term: "corners(base)".to_owned(),
            axis: Axis::Deep,
            other: "flush(base.back)".to_owned(),
        }
    );
    assert!(matches!(
        apply(CABINET, "set side_l.place.up = ground").unwrap_err(),
        PatchErrorKind::TermClash {
            axis: Axis::Across,
            ..
        }
    ));
}

#[test]
fn set_a_placement_term_whose_axis_cant_be_read_is_let_through() {
    // An `Anchor` param's face can't be read from the text; the design is
    // checked again as a whole after the patch.
    let text = applied(CABINET, "set side_l.place.deep = flush(hinge_face)");
    assert!(text.contains("    flush(hinge_face)\n"), "{text}");
    assert!(!text.contains("flush(base.back)"));
}

#[test]
fn set_a_placement_term_on_a_one_line_part_lays_the_part_out() {
    let source = "stave 1\n\npart a : s { length: 1mm, width: 20mm }   // a\n";
    let text = applied(source, "set a.place.up = against(b.top)");
    assert_eq!(
        text,
        "stave 1\n\npart a : s {\n  length: 1mm\n  width:  20mm\n\n  place {\n    against(b.top)\n  }\n}   // a\n"
    );
    assert_eq!(format(&text), text);

    let source = "stave 1\n\ncomponent C {\n  part a : s { length: 1mm }\n}\n";
    let text = applied(source, "set C.a.place.deep = flush(b.back)");
    assert_eq!(
        text,
        "stave 1\n\ncomponent C {\n  part a : s {\n    length: 1mm\n\n    place {\n      flush(b.back)\n    }\n  }\n}\n"
    );
    assert_eq!(format(&text), text);
}

#[test]
fn set_a_hardware_term_adds_a_body() {
    let source = "stave 1\n\nhardware pull : knob(door)\n";
    let text = applied(source, "set pull.place.up = center(door, up)");
    assert_eq!(
        text,
        "stave 1\n\nhardware pull : knob(door) {\n  place {\n    center(door, up)\n  }\n}\n"
    );
}

#[test]
fn set_with_the_same_value_changes_nothing() {
    let source = "stave 1\nparam  a =  1mm+2mm   // odd spacing\n";
    assert_eq!(applied(source, "set a = 1mm + 2mm"), source);
}

#[test]
fn set_a_multi_line_value_is_laid_out() {
    let source = "stave 1\n\nregion o = opening(left: a)\n";
    let text = applied(source, "set o = opening(\n      left: a,\n  bottom: b,\n)");
    assert_eq!(
        text,
        "stave 1\n\nregion o = opening(\n  left:   a,\n  bottom: b,\n)\n"
    );
}

#[test]
fn set_an_argument_of_an_assembly_and_a_component_param() {
    let module = "\
stave 1

component Drawer {
  param pull_kind : PullKind = bar_pull(centres: 160mm)
  param box_height = 100mm       // inside
  part front : s {
    length: 1mm
  }
}

assembly drawer[bottom, top] : Drawer {
  opening:   drawer_bay[same]   // each its own bay
  pull_kind: knob
}
";
    let text = applied(
        module,
        "set drawer.pull_kind = bar_pull(centres: 128mm)\nset Drawer.box_height = 120mm\nset Drawer.front.length = 2mm",
    );
    assert!(
        text.contains("  pull_kind: bar_pull(centres: 128mm)\n"),
        "{text}"
    );
    assert!(text.contains("  opening:   drawer_bay[same]   // each its own bay\n"));
    assert!(
        text.contains("  param box_height = 120mm       // inside\n"),
        "{text}"
    );
    assert!(text.contains("    length: 2mm\n"));
}

#[test]
fn set_a_kind_param_and_field() {
    let module = "\
stave 1

kind groove : joint {
  roles { piece: Part, into: Parts }
  param depth = into.thickness / 3
  insert: [push(piece)]
}
";
    let text = applied(
        module,
        "set groove.depth = 6mm\nset groove.insert = [slide(piece)]",
    );
    assert!(text.contains("  param depth = 6mm\n"), "{text}");
    assert!(text.contains("  insert: [slide(piece)]\n"), "{text}");
}

#[test]
fn set_errors_name_what_is_missing() {
    assert_eq!(
        apply(CABINET, "set shelf.width = 1mm").unwrap_err(),
        PatchErrorKind::UnknownItem {
            name: "shelf".to_owned(),
            scope: Scope::Module(ModuleKey::Project)
        }
    );
    assert_eq!(
        apply(CABINET, "set side_l.place.sideways = flush(base.left)").unwrap_err(),
        PatchErrorKind::UnknownMember {
            item: "side_l.place".to_owned(),
            what: "placement axis",
            name: "sideways".to_owned(),
        }
    );
    assert!(matches!(
        apply(CABINET, "set side_r.place.up = ground").unwrap_err(),
        PatchErrorKind::Unaddressable { .. }
    ));
    assert!(matches!(
        apply(CABINET, "set base = 1mm").unwrap_err(),
        PatchErrorKind::Unaddressable { .. }
    ));
}

// ---------------------------------------------------------------------------
// unset
// ---------------------------------------------------------------------------

#[test]
fn unset_a_field_removes_its_line() {
    let text = applied(CABINET, "unset base.width");
    assert!(
        text.contains("part base : oak_19 {\n  length: cabinet_width\n  place {\n"),
        "{text}"
    );
}

#[test]
fn unset_a_named_argument() {
    let text = applied(CABINET, "unset dados.depth");
    assert!(text.contains("joint dados : dado(base, into: [side_l, side_r])\n"));
    let source = "stave 1\n\njoint j : dado(\n  a,\n  into: b,\n  depth: 9mm,\n)\n";
    assert_eq!(
        applied(source, "unset j.depth"),
        "stave 1\n\njoint j : dado(\n  a,\n  into: b,\n)\n"
    );
}

#[test]
fn unset_a_placement_term_keeps_the_comments_around_it() {
    let text = applied(CABINET, "unset side_l.place.across");
    assert!(
        text.contains(
            "    against(base.top)            // stands on the base\n    flush(base.back)\n"
        ),
        "{text}"
    );
}

#[test]
fn unset_a_param_default() {
    let source = "stave 1\n\ncomponent C {\n  param pull : PullKind = knob   // the pull\n}\n";
    assert_eq!(
        applied(source, "unset C.pull"),
        "stave 1\n\ncomponent C {\n  param pull : PullKind   // the pull\n}\n"
    );
    assert!(matches!(
        apply(CABINET, "unset depth").unwrap_err(),
        PatchErrorKind::Unaddressable { .. }
    ));
}

#[test]
fn unset_what_is_not_written_names_it() {
    assert_eq!(
        apply(CABINET, "unset base.note").unwrap_err(),
        PatchErrorKind::NotSet("base.note".to_owned())
    );
    assert_eq!(
        apply(CABINET, "unset dados.width").unwrap_err(),
        PatchErrorKind::NotSet("dados.width".to_owned())
    );
}

// ---------------------------------------------------------------------------
// remove
// ---------------------------------------------------------------------------

#[test]
fn remove_an_item_with_its_doc_comment() {
    let text = applied(CABINET, "remove part base");
    assert!(!text.contains("The bottom of the box"));
    assert!(
        text.contains(
            "stock oak_19 { species: white_oak, form: solid, thickness: 19mm }\n\npart side_l"
        ),
        "{text}"
    );
}

#[test]
fn remove_one_of_a_run_of_one_line_items() {
    let text = applied(CABINET, "remove param depth");
    assert!(text.contains(
        "// Sizes.\nparam cabinet_width = 900mm within 600mm..1200mm   // the outside width\nparam opening       : Region\n"
    ));
}

#[test]
fn remove_checks_the_tag() {
    assert_eq!(
        apply(CABINET, "remove joint base").unwrap_err(),
        PatchErrorKind::WrongTag {
            path: "base".to_owned(),
            expected: "joint",
            found: "part"
        }
    );
}

#[test]
fn remove_an_item_inside_a_component() {
    let source =
        "stave 1\n\ncomponent C {\n  param a = 1mm\n\n  part back : s {\n    length: a\n  }\n}\n";
    assert_eq!(
        applied(source, "remove part C.back"),
        "stave 1\n\ncomponent C {\n  param a = 1mm\n}\n"
    );
}

// ---------------------------------------------------------------------------
// add
// ---------------------------------------------------------------------------

#[test]
fn add_an_item_after_the_last_of_its_kind() {
    let text = applied(CABINET, "add param reveal = 1.5mm");
    assert!(
        text.contains("param opening       : Region\nparam reveal        = 1.5mm\nlet overhang"),
        "aligned with the run, as the formatter would: {text}"
    );
}

#[test]
fn add_a_multi_line_item_is_laid_out() {
    let text = applied(
        CABINET,
        "add part shelf : oak_19 {\n      length:  fit\n  width: depth  // to the back\n      place {\n  against(base.top, gap: 300mm)\n  }\n}",
    );
    assert!(
        text.contains(
            "part side_r = mirror(side_l, about: mid(base, across))\n\npart shelf : oak_19 {\n  length: fit\n  width:  depth // to the back\n  place {\n    against(base.top, gap: 300mm)\n  }\n}\n\njoint dados"
        ),
        "{text}"
    );
}

#[test]
fn add_an_item_with_a_doc_comment_to_a_component() {
    let source =
        "stave 1\n\ncomponent C {\n  param a = 1mm\n\n  part back : s {\n    length: a\n  }\n}\n";
    let text = applied(
        source,
        "add param b = 2mm to C\nadd /// Under the box.\npart bottom : s {\n  length: b\n} to C",
    );
    assert_eq!(
        text,
        "stave 1\n\ncomponent C {\n  param a = 1mm\n  param b = 2mm\n\n  part back : s {\n    length: a\n  }\n\n  /// Under the box.\n  part bottom : s {\n    length: b\n  }\n}\n"
    );
}

#[test]
fn add_refuses_a_name_that_is_taken() {
    assert_eq!(
        apply(CABINET, "add let depth = 1mm").unwrap_err(),
        PatchErrorKind::AlreadyExists {
            name: "depth".to_owned(),
            scope: Scope::Module(ModuleKey::Project)
        }
    );
}

#[test]
fn add_to_something_that_is_not_a_component() {
    assert_eq!(
        apply(CABINET, "add param b = 2mm to base").unwrap_err(),
        PatchErrorKind::NotAComponent("base".to_owned())
    );
}

fn files<'a>(parsed: &'a [(&'a str, Parse<ast::File>)]) -> Vec<ModuleFile<'a>> {
    parsed
        .iter()
        .map(|(name, parse)| ModuleFile { name, parse })
        .collect()
}

#[test]
fn add_picks_the_file_of_the_first_item_it_refers_to() {
    let parsed = [
        (
            "a.stave",
            parse("stave 1\n\nproject \"P\" {\n  units: mm\n}\n\nparam width = 1mm\n"),
        ),
        ("b.stave", parse("stave 1\n\nstock oak { species: oak }\n")),
    ];
    let target = PatchTarget::new(files(&parsed));
    let changed = apply_patch(
        &target,
        &parse_patch(&patch_text("add let half = width / 2\nadd stock ash { species: ash }\nadd let x = 1mm\nadd let y = 2mm in \"c.stave\"")),
    )
    .unwrap();
    let names: Vec<(&str, bool)> = changed
        .iter()
        .map(|file| (file.name.as_str(), file.created))
        .collect();
    assert_eq!(names, [("a.stave", false), ("c.stave", true)]);
    assert_eq!(
        changed[0].text,
        "stave 1\n\nproject \"P\" {\n  units: mm\n}\n\nparam width = 1mm\n\nlet half = width / 2\nlet x    = 1mm\n\nstock ash { species: ash }\n"
    );
    assert_eq!(changed[1].text, "stave 1\n\nlet y = 2mm\n");
    assert_edits_rebuild(&parsed[0].1.syntax().to_string(), &changed[0]);
    assert_edits_rebuild("", &changed[1]);
    assert_eq!(changed[1].edits.len(), 1, "a new file is one insertion");
}

#[test]
fn add_refuses_a_path_as_file_name() {
    assert_eq!(
        apply(CABINET, "add let x = 1mm in \"../x.stave\"").unwrap_err(),
        PatchErrorKind::InvalidFileName("../x.stave".to_owned())
    );
}

#[test]
fn add_and_remove_imports() {
    let source = "stave 1\n\nimport \"./casework\"   // doors and drawers\n\nlet a = 1mm\n";
    let text = applied(source, "add import \"./hardware\" as hw in \"main.stave\"");
    assert_eq!(
        text,
        "stave 1\n\nimport \"./casework\"   // doors and drawers\nimport \"./hardware\" as hw\n\nlet a = 1mm\n"
    );
    assert_eq!(
        applied(source, "remove import \"./casework\""),
        "stave 1\n\nlet a = 1mm\n"
    );
    assert_eq!(
        applied("stave 1\n\nlet a = 1mm\n", "add import \"std:casework\""),
        "stave 1\n\nimport \"std:casework\"\n\nlet a = 1mm\n"
    );
    assert_eq!(applied(source, "add import \"./casework\""), source);
    assert!(matches!(
        apply(source, "remove import \"./nothing\"").unwrap_err(),
        PatchErrorKind::UnknownImport { .. }
    ));
}

// ---------------------------------------------------------------------------
// replace and rename
// ---------------------------------------------------------------------------

#[test]
fn rename_is_not_applied() {
    // Renaming needs every reference to the item, which takes name
    // resolution; this crate has none.
    assert_eq!(
        apply(CABINET, "rename part base to bottom").unwrap_err(),
        PatchErrorKind::NotSupported("rename")
    );
}

#[test]
fn replace_a_one_line_item_keeps_its_neighbours_and_comments() {
    let text = applied(CABINET, "replace let overhang = 3mm");
    assert_eq!(
        text,
        CABINET.replace("let overhang = 20mm", "let overhang = 3mm")
    );
    let text = applied(
        CABINET,
        "replace param cabinet_width = 950mm within 600mm..1000mm",
    );
    assert!(
        text.contains(
            "param cabinet_width = 950mm within 600mm..1000mm   // the outside width\n\
             param depth         = 560mm\n"
        ),
        "{text}"
    );
}

#[test]
fn replace_a_block_keeps_the_doc_comment_unless_it_writes_one() {
    let text = applied(
        CABINET,
        "replace part base : oak_19 {\n  length: 800mm\n  place { ground }\n}",
    );
    assert!(
        text.contains(
            "/// The bottom of the box.\npart base : oak_19 {\n  length: 800mm\n  place { ground }\n}\n\npart side_l"
        ),
        "{text}"
    );
    let text = applied(
        CABINET,
        "replace stock oak_19 {\n  species: white_oak, form: solid, thickness: 18mm\n}",
    );
    assert!(
        text.contains(
            "stock oak_19 {\n  species: white_oak, form: solid, thickness: 18mm\n}\n\n/// The bottom"
        ),
        "{text}"
    );
    let text = applied(
        CABINET,
        "replace /// The floor of the box.\npart base : oak_19 { length: 800mm }",
    );
    assert!(
        text.contains(
            "/// The floor of the box.\npart base : oak_19 { length: 800mm }\n\npart side_l"
        ),
        "{text}"
    );
    assert!(!text.contains("The bottom of the box"));
}

#[test]
fn replace_is_laid_out_for_its_place() {
    let text = applied(
        CABINET,
        "replace scene all_open \"Open\" {\n      explode:   10mm\n      }",
    );
    assert!(
        text.ends_with("scene all_open \"Open\" {\n  explode: 10mm\n}\n"),
        "{text}"
    );
}

#[test]
fn replace_with_the_same_item_changes_nothing() {
    assert_eq!(applied(CABINET, "replace let  overhang=20mm"), CABINET);
}

#[test]
fn replace_refuses_what_is_not_there_or_of_another_kind() {
    assert_eq!(
        apply(CABINET, "replace let nothing = 3mm").unwrap_err(),
        PatchErrorKind::UnknownItem {
            name: "nothing".to_owned(),
            scope: Scope::Module(ModuleKey::Project)
        }
    );
    assert_eq!(
        apply(CABINET, "replace let base = 3mm").unwrap_err(),
        PatchErrorKind::WrongTag {
            path: "base".to_owned(),
            expected: "let",
            found: "part"
        }
    );
}

#[test]
fn a_patch_with_syntax_errors_is_refused() {
    let module = parse(CABINET);
    let mut sources = SourceMap::new();
    let text = "stave 1\npatch \"bad\" {\n  set = 1mm\n}\n";
    let file = sources.add("bad.stave", text).unwrap();
    let patch = parse_patch_file(file, text);
    let target = PatchTarget::new(vec![ModuleFile {
        name: "main.stave",
        parse: &module,
    }]);
    assert!(matches!(
        apply_patch(&target, &patch).unwrap_err().kind,
        PatchErrorKind::Syntax(_)
    ));
}

#[test]
fn a_value_that_breaks_the_file_is_refused() {
    let source = "stave 1\n\ncomponent C {\n  param a = 1mm\n}\n";
    assert!(matches!(
        apply(source, "add private param b = 2mm to C").unwrap_err(),
        PatchErrorKind::BreaksSyntax { ref file, .. } if file == "main.stave"
    ));
    assert!(matches!(
        apply(CABINET, "add part p : oak_19 { length: 1mm } to nowhere").unwrap_err(),
        PatchErrorKind::UnknownItem { .. }
    ));
}

// ---------------------------------------------------------------------------
// The corpus
// ---------------------------------------------------------------------------

fn collect(dir: &Path, files: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect(&path, files);
        } else if path
            .extension()
            .is_some_and(|extension| extension == "stave")
        {
            files.push(path);
        }
    }
}

fn corpus() -> Vec<(String, String)> {
    let root = workspace();
    let mut paths = Vec::new();
    collect(&root.join("examples"), &mut paths);
    collect(&root.join("std"), &mut paths);
    assert!(paths.len() >= 30, "the corpus went missing");
    paths
        .into_iter()
        .map(|path| {
            let text = std::fs::read_to_string(&path).unwrap();
            let name = path
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            (name, text)
        })
        .collect()
}

/// `set` operations that write back, for every param, let, region, field
/// and named kind-call argument at the top level and inside components,
/// the value already there.
fn same_value_ops(text: &str) -> Vec<String> {
    let module = parse(text);
    let root = module.syntax();
    let slice = |node: &stave_lang::SyntaxNode| node.text().to_string();
    let mut ops = Vec::new();
    let visit = |prefix: &str, item: &stave_lang::SyntaxNode, ops: &mut Vec<String>| {
        use stave_lang::SyntaxKind as K;
        let Some(name) = item
            .children()
            .find(|child| child.kind() == K::NAME)
            .map(|name| name.text().to_string())
        else {
            return;
        };
        let value_after = |node: &stave_lang::SyntaxNode, kind: K| {
            let mut seen = false;
            node.children_with_tokens()
                .find_map(|element| match element {
                    rowan::NodeOrToken::Token(token) if token.kind() == kind => {
                        seen = true;
                        None
                    }
                    rowan::NodeOrToken::Node(child) if seen && child.kind().is_expr() => {
                        Some(child)
                    }
                    _ => None,
                })
        };
        match item.kind() {
            K::PARAM | K::LET | K::REGION => {
                if let Some(value) = value_after(item, K::EQ) {
                    ops.push(format!("set {prefix}{name} = {}", slice(&value)));
                }
            }
            K::JOINT | K::HARDWARE => {
                let args = item
                    .children()
                    .find(|child| child.kind() == K::KIND_CALL)
                    .and_then(|call| call.children().find(|child| child.kind() == K::ARG_LIST));
                for arg in args.iter().flat_map(|args| args.children()) {
                    let key = arg.children().find(|child| child.kind() == K::NAME);
                    if let (Some(key), Some(value)) = (key, value_after(&arg, K::COLON)) {
                        ops.push(format!(
                            "set {prefix}{name}.{} = {}",
                            key.text(),
                            slice(&value)
                        ));
                    }
                }
            }
            K::PART | K::STOCK | K::SCENE | K::ASSEMBLY | K::RECIPE | K::PROFILE => {
                let block = item.children().find(|child| child.kind() == K::BLOCK);
                for field in block
                    .iter()
                    .flat_map(|block| block.children())
                    .filter(|child| child.kind() == K::FIELD)
                {
                    let key = field.children().find(|child| child.kind() == K::NAME);
                    if let (Some(key), Some(value)) = (key, value_after(&field, K::COLON)) {
                        ops.push(format!(
                            "set {prefix}{name}.{} = {}",
                            key.text(),
                            slice(&value)
                        ));
                    }
                }
            }
            _ => {}
        }
    };
    for item in root.children() {
        visit("", &item, &mut ops);
        if item.kind() == stave_lang::SyntaxKind::COMPONENT {
            let name = item
                .children()
                .find(|child| child.kind() == stave_lang::SyntaxKind::NAME)
                .map(|name| name.text().to_string())
                .unwrap_or_default();
            let block = item
                .children()
                .find(|child| child.kind() == stave_lang::SyntaxKind::BLOCK);
            for inner in block.iter().flat_map(|block| block.children()) {
                visit(&format!("{name}."), &inner, &mut ops);
            }
        }
    }
    ops
}

#[test]
fn patches_that_change_nothing_leave_every_corpus_file_byte_for_byte() {
    let mut checked = 0;
    for (name, text) in corpus() {
        let module = parse(&text);
        let target = PatchTarget::new(vec![ModuleFile {
            name: "file.stave",
            parse: &module,
        }]);
        let empty = parse_patch("stave 1\n\npatch \"nothing\" { }\n");
        assert_eq!(
            apply_patch(&target, &empty).unwrap(),
            Vec::<PatchedFile>::new(),
            "{name}"
        );

        let ops = same_value_ops(&text);
        checked += ops.len();
        let patch = parse_patch(&patch_text(&ops.join("\n")));
        let changed = apply_patch(&target, &patch)
            .unwrap_or_else(|error| panic!("{name}: {error} at {:?}", error.span));
        assert_eq!(changed, Vec::<PatchedFile>::new(), "{name}");
    }
    assert!(checked > 300, "only {checked} values were written back");
}

fn format(text: &str) -> String {
    let mut sources = SourceMap::new();
    let file = sources.add("f.stave", text).unwrap();
    format_file(file, text).unwrap_or_else(|error| panic!("{error}\n{text}"))
}

#[test]
fn patched_corpus_files_parse_and_format_stably() {
    for (name, text) in corpus() {
        let module = parse(&text);
        let root = module.syntax();
        let mut ops = vec!["add let patch_marker = 1mm".to_owned()];
        // Change the first value of each kind the corpus file has.
        for op in same_value_ops(&text).into_iter().take(6) {
            let (path, _) = op.split_once(" = ").unwrap();
            ops.push(format!("{path} = 1mm + 2mm"));
        }
        let first_part = root
            .children()
            .find(|child| child.kind() == stave_lang::SyntaxKind::PART)
            .and_then(|part| {
                part.children()
                    .find(|child| child.kind() == stave_lang::SyntaxKind::NAME)
            })
            .map(|name| name.text().to_string());
        if let Some(part) = first_part {
            ops.push(format!("set {part}.note = \"patched\""));
        }
        let target = PatchTarget::new(vec![ModuleFile {
            name: "file.stave",
            parse: &module,
        }]);
        let patch = parse_patch(&patch_text(&ops.join("\n")));
        let changed = apply_patch(&target, &patch)
            .unwrap_or_else(|error| panic!("{name}: {error} at {:?}", error.span));
        let [patched] = changed.as_slice() else {
            panic!("{name}: expected one changed file");
        };
        assert_edits_rebuild(&text, patched);
        assert!(
            !parse(&patched.text).has_errors(),
            "{name}: the patched file doesn't parse:\n{}",
            patched.text
        );
        let once = format(&patched.text);
        assert_eq!(format(&once), once, "{name}: formatting is not stable");
        if format(&text) == text {
            // Lines the patch didn't write are untouched, and the lines it
            // wrote are laid out as the formatter would: formatting moves
            // at most the lines aligned with what changed.
            let differing = once
                .lines()
                .zip(patched.text.lines())
                .filter(|(a, b)| a != b)
                .count();
            assert!(
                differing <= 3,
                "{name}: formatting the patched file changed {differing} lines:\n{}",
                patched.text
            );
        }
    }
}

/// The tag a patch writes for a top-level item node.
fn tag_word(kind: stave_lang::SyntaxKind) -> Option<&'static str> {
    use stave_lang::SyntaxKind as K;
    Some(match kind {
        K::PARAM => "param",
        K::LET => "let",
        K::REGION => "region",
        K::STOCK => "stock",
        K::PART => "part",
        K::JOINT => "joint",
        K::HARDWARE => "hardware",
        K::ASSEMBLY => "assembly",
        K::PROFILE => "profile",
        K::COMPONENT => "component",
        K::SCENE => "scene",
        K::REQUIRE => "require",
        K::KIND => "kind",
        K::RECIPE => "recipe",
        _ => return None,
    })
}

/// One patch for each operation the file can take: every operation the
/// patcher applies, each on its own.
fn one_patch_per_operation(text: &str) -> Vec<String> {
    use stave_lang::SyntaxKind as K;
    let module = parse(text);
    let root = module.syntax();
    let name_of = |node: &stave_lang::SyntaxNode| {
        node.children()
            .find(|child| child.kind() == K::NAME)
            .map(|name| name.text().to_string())
    };
    let mut patches = vec![
        "add let patch_marker = 1mm".to_owned(),
        "add import \"./patch_marker\"".to_owned(),
        "add let patch_marker = 1mm\nreplace let patch_marker = 2mm".to_owned(),
    ];
    if let Some(import) = root.children().find(|child| child.kind() == K::IMPORT) {
        let source = import
            .children()
            .find(|child| child.kind() == K::STRING)
            .unwrap()
            .text()
            .to_string();
        patches.push(format!("remove import {source}"));
    }
    if let Some(op) = same_value_ops(text).first() {
        let (path, _) = op.split_once(" = ").unwrap();
        patches.push(format!("{path} = 1mm + 2mm"));
    }
    let items: Vec<_> = root
        .children()
        .filter(|child| tag_word(child.kind()).is_some())
        .collect();
    if let Some(param) = items.iter().find(|item| item.kind() == K::PARAM) {
        patches.push(format!("replace param {} = 7mm", name_of(param).unwrap()));
    }
    let field = items.iter().find_map(|item| {
        let block = item.children().find(|child| child.kind() == K::BLOCK)?;
        let field = block.children().find(|child| child.kind() == K::FIELD)?;
        Some(format!("{}.{}", name_of(item)?, name_of(&field)?))
    });
    if let Some(field) = field {
        patches.push(format!("unset {field}"));
    }
    if let Some(last) = items.last() {
        patches.push(format!(
            "remove {} {}",
            tag_word(last.kind()).unwrap(),
            name_of(last).unwrap()
        ));
    }
    patches
}

#[test]
fn edits_rebuild_the_new_text_for_every_operation_and_every_corpus_file() {
    let mut applied_patches = 0;
    for (name, text) in corpus() {
        let module = parse(&text);
        let target = PatchTarget::new(vec![ModuleFile {
            name: "file.stave",
            parse: &module,
        }]);
        for ops in one_patch_per_operation(&text) {
            let patch = parse_patch(&patch_text(&ops));
            let changed = apply_patch(&target, &patch)
                .unwrap_or_else(|error| panic!("{name}: {ops}: {error}"));
            for file in &changed {
                assert_edits_rebuild(&text, file);
                assert!(
                    file.edits.len() <= 4,
                    "{name}: {ops}: {} edits for one operation",
                    file.edits.len()
                );
            }
            applied_patches += 1;
        }
    }
    assert!(
        applied_patches > 200,
        "only {applied_patches} patches applied"
    );
}

// ---------------------------------------------------------------------------
// The grammar's example
// ---------------------------------------------------------------------------

/// The `patch "Add a shoe shelf" { ... }` example in docs/grammar.md.
fn grammar_example() -> String {
    let grammar = std::fs::read_to_string(workspace().join("docs/grammar.md")).unwrap();
    let start = grammar.find("patch \"Add a shoe shelf\" {").unwrap();
    let end = start + grammar[start..].find("\n}\n").unwrap() + 3;
    format!("stave 1\n\n{}", &grammar[start..end])
}

fn as_files(files: &[(String, Parse<ast::File>)]) -> Vec<ModuleFile<'_>> {
    files
        .iter()
        .map(|(name, parse)| ModuleFile { name, parse })
        .collect()
}

fn module_files(folder: &Path) -> Vec<(String, Parse<ast::File>)> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(folder)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "stave")
        })
        .collect();
    entries.sort();
    entries
        .into_iter()
        .map(|path| {
            let text = std::fs::read_to_string(&path).unwrap();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let parsed = parse(&text);
            (name, parsed)
        })
        .collect()
}

/// The grammar's example less its `rename` line: renaming needs name
/// resolution, which this crate doesn't have, so the patcher refuses it
/// (see `rename_is_not_applied`). Everything else in the example applies.
#[test]
fn the_grammar_example_without_its_rename_line_applies_to_the_clothing_cabinet() {
    let folder = workspace().join("examples/clothing_cabinet");
    let project = module_files(&folder);
    // Drawer and FramePanelDoor moved to std:casework. They are handed over
    // here as a local module all the same: what this test pins is that a
    // patch can reach into a module the caller supplies.
    let casework = module_files(&workspace().join("std/casework"));
    let hardware = module_files(&folder.join("hardware"));
    let target = PatchTarget::new(as_files(&project))
        .with_local_module("casework", as_files(&casework))
        .with_local_module("hw", as_files(&hardware));

    let example = grammar_example();
    // With the rename line, the whole patch is refused at that line.
    assert_eq!(
        apply_patch(&target, &parse_patch(&example))
            .unwrap_err()
            .kind,
        PatchErrorKind::NotSupported("rename")
    );
    let without_rename = example.replace("  rename part side_l to left_side\n", "");
    assert_ne!(without_rename, example);
    let changed = apply_patch(&target, &parse_patch(&without_rename)).unwrap();
    let names: Vec<&str> = changed.iter().map(|file| file.name.as_str()).collect();
    assert_eq!(names, ["carcass.stave", "drawers.stave"]);
    for file in &changed {
        assert_eq!(file.module, ModuleKey::Project);
        let old = std::fs::read_to_string(folder.join(&file.name)).unwrap();
        assert_edits_rebuild(&old, file);
        let parsed = parse(&file.text);
        assert!(
            !parsed.has_errors(),
            "{}: {:?}\n{}",
            file.name,
            parsed.diagnostics,
            file.text
        );
        let once = format(&file.text);
        assert_eq!(format(&once), once);
    }
    let carcass = &changed[0].text;
    assert!(carcass.contains("part shoe_shelf : oak_19 {\n  length: fit\n  width:  cabinet_depth - back.thickness\n  place {\n    against(divider.top, gap: 200mm)\n    center(base, across)\n    flush(side_l.front)\n  }\n}\n"), "{carcass}");
    assert!(carcass.contains(
        "joint shoe_shelf_dados : dado(shoe_shelf, into: [side_l, side_r], depth: 9mm)\n"
    ));
    assert!(carcass.contains(": dado([divider, shelf], into: [side_l, side_r], depth: 8mm)\n"));
    assert!(carcass.contains("    flush(base.back, inset: 6mm)\n"));
    assert!(carcass.contains("    against(base.top)            // stands on the base\n"));
    let drawers = &changed[1].text;
    assert!(!drawers.contains("pull_kind"));
    assert!(drawers.contains("  slide:        side_mount_slide(length: slide_length)\n}\n"));

    // An addition inside a component of a local module.
    let patch = parse_patch(&patch_text(
        "add param handle_gap = 2mm to casework.Drawer\nset casework.Drawer.top_gap = 12mm\nremove part casework.Drawer.sub_front",
    ));
    let changed = apply_patch(&target, &patch).unwrap();
    let [drawer] = changed.as_slice() else {
        panic!("{changed:?}");
    };
    assert_eq!(drawer.module, ModuleKey::Local("casework".to_owned()));
    assert_eq!(drawer.name, "drawer.stave");
    assert!(drawer.text.contains("  param top_gap      = 12mm"));
    assert!(
        drawer.text.contains("  param handle_gap   = 2mm\n"),
        "{}",
        drawer.text
    );
    assert!(!drawer.text.contains("part sub_front"));
    assert!(!parse(&drawer.text).has_errors());
}

// ---------------------------------------------------------------------------
// Fixes
// ---------------------------------------------------------------------------

#[test]
fn a_fix_is_a_patch_that_applies_as_it_is() {
    let fix = stave_lang::Fix::new("Use 8mm deep dados", "set dados.depth = 8mm").unwrap();
    let module = parse(CABINET);
    let target = PatchTarget::new(vec![ModuleFile {
        name: "main.stave",
        parse: &module,
    }]);
    let changed = apply_patch(&target, &parse_patch(&fix.patch_file())).unwrap();
    let [file] = changed.as_slice() else {
        panic!("{changed:?}");
    };
    assert_edits_rebuild(CABINET, file);
    assert!(
        file.text
            .contains("joint dados : dado(base, into: [side_l, side_r], depth: 8mm)\n")
    );
}
