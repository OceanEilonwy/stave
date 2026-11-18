//! Signature matching and patch writing on small projects built here, in
//! memory: every kind of selection, the project's own components, settings
//! copied from the most recent assembly or chosen, kinds written as joints.

use std::path::PathBuf;

use stave_lang::ast::{Expr, ExprKind, Ident, Span};
use stave_lang::placement::Face;
use stave_modules::MemoryFiles;

use super::{EditScope, WriteError};
use crate::Opened;
use crate::edit::{
    Applicability, Candidate, Declaration, FacingPair, MenuGroup, PartFace, Picked, PlaceSlot,
    SettingValue,
};
use stave_model::nonempty::NonEmpty;

const PROJECT: &str = r#"stave 1

project "Test box" {
  units: mm
}

param width = 600mm

stock oak_19 { species: white_oak, form: solid, thickness: 19mm, max_width: 200mm, max_length: 2400mm }
stock ply_6  { species: birch, form: plywood, thickness: 6mm, max_length: 2440mm }

part base : oak_19 {
  length: width
  width:  180mm
  place {
    ground
  }
}

part side_l : oak_19 {
  length: 400mm
  width:  180mm
  place {
    orient(length: up, width: back)
    against(base.top)
    flush(base.left)
    flush(base.back)
  }
}

part side_r = mirror(side_l, about: mid(base, across))

region inside = opening(
  left:   side_l.right,
  right:  side_r.left,
  bottom: base.top,
  top:    side_l.top,
  front:  side_l.front,
  back:   side_l.back,
)

/// Rests on a face. Holds books.
component Ledge {
  param on    : Anchor
  param piece : Part
  param wood  : Stock
  param depth = 200mm
  require deep_enough : depth >= 10mm "the ledge is too shallow"
}

/// Fills a space with something.
component Filler {
  param space : Region
  param wood  : Stock
}

/// Sits on a face, but names no part.
component Pad {
  param on    : Anchor
  param wood  : Stock
}

/// Hangs from a part, but names no face.
component Hook {
  param from  : Part
  param wood  : Stock
}

/// Ties parts together.
component Tie {
  param ends  : Parts
  param wood  : Stock
}

assembly ledge : Ledge {
  on:    base.top
  piece: base
  wood:  oak_19
}
assembly ledge_2 : Ledge {
  on:    side_l.right
  piece: side_l
  wood:  ply_6
}
"#;

fn folder() -> PathBuf {
    PathBuf::from("/test/box")
}

fn files(text: &str) -> MemoryFiles {
    MemoryFiles::new().with(folder().join("box.stave"), text)
}

fn open(text: &str) -> Opened {
    let opened = crate::open_with(&folder(), &files(text)).unwrap();
    let errors: Vec<_> = opened
        .diagnostics
        .iter()
        .filter(|d| d.is_error() && d.code.layer() <= 2)
        .collect();
    assert!(errors.is_empty(), "the test project resolves: {errors:#?}");
    opened
}

fn part(opened: &Opened, name: &str) -> stave_model::ids::PartId {
    let model = opened.model.as_ref().unwrap();
    model.parts.iter().find(|p| p.name == name).unwrap().id
}

fn find<'a>(menu: &'a [Candidate], label: &str) -> Option<&'a Candidate> {
    menu.iter().find(|c| c.entry.label.as_str() == label)
}

fn labels(menu: &[Candidate]) -> Vec<&str> {
    menu.iter().map(|c| c.entry.label.as_str()).collect()
}

/// Applies a written patch to the one-file project and opens the result,
/// returning its text and its errors of layers 1 and 2.
fn apply(scope: &EditScope<'_>, patch: &super::WrittenPatch) -> (String, Vec<String>) {
    let changed = stave_lang::apply_patch(&scope.patch_target(), &patch.parse)
        .unwrap_or_else(|e| panic!("{e}\n{}", patch.text));
    let [only] = changed.as_slice() else {
        panic!("one file changed")
    };
    assert_eq!(only.name, "box.stave");
    let opened = crate::open_with(&folder(), &files(&only.text)).unwrap();
    let errors = opened
        .diagnostics
        .iter()
        .filter(|d| d.is_error() && d.code.layer() <= 2)
        .map(|d| d.message.clone())
        .collect();
    (only.text.clone(), errors)
}

fn name_expr(name: &str) -> Expr {
    Expr {
        kind: ExprKind::Name(Ident::new(name, Span::default()).unwrap()),
        span: Span::default(),
    }
}

