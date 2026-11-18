//! `stave edit` without a window: the edit loop as a script of steps
//! (`docs/editing.md`, "The command line"). The same steps come from the
//! command line's flags (`--region`, `--choose`, `--set`, `--commit`, ...)
//! or from a script file (`--script`), one step a line:
//!
//! ```text
//! # a hat shelf above the divider
//! select region divider.above      # or: face door_l.stile[hinge].front,
//! list                             #     between side_l side_r, parts a b c
//! choose casework.Shelf
//! bind parts side_l side_r         # point at what the card asks for
//! set headroom = 400mm
//! name sweater_shelf
//! show
//! commit                           # or: cancel; fix <problem> <fix>
//! ```
//!
//! Each step of a script is echoed (`> list`) before what it prints, so
//! the output reads as a transcript; the flags print the same without the
//! echo. The output is the test harness, so it is stable: entries in menu
//! order, paths as given on the command line, diagnostics as `stave
//! check` prints them. The run stops at the first step that fails (a name
//! that isn't in the design, a command the edit rejects, a commit of a
//! candidate that doesn't fit), and `stave edit` then exits with 1.

use std::fmt::Write as _;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::{Result, bail};
use stave_app::Opened;
use stave_app::edit::{
    Applicability, Candidate, FacingPair, MenuGroup, PartFace, Picked, PlaceSlot, SettingValue,
    Verdict,
};
use stave_app::editing::{
    CommitError, Edit, EditCommand, EditEvent, Progress, entry_matches, parse_name, parse_value,
};
use stave_app::scratch::DiskWriter;
use stave_lang::placement::Face;
use stave_lang::render_diagnostics;
use stave_model::nonempty::NonEmpty;
use stave_modules::DiskFiles;

/// What a step points at, by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    /// `region divider.above`
    Region(String),
    /// `face door_l.stile[hinge].front`: a part and one of its faces.
    Face(String, Face),
    /// `between side_l side_r`: two facing parts.
    Between(String, String),
    /// `parts side_l side_r`: a group of parts.
    Parts(Vec<String>),
}

/// One step of the edit loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Start an edit from a selection.
    Select(Selection),
    /// Print the menu with each entry's applicability.
    List,
    /// Choose a menu entry by its label (`Shelf`) or with its group
    /// (`casework.Shelf`).
    Choose(String),
    /// Point at what the card asks for.
    Bind(Selection),
    /// Set a setting: its name and the value as typed.
    Set(String, String),
    /// Name the new item.
    Name(String),
    /// Apply a refusal's fix: the problem and the fix, counted from 1.
    Fix(usize, usize),
    /// Print the card: the candidate, its verdict and its patch.
    Show,
    /// Write the candidate into the files.
    Commit,
    /// Drop the edit.
    Cancel,
}

/// Reads a selection: `region <name>`, `face <part>.<face>`, `between
/// <part> <part>`, `parts <part>...`.
///
/// # Errors
///
/// Anything else, said in words.
pub fn parse_selection(words: &[&str]) -> Result<Selection> {
    match words {
        ["region", name] => Ok(Selection::Region((*name).to_owned())),
        ["face", anchor] => face(anchor),
        ["between", first, second] => Ok(Selection::Between(
            (*first).to_owned(),
            (*second).to_owned(),
        )),
        ["parts", parts @ ..] if !parts.is_empty() => Ok(Selection::Parts(
            parts.iter().map(|part| (*part).to_owned()).collect(),
        )),
        _ => bail!(
            "a selection is 'region <name>', 'face <part>.<face>', 'between <part> <part>' or 'parts <part>...', not '{}'",
            words.join(" ")
        ),
    }
}

/// Reads `door_l.stile[hinge].front` as a part and a face.
///
/// # Errors
///
/// When the last segment is not a face's name.
pub fn face(anchor: &str) -> Result<Selection> {
    let Some((part, word)) = anchor.rsplit_once('.') else {
        bail!("a face is written '<part>.<face>', such as 'side_l.right', not '{anchor}'");
    };
    let Some(face) = Face::from_word(word) else {
        let words: Vec<&str> = Face::ALL.iter().map(|face| face.word()).collect();
        bail!(
            "'{word}' is not a face; a face is one of {}",
            words.join(", ")
        );
    };
    Ok(Selection::Face(part.to_owned(), face))
}

