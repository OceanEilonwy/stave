//! Issue #78: a piece of hardware whose member hangs almost entirely off the
//! part that carries it is accepted, and `no-contact` fires only once the
//! member leaves the part completely.
//!
//! The design: a 900mm wall, a lid lying on its top edge flush with its
//! right end, and two butt hinges spaced 25mm in from the lid's ends. A lid
//! longer than 925mm carries its left hinge past the wall's left end.

mod common;

use common::{Checked, check, mm};
use stave_lang::Code;
use stave_lang::placement::Axis;
use stave_lang::units::Length;
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

fn codes(checked: &Checked) -> Vec<String> {
    checked
        .diagnostics
        .iter()
        .map(|d| format!("{}[{}] {}", d.severity, d.code, d.message))
        .collect()
}

/// The control: a lid no longer than the wall less the inset keeps both
/// leaves wholly on the wall, and the design is clean.
#[test]
fn both_leaves_wholly_on_the_wall_is_clean() {
    let checked = check(&design(900));
    let design = checked.clean();
    for which in ["left", "right"] {
        let (on_wall, length) = fixed_leaf_on_wall(design, which);
        assert_eq!(on_wall, length, "hinges[{which}]");
        assert_eq!(length, mm(40));
    }
}

/// Probe: at 960mm the left leaf's shape is the full 40mm but only 5mm of
/// it lies over the wall. The member itself is not moved or clipped; only
/// its mortise is (the footprint is "the region the shape's box and the
/// part's cut box share", grammar, "Faces to cut").
#[test]
fn probe_the_left_leaf_is_5mm_on_the_wall_at_960mm() {
    let checked = check(&design(960));
    let design = checked.design.as_ref().expect("solved");
    assert_eq!(fixed_leaf_on_wall(design, "left"), (mm(5), mm(40)));
    assert_eq!(fixed_leaf_on_wall(design, "right"), (mm(40), mm(40)));
}

/// Probe: where the check flips. `no-contact` comes only when nothing of
/// the leaf is left on the wall: the solver's `check_contact` asks that a
/// member "touches or enters" its part (`meets`: the boxes share an area),
/// which a 1mm sliver does.
#[test]
fn probe_no_contact_fires_only_once_the_leaf_is_wholly_off() {
    let no_contact = |lid: i64| {
        check(&design(lid))
            .diagnostics
            .iter()
            .any(|d| d.code == Code::NoContact)
    };
    // The lid's right end is flush with the wall's, so the left leaf runs
    // from 925mm - lid to 965mm - lid past the wall's left end: 1mm on the
    // wall at 964mm, none at 965mm.
    let mut rows = Vec::new();
    for lid in [924, 925, 940, 960, 964, 965, 966, 970, 979, 980, 981] {
        let checked = check(&design(lid));
        let on_wall = checked
            .design
            .as_ref()
            .map(|design| fixed_leaf_on_wall(design, "left").0.to_mm_f64());
        rows.push(format!(
            "lid {lid}: left leaf on wall {on_wall:?}mm, no-contact {}",
            no_contact(lid)
        ));
    }
    eprintln!("{}", rows.join("\n"));
    let checked = check(&design(964));
    let design = checked.design.as_ref().expect("solved");
    assert_eq!(fixed_leaf_on_wall(design, "left"), (mm(1), mm(40)));
    assert!(!no_contact(964), "1mm on the wall passes the contact rule");
}

/// At 965mm the left leaf is wholly off the wall: its end face only lies
/// on the wall's end face. `meets` counts that as touching (the boxes
/// share an area on two axes), so `no-contact` stays quiet, and
/// `crossing_face` picks the wall's end, where the mortise becomes a notch
/// in the end grain 0mm long. It fails today: nothing is reported.
#[test]
#[ignore = "issue #78: fails until hardware checks a member is supported by its part"]
fn a_leaf_touching_only_the_walls_end_is_reported() {
    let checked = check(&design(965));
    let design = checked.design.as_ref().expect("solved");
    assert_eq!(fixed_leaf_on_wall(design, "left"), (mm(0), mm(40)));
    assert!(
        !codes(&checked).is_empty(),
        "hinges[left]'s fixed_leaf lies wholly off the wall, and nothing is reported"
    );
}

/// The behaviour issue #78 asks for: a hardware member that hangs off the
/// part carrying it (here 35mm of a 40mm leaf over air) is reported. It
/// fails today: the design checks with no diagnostics at all.
#[test]
#[ignore = "issue #78: fails until hardware checks a member is supported by its part"]
fn a_leaf_hanging_off_its_wall_is_reported() {
    let checked = check(&design(960));
    let messages = codes(&checked);
    assert!(
        !messages.is_empty(),
        "hinges[left]'s fixed_leaf has 35mm of its 40mm over air, and nothing is reported"
    );
}