#[test]
fn a_face_fills_an_anchor_and_a_part_param_and_copies_from_the_latest_assembly() {
    let opened = open(PROJECT);
    let scope = EditScope::new(&opened).unwrap();
    let face = PartFace {
        part: part(&opened, "base"),
        face: Face::Top,
    };
    let menu = scope.candidates(&Picked::Face(face));
    let ledge = find(&menu, "Ledge").unwrap_or_else(|| panic!("{:?}", labels(&menu)));
    assert!(find(&menu, "Filler").is_none());
    assert!(find(&menu, "Tie").is_none());
    // A face is two typed values, its Anchor and its Part, and a
    // component is listed when either fills a place param (the face rule).
    let pad = find(&menu, "Pad").unwrap_or_else(|| panic!("{:?}", labels(&menu)));
    assert_eq!(pad.place[0].slot, PlaceSlot::Anchor(Some(face)));
    let hook = find(&menu, "Hook").unwrap_or_else(|| panic!("{:?}", labels(&menu)));
    assert_eq!(hook.place[0].slot, PlaceSlot::Part(Some(face.part)));
    assert_eq!(ledge.entry.group, MenuGroup::Project);
    assert_eq!(ledge.entry.hint.as_deref(), Some("Rests on a face."));
    assert_eq!(ledge.place[0].slot, PlaceSlot::Anchor(Some(face)));
    assert_eq!(ledge.place[1].slot, PlaceSlot::Part(Some(face.part)));
    // The project's own components come first.
    assert_eq!(
        menu.first().map(|c| &c.entry.group),
        Some(&MenuGroup::Project)
    );
    // wood: from ledge_2, the last Ledge in the file.
    let SettingValue::Copied { value, from } = &ledge.settings[0].value else {
        panic!("copied: {:?}", ledge.settings[0]);
    };
    assert_eq!(super::print::expr_text(value), "ply_6");
    let model = scope.model();
    assert_eq!(model.assemblies.get(from.index()).unwrap().name, "ledge_2");
    assert_eq!(ledge.settings[1].value, SettingValue::Default);

    // A kind has no Anchor role, but its first part role takes the face's
    // part: the prelude's pulls are offered on a face.
    let knob = find(&menu, "knob").unwrap_or_else(|| panic!("{:?}", labels(&menu)));
    assert!(matches!(knob.declaration, Declaration::Kind(_)));
    assert_eq!(knob.place[0].slot, PlaceSlot::Part(Some(face.part)));

    let written = scope.write_patch(ledge, None).unwrap();
    assert_eq!(written.name.as_str(), "ledge_3");
    assert_eq!(written.file, "box.stave");
    assert_eq!(
        written.text,
        "stave 1\n\npatch \"Add a ledge\" {\n  add assembly ledge_3 : Ledge {\n    on:    base.top\n    piece: base\n    wood:  ply_6\n  } in \"box.stave\"\n}\n"
    );
    let (text, errors) = apply(&scope, &written);
    assert!(errors.is_empty(), "{errors:?}\n{text}");
}

#[test]
fn a_region_with_nothing_to_copy_waits_for_a_choice() {
    let opened = open(PROJECT);
    let scope = EditScope::new(&opened).unwrap();
    let model = scope.model();
    let inside = model
        .regions
        .iter()
        .find(|r| r.name == "inside")
        .unwrap()
        .id;
    let menu = scope.candidates(&Picked::Region(inside));
    let filler = find(&menu, "Filler").unwrap_or_else(|| panic!("{:?}", labels(&menu)));
    assert!(find(&menu, "Ledge").is_none());
    // A slide's `in` role takes the region; its piece is still to point at.
    let slide = find(&menu, "side_mount_slide").unwrap();
    assert_eq!(slide.place[1].slot, PlaceSlot::Region(Some(inside)));
    assert_eq!(slide.place[0].slot, PlaceSlot::Part(None));

    assert_eq!(filler.settings[0].value, SettingValue::Unset);
    assert!(!filler.is_complete());
    assert_eq!(
        scope.write_patch(filler, None).unwrap_err(),
        WriteError::Incomplete(vec!["wood".to_owned()])
    );
    let mut filler = filler.clone();
    filler.settings[0].value = SettingValue::Chosen(name_expr("oak_19"));
    let written = scope.write_patch(&filler, None).unwrap();
    assert!(written.text.contains("  add assembly filler : Filler {\n    space: inside\n    wood:  oak_19\n  } in \"box.stave\"\n"), "{}", written.text);
    let (text, errors) = apply(&scope, &written);
    assert!(errors.is_empty(), "{errors:?}\n{text}");
}