/// Reads a script: one step a line, `#` starting a comment.
///
/// # Errors
///
/// A line that is not a step, with its line number.
pub fn parse_script(text: &str) -> Result<Vec<(String, Step)>> {
    let mut steps = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.split_once('#').map_or(line, |(code, _)| code).trim();
        if line.is_empty() {
            continue;
        }
        let step = parse_step(line)
            .map_err(|error| anyhow::anyhow!("line {}: {error:#}", index.saturating_add(1)))?;
        steps.push((line.to_owned(), step));
    }
    Ok(steps)
}

fn parse_step(line: &str) -> Result<Step> {
    let words: Vec<&str> = line.split_whitespace().collect();
    match words.as_slice() {
        ["select", rest @ ..] => Ok(Step::Select(parse_selection(rest)?)),
        ["list"] => Ok(Step::List),
        ["choose", label] => Ok(Step::Choose((*label).to_owned())),
        ["bind", rest @ ..] => Ok(Step::Bind(parse_selection(rest)?)),
        ["set", ..] => {
            let rest = line.trim_start_matches("set").trim();
            setting(rest)
        }
        ["name", name] => Ok(Step::Name((*name).to_owned())),
        ["fix", problem, fix] => Ok(Step::Fix(count(problem)?, count(fix)?)),
        ["show"] => Ok(Step::Show),
        ["commit"] => Ok(Step::Commit),
        ["cancel"] => Ok(Step::Cancel),
        _ => bail!(
            "'{line}' is not a step: select, list, choose, bind, set, name, fix, show, commit or cancel"
        ),
    }
}

/// Reads `headroom = 400mm` (or `headroom=400mm`) as a setting's name and
/// value.
///
/// # Errors
///
/// When there is no `=`.
pub fn setting(text: &str) -> Result<Step> {
    let Some((name, value)) = text.split_once('=') else {
        bail!("a setting is written '<name> = <value>', such as 'headroom = 400mm', not '{text}'");
    };
    Ok(Step::Set(name.trim().to_owned(), value.trim().to_owned()))
}

fn count(word: &str) -> Result<usize> {
    match word.parse::<usize>() {
        Ok(number) if number > 0 => Ok(number),
        _ => bail!("'{word}' is not a number counted from 1"),
    }
}

/// Runs the steps on the project in `folder`, printing to `out`. `echo`
/// prints each step's line before its output. `Ok(false)` when a step
/// failed (the steps after it are not run).
///
/// # Errors
///
/// Only when the output can't be written.
pub fn run(
    folder: &Path,
    steps: &[(String, Step)],
    echo: bool,
    out: &mut dyn std::io::Write,
) -> Result<bool> {
    let mut runner = Runner {
        folder,
        edit: None,
        echo,
    };
    for (line, step) in steps {
        let mut text = String::new();
        if echo {
            writeln!(text, "> {line}")?;
        }
        let fine = match runner.step(step, &mut text) {
            Ok(fine) => fine,
            Err(error) => {
                writeln!(text, "error: {error:#}")?;
                false
            }
        };
        out.write_all(text.as_bytes())?;
        if !fine {
            return Ok(false);
        }
    }
    Ok(true)
}

struct Runner<'a> {
    folder: &'a Path,
    edit: Option<Edit>,
    /// A script's transcript: each step says where the candidate stands.
    /// Without it (the flags) only failures and the card are printed.
    echo: bool,
}

