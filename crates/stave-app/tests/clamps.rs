//! Executable spec: clamps (grammar, "Clamps"). A hardware kind that says
//! `clamp a, b along w` draws member `a` and member `b` together when it
//! is tightened, so the stack between `a` and the part `b` moves with must
//! be solid, or it is `error[no-contact]` at the hardware's call.
//!
//! The corpus is the five turning-frames designs
//! (`docs/reviews/2026-11-22-turning-frames.md`, appendix), in
//! `tests/clamps/`: each bolts one wooden part beside another with a
//! `knob_bolt` across a 3mm gap with nothing in it, and its kind says
//! `clamp knob, bolt along depth`. As written each is an error for every
//! bolt; with the gap closed, or a 3mm washer member in it, each checks
//! clean; with a 1mm washer each is an error again.

mod common;

use common::{HEADER, open_files, open_files_with_every_warning};
use stave_app::Opened;
use stave_lang::{Code, render_diagnostics};

/// The five designs, and how many knob bolts each has.
const DESIGNS: [(&str, usize); 5] = [
    ("cheval", 2),
    ("stool", 2),
    ("drafting_table", 2),
    ("sawhorse", 2),
    ("glider", 4),
];

fn fixture(name: &str) -> String {
    let path = format!("{}/tests/clamps/{name}.stave", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn open(text: &str) -> Opened {
    open_files_with_every_warning(&[("project.stave", text)])
}

/// A fragment of a project, whose parts nothing joins: loose parts are
/// not reported.
fn fragment(text: &str) -> Opened {
    open_files(&[("project.stave", text)])
}

fn rendered(opened: &Opened) -> String {
    render_diagnostics(&opened.sources, &opened.diagnostics)
}

/// The design with the gap closed: each part placed against the other,
/// and the bolt's `span` 0mm.
fn closed(text: &str) -> String {
    assert!(text.contains(", gap: 3mm)") && text.contains("span: 3mm"));
    text.replace(", gap: 3mm)", ")")
        .replace("span: 3mm", "span: 0mm")
}

/// The design with a washer member of this thickness on the knob's part,
/// between the two parts.
fn with_washer(text: &str, thickness: &str) -> String {
    let anchor = "  cut bolt_hole:";
    assert_eq!(text.matches(anchor).count(), 1);
    text.replace(
        anchor,
        &format!(
            "  /// A steel washer in the gap, on the knob's part.
  member washer {{
    with:  piece
    shape: cylinder(diameter: 16mm, depth: piece.thickness..piece.thickness + {thickness})
  }}

{anchor}"
        ),
    )
}

/// Every diagnostic is `error[no-contact]`, one for each bolt.
fn every_bolt_has_a_gap(name: &str, opened: &Opened, bolts: usize) {
    assert_eq!(
        opened.diagnostics.len(),
        bolts,
        "{name}:\n{}",
        rendered(opened)
    );
    for diagnostic in &opened.diagnostics {
        assert_eq!(
            diagnostic.code,
            Code::NoContact,
            "{name}:\n{}",
            rendered(opened)
        );
        assert!(diagnostic.is_error());
    }
}

#[test]
fn a_bolt_across_an_empty_gap_is_no_contact() {
    for (name, bolts) in DESIGNS {
        let opened = open(&fixture(name));
        every_bolt_has_a_gap(name, &opened, bolts);
        insta::assert_snapshot!(format!("gap_{name}"), rendered(&opened));
    }
}

#[test]
fn the_gap_closed_checks_clean() {
    for (name, _) in DESIGNS {
        let opened = open(&closed(&fixture(name)));
        assert!(
            opened.diagnostics.is_empty(),
            "{name}:\n{}",
            rendered(&opened)
        );
    }
}

#[test]
fn a_washer_that_fills_the_gap_checks_clean() {
    for (name, _) in DESIGNS {
        let opened = open(&with_washer(&fixture(name), "3mm"));
        assert!(
            opened.diagnostics.is_empty(),
            "{name}:\n{}",
            rendered(&opened)
        );
    }
}

#[test]
fn a_washer_too_thin_for_the_gap_is_no_contact() {
    for (name, bolts) in DESIGNS {
        let opened = open(&with_washer(&fixture(name), "1mm"));
        every_bolt_has_a_gap(name, &opened, bolts);
        insta::assert_snapshot!(format!("thin_washer_{name}"), rendered(&opened));
    }
}

#[test]
fn a_kind_without_a_clamp_is_checked_as_before() {
    for (name, _) in DESIGNS {
        let text = fixture(name);
        let without = text.replace("  clamp knob, bolt along depth\n", "");
        assert_ne!(without, text);
        let opened = open(&without);
        assert!(
            opened.diagnostics.is_empty(),
            "{name}:\n{}",
            rendered(&opened)
        );
    }
}

/// Two posts side by side, 3mm apart, and a knob bolt from the left
/// post's left face into the right post. `ENTRIES` is the kind's last
/// entries and `CALL` the hardware's call.
fn posts(entries: &str, call: &str) -> String {
    format!(
        "{HEADER}
part post_l : pine_19 {{
  length: 300mm
  width:  60mm
  place {{
    orient(length: up, width: back)
    against(base.top)
    flush(base.left)
    flush(base.front)
  }}
}}
part post_r : pine_19 {{
  length: 300mm
  width:  60mm
  place {{
    orient(length: up, width: back)
    against(base.top)
    against(post_l.right, gap: 3mm)
    flush(base.front)
  }}
}}
kind knob_bolt : fitting {{
  roles {{
    piece: Part
    into:  Part
  }}
  param span = 3mm
  mount {{
    face:   front
    across: open
    along:  open
  }}
  /// The knob.
  member knob {{
    with:  piece
    shape: cylinder(diameter: 30mm, depth: -10mm..0mm)
  }}
  /// The bolt.
  member bolt {{
    with:  into
    shape: cylinder(diameter: 8mm, depth: 0mm..piece.thickness + span + 10mm)
  }}
  cut bolt_hole: crossing(piece, bolt) {{ drill(diameter: 9mm, depth: through, wall: 3mm) }}
  cut insert_hole: crossing(into, bolt) {{ drill(diameter: 12mm, depth: 10mm, wall: 3mm) }}
{entries}
}}
{call}
"
    )
}

const CALL: &str = "hardware lock : knob_bolt(post_l, into: post_r, face: left) {
  place {
    center(post_l, deep)
    flush(post_l.top, inset: 100mm)
  }
}";

#[test]
fn the_message_names_the_parts_either_side_of_the_gap() {
    let opened = fragment(&posts("  clamp knob, bolt along depth", CALL));
    let [only] = opened.diagnostics.as_slice() else {
        panic!("{}", rendered(&opened));
    };
    assert_eq!(only.code, Code::NoContact);
    assert_eq!(
        only.message,
        "lock draws post_r toward post_l, but they are 3mm apart with nothing between"
    );
    assert_eq!(
        only.notes,
        vec!["tightening the knob would pull post_r 3mm toward post_l".to_owned()]
    );
    assert_eq!(
        only.help.as_deref(),
        Some("put a washer or spacer in the gap, or place post_r against post_l")
    );
}

#[test]
fn the_other_elements_of_an_array_are_a_note() {
    let call = "hardware locks[upper, lower] : knob_bolt(post_l, into: post_r, face: left) {
  place {
    center(post_l, deep)
    spaced(ends(inset: 50mm), from: post_l.top, to: post_l.bottom)
  }
}";
    let opened = fragment(&posts("  clamp knob, bolt along depth", call));
    let [only] = opened.diagnostics.as_slice() else {
        panic!("{}", rendered(&opened));
    };
    assert_eq!(only.code, Code::NoContact);
    assert_eq!(
        only.message,
        "locks[upper] draws post_r toward post_l, but they are 3mm apart with nothing between"
    );
    assert!(
        only.notes.contains(&"the same for locks[lower]".to_owned()),
        "{:?}",
        only.notes
    );
}

