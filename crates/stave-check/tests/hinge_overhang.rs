//! Issue #78: a butt hinge whose leaf hangs off the end of the part it is
//! housed in. The hardware contact rule asks only that a member meet its
//! part somewhere; `butt_hinge`'s `fixed_leaf_on_wood` rule, written with
//! `contact(member, b)` (grammar, "Built-in functions"), keeps the leaf
//! wholly on the wood.
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

/// A leaf part way off the wall breaks the kind's rule, at the hardware
/// call, measuring how much of the leaf the wall holds along its length.
#[test]
fn a_leaf_hanging_off_its_wall_is_reported() {
    for (lid, held) in [(926, "39mm"), (960, "5mm"), (964, "1mm")] {
        let error = only_error(lid);
        assert_eq!(error.code, Code::Require, "lid {lid}");
        assert_eq!(
            error.message,
            "'fixed_leaf_on_wood' fails: the hinge's leaf hangs off the end of the part it is housed in"
        );
        assert!(
            error.notes.contains(&"for hinges[left]".to_owned()),
            "{:?}",
            error.notes
        );
        assert!(
            error
                .notes
                .contains(&format!("the rule measures {held} and needs at least 40mm")),
            "lid {lid}: {:?}",
            error.notes
        );
        let help = format!(
            "wall holds only {held} of the 40mm fixed_leaf; place the hardware so its fixed_leaf lies wholly on wall"
        );
        assert_eq!(error.help.as_deref(), Some(help.as_str()), "lid {lid}");
    }
}

/// At 965mm the leaf lies wholly off the wall, its end only touching the
/// wall's end. The hardware contact rule still counts that as meeting;
/// `contact(fixed_leaf, on)` is the 12mm x 1.5mm end the two touch on, so
/// the rule fails at the hardware call and says only the end touches
/// (before the rule, a 0mm notch was cut in the wall's end grain).
#[test]
fn a_leaf_touching_only_the_walls_end_is_reported() {
    let error = only_error(965);
    assert_eq!(error.code, Code::Require);
    assert_eq!(
        error.message,
        "'fixed_leaf_on_wood' fails: the hinge's leaf hangs off the end of the part it is housed in"
    );
    assert_eq!(
        error.help.as_deref(),
        Some(
            "only the end of the fixed_leaf touches wall; place the hardware so its fixed_leaf lies wholly on wall"
        )
    );
    assert!(
        error.notes.contains(&"for hinges[left]".to_owned()),
        "{:?}",
        error.notes
    );
    let [call, rule] = error.labels.as_slice() else {
        panic!(
            "the hardware call, then the kind's rule: {:?}",
            error.labels
        );
    };
    assert_ne!(call.file, rule.file);
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
