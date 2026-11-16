//! Helpers for the pipeline tests: in-memory projects and exact lengths.

#![allow(dead_code)]

use std::path::Path;

use stave_app::Opened;
use stave_lang::render_diagnostics;
use stave_lang::units::Length;
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

/// Runs the pipeline on in-memory files under `/project`.
pub fn open_files(files: &[(&str, &str)]) -> Opened {
    let mut memory = MemoryFiles::new();
    for (path, text) in files {
        memory.insert(Path::new("/project").join(path), *text);
    }
    stave_app::open_with(Path::new("/project"), &memory).expect("the project loads")
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