#[test]
fn a_clamp_drawn_the_other_way_finds_the_same_gap() {
    // From the right post's right face, back across the gap into the left
    // post: the line runs down the axis.
    let call = "hardware lock : knob_bolt(post_r, into: post_l, face: right) {
  place {
    center(post_r, deep)
    flush(post_r.top, inset: 100mm)
  }
}";
    let opened = fragment(&posts("  clamp knob, bolt along depth", call));
    let [only] = opened.diagnostics.as_slice() else {
        panic!("{}", rendered(&opened));
    };
    assert_eq!(
        only.message,
        "lock draws post_l toward post_r, but they are 3mm apart with nothing between"
    );
}

/// The one diagnostic of a project, as `code: message`.
fn only(opened: &Opened) -> String {
    let [only] = opened.diagnostics.as_slice() else {
        panic!("{}", rendered(opened));
    };
    format!("{}: {}", only.code, only.message)
}

#[test]
fn a_clamp_names_two_members_with_a_body() {
    let opened = fragment(&posts("  clamp knob, nut along depth", CALL));
    assert_eq!(only(&opened), "unknown-name: unknown name 'nut'");
}

#[test]
fn a_clamp_names_two_different_members() {
    let opened = fragment(&posts("  clamp bolt, bolt along depth", CALL));
    assert_eq!(
        only(&opened),
        "type-mismatch: 'bolt' is named twice in one clamp"
    );
}

