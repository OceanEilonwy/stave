//! Helpers for the pipeline tests: in-memory projects and exact lengths.

#![allow(dead_code)]

use std::path::Path;

use stave_app::Opened;
use stave_lang::units::Length;
use stave_lang::{Diagnostic, render_diagnostics};
use stave_model::solved::{Part, SolvedDesign};
use stave_modules::MemoryFiles;

/// The start of every test project: a header, a stock and a ground part
/// `base` 600 x 300 x 19 lying flat at the origin.
pub const HEADER: &str = "stave 1

project \"Test\" {
  units: mm
}

stock pine_19 { species: pine, form: solid, thickness: 19mm }
stock pine_12 { species: pine, form: solid, thickness: 12mm }

part base : pine_19 {
  length: 600mm
  width:  300mm
  place {
    ground
  }
}
";

/// Runs the pipeline on in-memory files under `/project`, with every
/// diagnostic.
pub fn open_files_with_every_warning(files: &[(&str, &str)]) -> Opened {
    let mut memory = MemoryFiles::new();
    for (path, text) in files {
        memory.insert(Path::new("/project").join(path), *text);
    }
    stave_app::open_with(Path::new("/project"), &memory).expect("the project loads")
}

/// Runs the pipeline on in-memory files under `/project`. The projects
/// here are fragments that place a few parts to test one rule, mostly
/// without joining them and with stocks that don't say how long their
/// boards are, so `warning[loose-part]` and the `warning[missing-field]`
/// for a stock with no `max_length` are left out; the held check and the
/// stock warning have their own tests (`walls.rs`, `splices.rs`,
/// `tests/broken`), which use [`open_files_with_every_warning`].
pub fn open_files(files: &[(&str, &str)]) -> Opened {
    let mut opened = open_files_with_every_warning(files);
    opened
        .diagnostics
        .retain(|d| d.code != stave_lang::Code::LoosePart && !is_no_max_length_warning(d));
    opened
}

/// `true` for the warning that a stock doesn't say how long, or how wide,
/// its boards (or sheets) are.
pub fn is_no_max_length_warning(d: &Diagnostic) -> bool {
    d.code == stave_lang::Code::MissingField
        && !d.is_error()
        && (d.message.contains("doesn't say how long its")
            || d.message.contains("doesn't say how wide its"))
}

/// Runs the pipeline on one file: [`HEADER`] followed by `body`.
pub fn open(body: &str) -> Opened {
    open_files(&[("project.stave", &format!("{HEADER}\n{body}"))])
}

/// The diagnostics as `code: message`.
pub fn diagnostics(opened: &Opened) -> Vec<String> {
    opened
        .diagnostics
        .iter()
        .map(|d| format!("{}: {}", d.code, d.message))
        .collect()
}

/// The solved design; panics with the rendered diagnostics if there is none.
pub fn design(opened: &Opened) -> &SolvedDesign {
    match &opened.design {
        Some(design) if opened.diagnostics.iter().all(|d| !d.is_error()) => design,
        _ => panic!(
            "no design:\n{}",
            render_diagnostics(&opened.sources, &opened.diagnostics)
        ),
    }
}

/// The solved design of a test about placement or joints (layers 2 and
/// 3), whose made-up parts may well pass through each other or not touch:
/// errors of the fit checks (`overlap`, `no-contact`, `motion`, `wall`,
/// `cut-size`) are allowed, any other error panics.
pub fn solved(opened: &Opened) -> &SolvedDesign {
    let fit = |d: &stave_lang::Diagnostic| {
        matches!(
            d.code,
            stave_lang::Code::Overlap
                | stave_lang::Code::NoContact
                | stave_lang::Code::Motion
                | stave_lang::Code::Wall
                | stave_lang::Code::CutSize
        )
    };
    match &opened.design {
        Some(design) if opened.diagnostics.iter().all(|d| !d.is_error() || fit(d)) => design,
        _ => panic!(
            "no design:\n{}",
            render_diagnostics(&opened.sources, &opened.diagnostics)
        ),
    }
}

/// Exactly one error, with this code and message.
pub fn only_error(opened: &Opened, code: &str, message: &str) {
    let all = diagnostics(opened);
    assert_eq!(
        all,
        vec![format!("{code}: {message}")],
        "{}",
        render_diagnostics(&opened.sources, &opened.diagnostics)
    );
}

/// An exact length from millimetres written as a decimal: `mm("1104.75")`.
pub fn mm(text: &str) -> Length {
    let negative = text.starts_with('-');
    let digits = text.trim_start_matches('-');
    let (whole, fraction) = digits.split_once('.').unwrap_or((digits, ""));
    let mut nm: i64 = whole.parse::<i64>().unwrap() * 1_000_000;
    let mut scale = 100_000;
    for digit in fraction.chars() {
        nm += i64::from(digit.to_digit(10).unwrap()) * scale;
        scale /= 10;
    }
    Length::from_nm(if negative { -nm } else { nm })
}