#[test]
fn facing_parts_fill_parts_params_and_a_joint_s_roles() {
    let opened = open(PROJECT);
    let scope = EditScope::new(&opened).unwrap();
    let (side_l, side_r, base) = (
        part(&opened, "side_l"),
        part(&opened, "side_r"),
        part(&opened, "base"),
    );
    let pair = FacingPair::new(side_l, side_r).unwrap();
    let menu = scope.candidates(&Picked::Between(pair));
    let tie = find(&menu, "Tie").unwrap_or_else(|| panic!("{:?}", labels(&menu)));
    assert_eq!(
        tie.place[0].slot,
        PlaceSlot::Parts(NonEmpty::new(vec![side_l, side_r]))
    );
    let dado = find(&menu, "dado").unwrap();
    assert_eq!(
        dado.place[0].slot,
        PlaceSlot::Parts(NonEmpty::new(vec![side_l]))
    );
    assert_eq!(
        dado.place[1].slot,
        PlaceSlot::Parts(NonEmpty::new(vec![side_r]))
    );
    assert!(
        menu.iter()
            .all(|c| c.applicability == Applicability::NotChecked)
    );

    let written = scope.write_patch(dado, None).unwrap();
    assert_eq!(written.name.as_str(), "dado_2");
    assert!(
        written
            .text
            .contains("  add joint dado_2 : dado(side_l, into: side_r) in \"box.stave\"\n"),
        "{}",
        written.text
    );
    let (text, errors) = apply(&scope, &written);
    assert!(errors.is_empty(), "{errors:?}\n{text}");

    // A group: the joint's last `Parts` role takes the parts left over.
    let group = Picked::Group(NonEmpty::new(vec![side_l, side_r, base]).unwrap());
    let menu = scope.candidates(&group);
    let dado = find(&menu, "dado").unwrap();
    assert_eq!(
        dado.place[1].slot,
        PlaceSlot::Parts(NonEmpty::new(vec![side_r, base]))
    );
    let tie = find(&menu, "Tie").unwrap();
    assert_eq!(
        tie.place[0].slot,
        PlaceSlot::Parts(NonEmpty::new(vec![side_l, side_r, base]))
    );
    // A knob has one `Part` role, so three parts don't fit it.
    assert!(find(&menu, "knob").is_none());
}

#[test]
fn a_split_project_adds_to_the_file_of_what_it_refers_to_with_its_imports() {
    let main = r#"stave 1

project "Split" {
  units: mm
}

stock oak_19 { species: white_oak, form: solid, thickness: 19mm, max_width: 200mm, max_length: 2400mm }

part base : oak_19 {
  length: 600mm
  width:  180mm
  place {
    ground
  }
}
"#;
    let shelves = r#"stave 1

import "./shop" as shop

assembly first : shop.Rest {
  on:    base.top
  piece: base
  wood:  oak_19
}
"#;
    let shop_library = r#"stave 1

library "Shop" {
}

/// A rest.
component Rest {
  param on    : Anchor
  param piece : Part
  param wood  : Stock
}
"#;
    let other = r#"stave 1

region nothing_here = opening(left: base.left, right: base.right, bottom: base.top, top: base.top, front: base.front, back: base.back)
"#;
    let files = MemoryFiles::new()
        .with(folder().join("a_main.stave"), main)
        .with(folder().join("b_shelves.stave"), shelves)
        .with(folder().join("c_other.stave"), other)
        .with(folder().join("shop/library.stave"), shop_library);
    let opened = crate::open_with(&folder(), &files).unwrap();
    let errors: Vec<_> = opened
        .diagnostics
        .iter()
        .filter(|d| d.is_error() && d.code.layer() <= 2)
        .collect();
    assert!(errors.is_empty(), "{errors:#?}");
    let scope = EditScope::new(&opened).unwrap();
    let face = PartFace {
        part: part(&opened, "base"),
        face: Face::Front,
    };
    let menu = scope.candidates(&Picked::Face(face));
    let rest = find(&menu, "Rest").unwrap_or_else(|| panic!("{:?}", labels(&menu)));
    assert_eq!(
        rest.entry.group,
        MenuGroup::Module(Ident::new("shop", Span::default()).unwrap())
    );
    let written = scope.write_patch(rest, None).unwrap();
    // base is in a_main.stave, which doesn't import the shop yet.
    assert_eq!(written.file, "a_main.stave");
    assert!(
        written
            .text
            .contains("  add import \"./shop\" as shop in \"a_main.stave\"\n  add assembly rest : shop.Rest {\n"),
        "{}",
        written.text
    );
    let changed = stave_lang::apply_patch(&scope.patch_target(), &written.parse).unwrap();
    let [only] = changed.as_slice() else {
        panic!("one file")
    };
    let files = files.with(folder().join(&only.name), only.text.clone());
    let reopened = crate::open_with(&folder(), &files).unwrap();
    let errors: Vec<_> = reopened
        .diagnostics
        .iter()
        .filter(|d| d.is_error() && d.code.layer() <= 2)
        .collect();
    assert!(errors.is_empty(), "{errors:#?}\n{}", only.text);
}
