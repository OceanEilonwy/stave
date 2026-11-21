//! The standard kinds' rules that keep a member wholly on the wood it is
//! fixed to, written with `contact(member, b)` (grammar, "Built-in
//! functions"): each fixture checks clean as it is, and with one line
//! changed so the member hangs part way off its part, the kind's rule
//! fails with a help that says how much of it the part holds. Members in
//! drilled holes need no such rule: `error[wall]` already reports a hole
//! that breaks out of its part.

mod common;

use common::check;
use stave_lang::{Code, Diagnostic};

fn fixture(name: &str) -> String {
    let path = format!(
        "{}/tests/fixtures/on_wood/{name}.stave",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// The fixture with `from` (written once) changed to `to`.
fn changed(name: &str, from: &str, to: &str) -> String {
    let text = fixture(name);
    assert_eq!(text.matches(from).count(), 1, "{name}: {from}");
    text.replace(from, to)
}

/// The one error a design has, which must be the rule `rule` failing.
fn rule_fails(text: &str, rule: &str) -> Diagnostic {
    let checked = check(text);
    let errors: Vec<&Diagnostic> = checked
        .diagnostics
        .iter()
        .filter(|d| d.is_error())
        .collect();
    let [error] = errors.as_slice() else {
        panic!("{errors:#?}");
    };
    assert_eq!(error.code, Code::Require, "{error:#?}");
    assert!(
        error.message.starts_with(&format!("'{rule}' fails: ")),
        "{}",
        error.message
    );
    (*error).clone()
}

#[test]
fn the_fixtures_check_clean() {
    for name in [
        "euro_hinge_plate",
        "wide_hinge_plate",
        "butt_hinge_leaf",
        "slide",
    ] {
        let checked = check(&fixture(name));
        let messages: Vec<String> = checked
            .diagnostics
            .iter()
            .map(|d| format!("{}[{}] {}", d.severity, d.code, d.message))
            .collect();
        assert!(messages.is_empty(), "{name}: {messages:#?}");
    }
}

/// A door overlaying the cabinet side, its top hinge 2mm from its top: the
/// mounting plate runs 30.5mm past the top of the side, which holds
/// 19.5mm of it. The plate makes no cut, so nothing else would see it.
#[test]
fn a_cup_hinge_plate_off_the_end_of_its_side() {
    for name in ["euro_hinge_plate", "wide_hinge_plate"] {
        let text = changed(
            name,
            "flush(panel.top, inset: 80mm)",
            "flush(panel.top, inset: 2mm)",
        );
        let error = rule_fails(&text, "plate_on_wood");
        assert_eq!(
            error.help.as_deref(),
            Some(
                "side_l holds only 19.5mm of the 50mm plate; place the hardware so its plate lies wholly on side_l"
            ),
            "{name}"
        );
    }
}

/// A lid 20mm shorter than the wall, the hinges laid out from the wall's
/// ends: the left leaf in the lid has half its length over air, and its
/// notch would be cut 20mm long.
#[test]
fn a_butt_hinge_leaf_off_the_end_of_its_lid() {
    let text = changed("butt_hinge_leaf", "length: 875mm", "length: 855mm");
    let error = rule_fails(&text, "leaf_on_wood");
    assert_eq!(
        error.help.as_deref(),
        Some(
            "lid holds only 20mm of the 40mm leaf; place the hardware so its leaf lies wholly on lid"
        )
    );
}

/// A 500mm slide on a drawer side 300mm long, and on one 25mm tall.
#[test]
fn a_slide_off_its_drawer_side() {
    let longer = changed(
        "slide",
        "  length: 500mm\n  width:  100mm",
        "  length: 300mm\n  width:  100mm",
    );
    let error = rule_fails(&longer, "on_drawer_side");
    assert_eq!(
        error.help.as_deref(),
        Some(
            "drawer_side holds only 300mm of the 500mm drawer_member; place the hardware so its drawer_member lies wholly on drawer_side"
        )
    );
    let taller = changed(
        "slide",
        "  length: 500mm\n  width:  100mm",
        "  length: 500mm\n  width:  25mm",
    );
    let error = rule_fails(&taller, "on_drawer_side");
    assert_eq!(
        error.help.as_deref(),
        Some(
            "drawer_side holds only 25mm of the 45mm drawer_member; place the hardware so its drawer_member lies wholly on drawer_side"
        )
    );
}