impl Runner<'_> {
    fn step(&mut self, step: &Step, out: &mut String) -> Result<bool> {
        let verbose = self.echo;
        match step {
            Step::Select(selection) => {
                // The selection's names are looked up in the design as it
                // is on disk now.
                let opened = stave_app::open(self.folder)?;
                let picked = picked(&opened, selection)?;
                self.edit = None;
                let edit = Edit::start(self.folder, Arc::new(DiskFiles), picked)?;
                writeln!(
                    out,
                    "{}: {}",
                    edit.picked().heading().text(),
                    selection_text(selection)
                )?;
                writeln!(out, "{} menu entries", edit.menu().len())?;
                self.edit = Some(edit);
                Ok(true)
            }
            Step::List => {
                let edit = self.edit()?;
                edit.check_menu(&AtomicBool::new(false))
                    .map_err(|_| anyhow::anyhow!("the check was stopped"))?;
                list(edit, out)?;
                Ok(true)
            }
            Step::Choose(label) => {
                let edit = self.edit()?;
                let found: Vec<usize> = edit
                    .menu()
                    .iter()
                    .enumerate()
                    .filter(|(_, candidate)| entry_matches(candidate, label))
                    .map(|(index, _)| index)
                    .collect();
                let index = match found.as_slice() {
                    [only] => *only,
                    [] => bail!("the menu has no entry '{label}'"),
                    _ => bail!(
                        "'{label}' names more than one entry; write it with its group, such as 'casework.{label}'"
                    ),
                };
                let events = edit.apply(EditCommand::Choose(index));
                outcome(edit, &events, verbose, out)
            }
            Step::Bind(selection) => {
                let edit = self.edit()?;
                let picked = picked(edit.base(), selection)?;
                let events = edit.apply(EditCommand::Bind(picked));
                outcome(edit, &events, verbose, out)
            }
            Step::Set(name, value) => {
                let edit = self.edit()?;
                let command = EditCommand::Set {
                    name: parse_name(name)?,
                    value: parse_value(value)?,
                };
                let events = edit.apply(command);
                outcome(edit, &events, verbose, out)
            }
            Step::Name(name) => {
                let edit = self.edit()?;
                let events = edit.apply(EditCommand::Rename(parse_name(name)?));
                outcome(edit, &events, verbose, out)
            }
            Step::Fix(problem, fix) => {
                let edit = self.edit()?;
                let events = edit.apply(EditCommand::ApplyFix {
                    problem: problem.saturating_sub(1),
                    fix: fix.saturating_sub(1),
                });
                outcome(edit, &events, verbose, out)
            }
            Step::Show => {
                let edit = self.edit()?;
                card(edit, out)?;
                Ok(true)
            }
            Step::Commit => {
                let edit = self.edit()?;
                match edit.commit(&mut DiskWriter) {
                    Ok(commit) => {
                        for file in &commit.files {
                            let verb = if file.created { "created" } else { "wrote" };
                            writeln!(out, "{verb} {}", file.path.display())?;
                        }
                        let errors = commit.opened.error_count();
                        writeln!(
                            out,
                            "committed: the design checks with {errors} {}",
                            if errors == 1 { "error" } else { "errors" }
                        )?;
                        self.edit = None;
                        Ok(true)
                    }
                    Err(error @ (CommitError::NothingChosen | CommitError::DoesNotFit)) => {
                        writeln!(out, "not committed: {error}")?;
                        Ok(false)
                    }
                    Err(error @ CommitError::Write { .. }) => Err(error.into()),
                }
            }
            Step::Cancel => {
                if self.edit.take().is_none() {
                    bail!("nothing is being edited");
                }
                writeln!(out, "cancelled: nothing was written")?;
                Ok(true)
            }
        }
    }

    fn edit(&mut self) -> Result<&mut Edit> {
        match self.edit.as_mut() {
            Some(edit) => Ok(edit),
            None => {
                bail!("nothing is being edited: start with a selection ('select region <name>')")
            }
        }
    }
}

/// Prints what a command did: a rejection fails the step; otherwise,
/// when `verbose`, the chosen candidate's state in one line.
fn outcome(edit: &Edit, events: &[EditEvent], verbose: bool, out: &mut String) -> Result<bool> {
    for event in events {
        if let EditEvent::Rejected(rejection) = event {
            writeln!(out, "rejected: {}", rejection.text())?;
            return Ok(false);
        }
    }
    if let Some(chosen) = edit.chosen().filter(|_| verbose) {
        writeln!(out, "{}", progress_line(chosen.progress()))?;
    }
    Ok(true)
}

