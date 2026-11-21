//! Issue #78: a butt hinge whose leaf hangs off the end of the part it is
//! housed in. The hardware contact rule asks only that a member meet its
//! part somewhere; a member housed in a cut is seated, and a seated member
//! lies wholly on its part (grammar, "Cuts stay in their part",
//! `error[overhang]`), whatever its kind says.
//!
//! The design: a 900mm wall, a lid lying on its top edge flush with its
//! right end, and two butt hinges spaced 25mm in from the lid's ends. A lid
//! longer than 925mm carries its left hinge past the wall's left end: the
//! left leaf runs from `925mm - lid` to `965mm - lid` past it.

mod common;

use common::{Checked, check, mm};
use stave_lang::placement::Axis;
use stave_lang::units::Length;
use stave_lang::{Code, Diagnostic};
use stave_model::solved::{MemberShape, SolvedDesign};

fn design(lid_length_mm: i64) -> String {
    format!(
        r#"stave 1

project "Hinge overhang" {{
  units: mm
}}

param lid_length = {lid_length_mm}mm

stock cedar {{ species: cedar, form: solid, thickness: 19mm, max_width: 190mm, max_length: 2400mm }}

part wall : cedar {{
  length: 900mm
  width:  100mm
  place {{ ground }}
}}

part lid : cedar {{
  length: lid_length
  width:  100mm
  place {{
    against(wall.top)
    flush(wall.right)
    flush(wall.front)
  }}
}}

hardware hinges[left, right] : butt_hinge(lid, on: wall, length: 40mm, face: bottom) {{
  place {{
    spaced(ends(inset: 25mm), from: lid.left, to: lid.right)
  }}
}}
"#
    )
}

/// How much of `hinges[which]`'s `fixed_leaf` lies over the wall along the
/// wall's length (world `across`), and the leaf's own length there.
fn fixed_leaf_on_wall(design: &SolvedDesign, which: &str) -> (Length, Length) {
    let wall = common::drawn(design, "wall");
    let hinge = design
        .hardware()
        .iter()
        .find(|h| h.name == format!("hinges[{which}]"))
        .unwrap_or_else(|| panic!("no hinges[{which}]"));
    let leaf = hinge
        .members
        .iter()
        .find(|m| m.name == "fixed_leaf")
        .expect("a fixed_leaf member");
    let [MemberShape::Box(leaf_box)] = leaf.shapes() else {
        panic!("fixed_leaf is one box: {:?}", leaf.shapes());
    };
    let axis = Axis::Across;
    let low = leaf_box.min().get(axis).max(wall.min().get(axis));
    let high = leaf_box.max().get(axis).min(wall.max().get(axis));
    let on_wall = if high > low {
        Length::from_nm(high.nm() - low.nm())
    } else {
        Length::ZERO
    };
    (on_wall, leaf_box.size(axis))
}

/// The errors of a design.
fn errors(checked: &Checked) -> Vec<&Diagnostic> {
    checked
        .diagnostics
        .iter()
        .filter(|d| d.is_error())
        .collect()
}

/// The one error of a design, which must have exactly one.
fn only_error(lid: i64) -> Diagnostic {
    let checked = check(&design(lid));
    let errors = errors(&checked);
    let [error] = errors.as_slice() else {
        panic!("lid {lid}: {errors:#?}");
    };
    (*error).clone()
}

/// Both leaves wholly on the wall: clean, up to the last millimetre.
#[test]
fn both_leaves_wholly_on_the_wall_is_clean() {
    for lid in [900, 925] {
        let checked = check(&design(lid));
        let design = checked.clean();
        for which in ["left", "right"] {
            assert_eq!(
                fixed_leaf_on_wall(design, which),
                (mm(40), mm(40)),
                "lid {lid}, hinges[{which}]"
            );
        }
    }
}

/// A leaf part way off the wall is `error[overhang]` at the hardware
/// call, saying how much of the leaf the wall holds.
#[test]
fn a_leaf_hanging_off_its_wall_is_reported() {
    for (lid, held) in [(926, "39mm"), (960, "5mm"), (964, "1mm")] {
        let error = only_error(lid);
        assert_eq!(error.code, Code::Overhang, "lid {lid}");
        assert_eq!(
            error.message,
            "hinges[left]: its fixed_leaf hangs off the end of wall"
        );
        assert_eq!(
            error.notes,
            vec![format!("wall holds {held} of the 40mm fixed_leaf")],
            "lid {lid}"
        );
        assert_eq!(
            error.help.as_deref(),
            Some("place hinges[left] so its fixed_leaf lies wholly on wall")
        );
    }
}

/// At 965mm the leaf lies wholly off the wall, its end only touching the
/// wall's end. The hardware contact rule still counts that as meeting,
/// but a housed member must enter its part: before, a 0mm notch was cut
/// in the wall's end grain.
#[test]
fn a_leaf_touching_only_the_walls_end_is_reported() {
    let error = only_error(965);
    assert_eq!(error.code, Code::Overhang);
    assert_eq!(
        error.message,
        "hinges[left]: its fixed_leaf hangs off the end of wall"
    );
    assert_eq!(
        error.notes,
        vec!["wall holds 0mm of the 40mm fixed_leaf".to_owned()]
    );
}

/// Wholly off the wall, the hardware contact rule reports it as before.
#[test]
fn a_leaf_wholly_off_its_wall_meets_nothing() {
    let error = only_error(1200);
    assert_eq!(error.code, Code::NoContact);
    assert_eq!(
        error.message,
        "hinges[left]: lid meets none of the parts it should join"
    );
}