#[test]
fn a_clamp_runs_along_a_frame_axis() {
    let opened = fragment(&posts("  clamp knob, bolt along up", CALL));
    assert_eq!(
        only(&opened),
        "type-mismatch: a clamp draws along along, across or depth, found 'up'"
    );
}

#[test]
fn a_kind_has_one_clamp() {
    let opened = fragment(&posts(
        "  clamp knob, bolt along depth\n  clamp knob, bolt along depth",
        CALL,
    ));
    let duplicate = opened
        .diagnostics
        .iter()
        .find(|d| d.code == Code::DuplicateName)
        .unwrap_or_else(|| panic!("{}", rendered(&opened)));
    assert_eq!(duplicate.message, "'clamp' is declared twice");
}

#[test]
fn the_drawn_member_moves_with_a_part() {
    let text = posts("  clamp bolt, knob along depth", CALL).replace(
        "  member knob {\n    with:  piece\n",
        "  member knob {\n    with:  bolt\n",
    );
    let opened = fragment(&text);
    // The knob now rides on the bolt, which moves with a part: clean
    // wording aside, it is the bolt's part that is drawn.
    assert!(
        opened
            .diagnostics
            .iter()
            .all(|d| d.code != Code::TypeMismatch),
        "{}",
        rendered(&opened)
    );
    let region = posts("  clamp knob, bolt along depth", CALL)
        .replace(
            "    into:  Part\n",
            "    into:  Part\n    opening: Region\n",
        )
        .replace("    with:  into\n", "    with:  opening\n");
    let opened = fragment(&region);
    let mismatch = opened
        .diagnostics
        .iter()
        .find(|d| d.code == Code::TypeMismatch)
        .unwrap_or_else(|| panic!("{}", rendered(&opened)));
    assert_eq!(
        mismatch.message,
        "clamp knob, bolt: 'bolt' moves with no part, so there is nothing for it to draw"
    );
}

#[test]
fn a_joint_kind_has_no_clamp() {
    let opened = fragment(&format!(
        "{HEADER}
kind tied : joint {{
  clamp a, b along depth
}}"
    ));
    assert!(
        opened
            .diagnostics
            .iter()
            .any(|d| d.code == Code::UnknownField
                && d.message == "a joint kind has no field 'clamp'"),
        "{}",
        rendered(&opened)
    );
}
