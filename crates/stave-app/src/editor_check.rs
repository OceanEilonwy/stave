//! What an editor gets from one background check (grammar, "Tooling and
//! editors"): the pipeline's diagnostics, each fix of each diagnostic as
//! text edits made by applying its patch to the checked text ("Quick
//! fixes ... as workspace edits made by applying their patches"), and the
//! values worked out in the project's units for hover and inlay hints
//! (`length: fit` shows `= 844mm`, a `between(...)` its value, every size
//! its cut length).
//!
//! It is the slow tier: it runs the whole pipeline, then solves the design
//! once more to read every value off the solver
//! ([`stave_solve::values::worked_out`]). The language server runs it in a
//! background thread after typing pauses.
//!
//! A fix is applied to the text the check read, without being checked
//! again: the patch was written by the checker for that text, and the
//! editor's undo takes it back if the user does not want it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use stave_ide::{Document, ValueKind, ValueNote, WorkedValues};
use stave_lang::units::{ExactAngle, ExactLength, Rational};
use stave_lang::{Fix, TextEdit, apply_patch, parse_patch_file};
use stave_model::trace::Amount;
use stave_modules::{FileSource, LoadError};
use stave_schema::words::OutputUnit;
use stave_solve::values::{ValueSite, WorkedValue};

use crate::Opened;
use crate::scratch::Target;

/// How many different values one note lists before it says how many more
/// there are (an item in a component used many times).
const SHOWN_VALUES: usize = 3;

/// One fix of a diagnostic, as text edits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixEdits {
    /// The fix's label, for its button: "Use 18mm".
    pub label: String,
    /// The files it changes, by path; a file it creates has `created` set
    /// and one insertion.
    pub files: Vec<FixedFile>,
}

/// The edits a fix makes to one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixedFile {
    /// Where the file is.
    pub path: PathBuf,
    /// `true` when the fix creates the file.
    pub created: bool,
    /// The edits to the text the check read, last in the file first (see
    /// [`stave_lang::apply_text_edits`]).
    pub edits: Vec<TextEdit>,
}

/// One background check, for an editor.
#[derive(Debug)]
pub struct EditorCheck {
    /// The pipeline's result.
    pub opened: Opened,
    /// For each of [`Opened::diagnostics`], in the same order, its fixes
    /// that could be applied, in the order the diagnostic gives them.
    pub fixes: Vec<Vec<FixEdits>>,
    /// The values worked out, for hover and inlay hints.
    pub values: WorkedValues,
}

/// Runs the pipeline on `folder` read through `files`, and works out each
/// diagnostic's fixes as text edits and the design's values.
///
/// # Errors
///
/// When the folder can't be loaded at all ([`crate::prepare`]).
pub fn editor_check(folder: &Path, files: &dyn FileSource) -> Result<EditorCheck, LoadError> {
    let (opened, prepared) = crate::prepare(folder, files)?;
    let target = Target::of(&prepared.modules, folder);
    let fixes = opened
        .diagnostics
        .iter()
        .map(|diagnostic| {
            diagnostic
                .fixes
                .iter()
                .filter_map(|fix| fix_edits(&target, fix))
                .collect()
        })
        .collect();
    let values = worked_values(&opened);
    Ok(EditorCheck {
        opened,
        fixes,
        values,
    })
}

/// A fix as text edits, when its patch applies.
fn fix_edits(target: &Target, fix: &Fix) -> Option<FixEdits> {
    let text = fix.patch_file();
    // The patch's own diagnostics are not shown, so its file is in a map
    // of its own.
    let file = stave_lang::SourceMap::new().add("fix.stave", &text).ok()?;
    let patch = parse_patch_file(file, &text);
    let patched = apply_patch(&target.patch_target(), &patch).ok()?;
    let files: Vec<FixedFile> = patched
        .into_iter()
        .filter_map(|file| {
            let folder = target.folders.get(&file.module)?;
            Some(FixedFile {
                path: folder.join(&file.name),
                created: file.created,
                edits: file.edits,
            })
        })
        .collect();
    (!files.is_empty()).then(|| FixEdits {
        label: fix.label().to_owned(),
        files,
    })
}