/// A part by name.
pub fn part<'d>(design: &'d SolvedDesign, name: &str) -> &'d Part {
    design
        .part_named(name)
        .unwrap_or_else(|| panic!("no part '{name}'"))
}

/// A part's drawn box as `[across, up, deep]` ranges.
pub fn extent(part: &Part) -> [(Length, Length); 3] {
    let bounds = part.drawn_box();
    [
        (bounds.min().across, bounds.max().across),
        (bounds.min().up, bounds.max().up),
        (bounds.min().deep, bounds.max().deep),
    ]
}

/// `[across, up, deep]` ranges from millimetre strings.
pub fn ranges(values: [(&str, &str); 3]) -> [(Length, Length); 3] {
    values.map(|(a, b)| (mm(a), mm(b)))
}

/// Opens [`HEADER`] followed by `body`, checks that it solves without any
/// diagnostic, and that it has at least one `require` and every one holds:
/// the way the spec tests assert evaluated values (`require r : 1-1/2in ==
/// 38.1mm "..."`).
pub fn holds(body: &str) {
    let opened = open(body);
    assert!(
        opened.diagnostics.is_empty(),
        "{}",
        render_diagnostics(&opened.sources, &opened.diagnostics)
    );
    let design = design(&opened);
    assert!(!design.requires.is_empty(), "no require to check");
    for require in &design.requires {
        assert!(
            require.holds,
            "'{}' fails: {}",
            require.name, require.message
        );
    }
}

/// Exactly one diagnostic, with this code and message and a help line;
/// returns it so a test can look at its line, help or notes.
pub fn the_diagnostic<'o>(opened: &'o Opened, code: &str, message: &str) -> &'o Diagnostic {
    only_error(opened, code, message);
    let diagnostic = &opened.diagnostics[0];
    assert!(
        diagnostic.help.is_some(),
        "every diagnostic has a help line: {}",
        render_diagnostics(&opened.sources, &opened.diagnostics)
    );
    diagnostic
}

/// The 1-based line of a diagnostic's primary label, counted in the file
/// the label is in.
pub fn line(opened: &Opened, diagnostic: &Diagnostic) -> usize {
    let label = diagnostic.primary().expect("a primary label");
    let file = opened.sources.get(label.file).expect("the file");
    file.line_column(label.span.start()).line
}

/// The 1-based line, within `body`, of a diagnostic in a design opened with
/// [`open`] (which puts [`HEADER`] and a blank line before the body).
pub fn body_line(opened: &Opened, diagnostic: &Diagnostic) -> usize {
    line(opened, diagnostic) - HEADER.lines().count() - 1
}

/// The help line of the only diagnostic.
pub fn help(opened: &Opened) -> String {
    let [only] = opened.diagnostics.as_slice() else {
        panic!(
            "expected one diagnostic:\n{}",
            render_diagnostics(&opened.sources, &opened.diagnostics)
        );
    };
    only.help.clone().unwrap_or_default()
}

/// A region by name.
pub fn region<'d>(design: &'d SolvedDesign, name: &str) -> &'d stave_model::solved::Region {
    design
        .regions()
        .iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("no region '{name}'"))
}

/// A box as `[across, up, deep]` ranges.
pub fn bounds(world: &stave_model::solved::WorldBox) -> [(Length, Length); 3] {
    [
        (world.min().across, world.max().across),
        (world.min().up, world.max().up),
        (world.min().deep, world.max().deep),
    ]
}

/// A joint by name.
pub fn joint<'d>(design: &'d SolvedDesign, name: &str) -> &'d stave_model::solved::Joint {
    design
        .joints()
        .iter()
        .find(|j| j.name == name)
        .unwrap_or_else(|| panic!("no joint '{name}'"))
}

/// A piece of hardware by name.
pub fn hardware<'d>(design: &'d SolvedDesign, name: &str) -> &'d stave_model::solved::Hardware {
    design
        .hardware()
        .iter()
        .find(|h| h.name == name)
        .unwrap_or_else(|| panic!("no hardware '{name}'"))
}

/// A piece of hardware's role by name.
pub fn role(
    hardware: &stave_model::solved::Hardware,
    name: &str,
) -> Option<stave_model::solved::RoleValue> {
    hardware
        .roles
        .iter()
        .find(|(role, _)| role.as_str() == name)
        .map(|(_, value)| *value)
}

/// A piece of hardware's member by name, as its world box at rest.
pub fn member_bounds(
    hardware: &stave_model::solved::Hardware,
    name: &str,
) -> stave_model::solved::WorldBox {
    hardware
        .members
        .iter()
        .find(|member| member.name == name)
        .unwrap_or_else(|| panic!("{} has no member '{name}'", hardware.name))
        .bounds()
}