/// `fits`, `refused: 2 errors`, `needs a choice: sides`.
fn progress_line(progress: &Progress) -> String {
    match progress {
        Progress::NeedsChoice(names) => format!("needs a choice: {}", names_text(names)),
        Progress::Unwritable(reason) => format!("can't be written: {reason}"),
        Progress::Tried(Verdict::Fits(fits)) => match fits.warnings().len() {
            0 => "fits".to_owned(),
            1 => "fits, with 1 warning".to_owned(),
            count => format!("fits, with {count} warnings"),
        },
        Progress::Tried(Verdict::Refused(refusal)) => match refusal.problems().count() {
            1 => "refused: 1 error".to_owned(),
            count => format!("refused: {count} errors"),
        },
        Progress::Tried(Verdict::Checking) => "checking".to_owned(),
    }
}

fn names_text(names: &NonEmpty<stave_lang::ast::Name>) -> String {
    names
        .iter()
        .map(|name| name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// The menu, one entry a line in menu order, numbered from 1, with its
/// applicability; a refused entry's errors follow it, one a line.
fn list(edit: &Edit, out: &mut String) -> Result<()> {
    let width = edit
        .menu()
        .iter()
        .map(|candidate| entry_name(candidate).len())
        .max()
        .unwrap_or(0);
    for (index, candidate) in edit.menu().iter().enumerate() {
        let name = entry_name(candidate);
        let pad = " ".repeat(width - name.len());
        let state = match &candidate.applicability {
            Applicability::NotChecked => "not checked".to_owned(),
            Applicability::Applies => "applies".to_owned(),
            Applicability::NeedsChoice(names) => format!("needs a choice: {}", names_text(names)),
            Applicability::Greyed(_) => "refused".to_owned(),
        };
        writeln!(out, "{:>3}. {name}{pad}  {state}", index + 1)?;
        if let Applicability::Greyed(problems) = &candidate.applicability {
            for problem in problems.iter() {
                writeln!(
                    out,
                    "       {}[{}]: {}",
                    problem.severity, problem.code, problem.message
                )?;
            }
        }
    }
    Ok(())
}

/// How the command line names a menu entry: `casework.Shelf`, or the bare
/// label for the project's own declarations.
fn entry_name(candidate: &Candidate) -> String {
    let label = candidate.entry.label.as_str();
    match &candidate.entry.group {
        MenuGroup::Project => label.to_owned(),
        MenuGroup::Module(group) => format!("{}.{label}", group.as_str()),
        MenuGroup::Standard(group) => format!("{}.{label}", group.as_str()),
    }
}

/// The card: the chosen candidate, what fills each of its params, where it
/// stands, the checker's words, and its patch.
fn card(edit: &Edit, out: &mut String) -> Result<()> {
    let Some(chosen) = edit.chosen() else {
        bail!("choose something from the menu first");
    };
    let candidate = chosen.candidate();
    let name = chosen
        .item_name()
        .map_or("(not named yet)", |name| name.as_str());
    writeln!(
        out,
        "{} as {name} ({})",
        entry_name(candidate),
        edit.picked().heading().text()
    )?;
    let rows: Vec<(String, String)> = candidate
        .place
        .iter()
        .map(|param| {
            (
                param.name.as_str().to_owned(),
                place_text(edit.base(), &param.slot),
            )
        })
        .chain(candidate.settings.iter().map(|setting| {
            let value = match &setting.value {
                SettingValue::Default => "default".to_owned(),
                SettingValue::Copied { from, value } => {
                    let from = edit
                        .base()
                        .model
                        .as_ref()
                        .and_then(|model| model.assemblies.get(from.index()))
                        .map_or("?", |assembly| assembly.name.as_str());
                    format!(
                        "{} (copied from {from})",
                        stave_app::candidates::print::expr_text(value)
                    )
                }
                SettingValue::Chosen(value) => stave_app::candidates::print::expr_text(value),
                SettingValue::Unset => "choose one".to_owned(),
            };
            (setting.name.as_str().to_owned(), value)
        }))
        .collect();
    let width = rows.iter().map(|(key, _)| key.len()).max().unwrap_or(0);
    for (key, value) in &rows {
        writeln!(out, "  {key}{}  {value}", " ".repeat(width - key.len()))?;
    }
    writeln!(out, "verdict: {}", progress_line(chosen.progress()))?;
    if let Some(tried) = chosen.tried() {
        out.push_str(&render_diagnostics(tried.sources(), &tried.shown()));
        if let Progress::Tried(Verdict::Refused(refusal)) = chosen.progress() {
            for (number, problem) in refusal.problems().iter().enumerate() {
                for (fix_number, fix) in problem.diagnostic().fixes.iter().enumerate() {
                    writeln!(
                        out,
                        "fix {} {}: {}",
                        number + 1,
                        fix_number + 1,
                        fix.label()
                    )?;
                }
            }
        }
    }
    if let Some(patch) = chosen.patch() {
        writeln!(out, "patch:")?;
        for line in patch.lines() {
            if line.is_empty() {
                writeln!(out)?;
            } else {
                writeln!(out, "  {line}")?;
            }
        }
    }
    Ok(())
}

/// A place param's value as written, or that it is still to point at.
fn place_text(opened: &Opened, slot: &PlaceSlot) -> String {
    let Some(model) = opened.model.as_ref() else {
        return "?".to_owned();
    };
    let part = |id: stave_model::ids::PartId| {
        model
            .parts
            .get(id.index())
            .map_or("?".to_owned(), |part| part.name.clone())
    };
    match slot {
        PlaceSlot::Region(Some(id)) => model
            .regions
            .get(id.index())
            .map_or("?".to_owned(), |region| region.name.clone()),
        PlaceSlot::Anchor(Some(face)) => format!("{}.{}", part(face.part), face.face.word()),
        PlaceSlot::Part(Some(id)) => part(*id),
        PlaceSlot::Parts(Some(parts)) => format!(
            "[{}]",
            parts
                .iter()
                .map(|id| part(*id))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        PlaceSlot::Region(None)
        | PlaceSlot::Anchor(None)
        | PlaceSlot::Part(None)
        | PlaceSlot::Parts(None) => "point at it".to_owned(),
    }
}

fn selection_text(selection: &Selection) -> String {
    match selection {
        Selection::Region(name) => name.clone(),
        Selection::Face(part, face) => format!("{part}.{}", face.word()),
        Selection::Between(first, second) => format!("{first} and {second}"),
        Selection::Parts(parts) => parts.join(", "),
    }
}

/// The typed selection a named one is in `opened`.
fn picked(opened: &Opened, selection: &Selection) -> Result<Picked> {
    let Some(model) = opened.model.as_ref() else {
        bail!("the project's names and types don't resolve; see stave check");
    };
    let part = |name: &str| -> Result<stave_model::ids::PartId> {
        match model.parts.iter().find(|part| part.name == name) {
            Some(part) => Ok(part.id),
            None => bail!("the design has no part called '{name}'"),
        }
    };
    Ok(match selection {
        Selection::Region(name) => match model.regions.iter().find(|r| r.name == *name) {
            Some(region) => Picked::Region(region.id),
            None => bail!("the design has no region called '{name}'"),
        },
        Selection::Face(name, face) => Picked::Face(PartFace {
            part: part(name)?,
            face: *face,
        }),
        Selection::Between(first, second) => match FacingPair::new(part(first)?, part(second)?) {
            Ok(pair) => Picked::Between(pair),
            Err(error) => bail!("{error}"),
        },
        Selection::Parts(names) => {
            let parts = names
                .iter()
                .map(|name| part(name))
                .collect::<Result<Vec<_>>>()?;
            match NonEmpty::new(parts) {
                Some(parts) => Picked::Group(parts),
                None => bail!("name at least one part"),
            }
        }
    })
}