/// The values of a checked design as notes for the editor, each in the
/// project's units. Values at the same place (an item of a component
/// used by several assemblies, an array's elements) share one note
/// listing the different values.
pub fn worked_values(opened: &Opened) -> WorkedValues {
    let Some(worked) = opened
        .model
        .as_ref()
        .and_then(stave_solve::values::worked_out)
    else {
        return WorkedValues::default();
    };
    let units = worked.units;
    let mut grouped: BTreeMap<(stave_lang::FileId, u32, u32, ValueSite), Vec<&WorkedValue>> =
        BTreeMap::new();
    for value in &worked.values {
        grouped
            .entry((value.file, value.span.start(), value.span.end(), value.site))
            .or_default()
            .push(value);
    }
    let notes = grouped.into_values().filter_map(|values| {
        let first = values.first()?;
        let source = opened.sources.get(first.file)?;
        let text = source.text();
        let covered = |span: stave_lang::ast::Span| {
            text.get(usize::try_from(span.start()).ok()?..usize::try_from(span.end()).ok()?)
        };
        let mut shown: Vec<String> = Vec::new();
        for value in &values {
            let one = match value.cut {
                Some(cut) if cut != value.value => format!(
                    "{}, cut {}",
                    amount_text(value.value, units),
                    amount_text(cut, units)
                ),
                Some(_) | None => amount_text(value.value, units),
            };
            if !shown.contains(&one) {
                shown.push(one);
            }
        }
        let more = shown.len().saturating_sub(SHOWN_VALUES);
        shown.truncate(SHOWN_VALUES);
        let mut text_shown = shown.join("; ");
        if more > 0 {
            text_shown.push_str(&format!("; and {more} more"));
        }
        // A literal size is still worth a hint when its cut length differs.
        let cut_differs = values
            .iter()
            .any(|value| value.cut.is_some_and(|cut| cut != value.value));
        Some(ValueNote {
            document: Document::from_source_path(source.path()),
            span: first.span,
            covers: covered(first.span)?.to_owned(),
            name: first
                .name
                .and_then(|name| Some((name, covered(name)?.to_owned()))),
            kind: match first.site {
                ValueSite::Param => ValueKind::Param,
                ValueSite::Let => ValueKind::Let,
                ValueSite::Size(axis) => ValueKind::Size(axis),
                ValueSite::Between => ValueKind::Between,
            },
            literal: first.literal && values.len() == 1 && !cut_differs,
            text: text_shown,
        })
    });
    WorkedValues::new(notes)
}

/// A worked-out amount in the project's units: `844mm`, `33.25in`,
/// `110deg`, `4`.
pub fn amount_text(amount: Amount, units: OutputUnit) -> String {
    match amount {
        Amount::Length(length) => length_text(length, units),
        Amount::Angle(angle) => angle_text(angle),
        Amount::Count(number) | Amount::Ratio(number) => number.to_string(),
    }
}

/// A length in the project's units, to a thousandth of the unit, trailing
/// zeros trimmed: `844mm`, `33.25in`, `about 281.333mm` for a length that
/// is not a whole number of thousandths.
pub fn length_text(length: ExactLength, units: OutputUnit) -> String {
    let nanometres_per_unit: i64 = match units {
        OutputUnit::Millimetre => 1_000_000,
        OutputUnit::Centimetre => 10_000_000,
        OutputUnit::Metre => 1_000_000_000,
        OutputUnit::Inch => 25_400_000,
        OutputUnit::Foot => 304_800_000,
    };
    let suffix = units.unit().suffix();
    let thousandths = length
        .value()
        .checked_mul(Rational::from(1000_i64))
        .and_then(|scaled| scaled.checked_div(Rational::from(nanometres_per_unit)));
    let Some(thousandths) = thousandths else {
        return format!("{}nm", length.value());
    };
    let rounded = thousandths.round();
    let exact = Rational::new(rounded, 1) == Some(thousandths);
    let sign = if rounded < 0 { "-" } else { "" };
    let magnitude = rounded.unsigned_abs();
    let (whole, fraction) = (magnitude / 1000, magnitude % 1000);
    let number = if fraction == 0 {
        format!("{sign}{whole}")
    } else {
        let digits = format!("{fraction:03}");
        format!("{sign}{whole}.{}", digits.trim_end_matches('0'))
    };
    if exact {
        format!("{number}{suffix}")
    } else {
        format!("about {number}{suffix}")
    }
}

/// An angle in degrees: `110deg`, `about 33.333333deg`.
fn angle_text(angle: ExactAngle) -> String {
    match (angle.to_angle(), angle.to_nearest_angle()) {
        (Some(whole), _) => whole.to_string(),
        (None, Some(nearest)) => format!("about {nearest}"),
        (None, None) => format!("{}µdeg", angle.value()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stave_lang::units::Length;

    #[test]
    fn lengths_are_shown_in_the_project_units() {
        let mm = |value: i64| Length::from_mm(value).to_exact();
        assert_eq!(length_text(mm(844), OutputUnit::Millimetre), "844mm");
        assert_eq!(length_text(mm(844), OutputUnit::Centimetre), "84.4cm");
        assert_eq!(length_text(mm(1200), OutputUnit::Metre), "1.2m");
        let inches = ExactLength::new(Rational::from(33_i64 * 25_400_000 + 6_350_000));
        assert_eq!(length_text(inches, OutputUnit::Inch), "33.25in");
        assert_eq!(length_text(mm(-19), OutputUnit::Millimetre), "-19mm");
        let third = ExactLength::new(Rational::new(844_000_000, 3).unwrap());
        assert_eq!(
            length_text(third, OutputUnit::Millimetre),
            "about 281.333mm"
        );
    }
}
