//! `stave edit` without a window: the edit loop as a script of steps
//! (`docs/editing.md`, "The command line"). The same steps come from the
//! command line's flags (`--region`, `--choose`, `--set`, `--commit`, ...)
//! or from a script file (`--script`), one step a line. Every step drives
//! the editor's session ([`stave_app::session::Session`]) with the same
//! commands a window sends, so a whole piece of furniture can be built by
//! a script:
//!
//! ```text
//! # a new project from its sizes, stock and tools
//! new "Hall cabinet" 600mm 800mm 400mm   # width, height, depth
//! import std:casework
//! stock oak_19 white_oak solid 19mm max_width 200mm max_length 2400mm
//! tools saw chisel
//! create
//! # a hat shelf above the divider
//! select region divider.above      # or: face door_l.stile[hinge].front,
//! list                             #     between side_l side_r, parts a b c
//! choose casework.Shelf
//! bind parts side_l side_r         # point at what the card asks for
//! set headroom = 400mm
//! name sweater_shelf
//! repeat 2 up                      # the chosen entry as two pieces
//! show
//! commit                           # or: add; cancel; fix <problem> <fix>
//! # changes of the design
//! repeat drawer 3                  # an assembly's count
//! shares drawer_bay 1 180mm        # the boundary above piece 1
//! mirror door                      # offer mirror door: only say if it has one
//! hold slat[0] apron_front min 12mm "room for a finger"
//! remove divider                   # then: include dependants
//! rename shelf hat_shelf
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

use anyhow::{Result, bail};
use stave_app::Opened;
use stave_app::candidates::Repeat;
use stave_app::candidates::new_project::{BoardForm, NewProject, NewStock, StockCut};
use stave_app::change::{Change, ChangeDetail, new_project_file};
use stave_app::edit::{
    Applicability, Candidate, FacingPair, MenuGroup, PartFace, Picked, PlaceSlot, SettingValue,
    Verdict,
};
use stave_app::editing::{
    Edit, EditCommand, EditEvent, Progress, entry_matches, parse_length, parse_name, parse_value,
    write_all,
};
use stave_app::hold::{HoldRequest, Measured};
use stave_app::naming::{Bound, RepeatCount};
use stave_app::remove::Dependants;
use stave_app::scratch::DiskWriter;
use stave_app::session::{Command, Event, Session};
use stave_lang::ast::Ident;
use stave_lang::placement::{Axis, Face};
use stave_lang::render_diagnostics;
use stave_model::nonempty::NonEmpty;
use stave_schema::species::Species;

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

/// One stock line of a new project: `stock oak_19 white_oak solid 19mm
/// max_length 2400mm`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StockLine(pub NewStock);

/// A `hold` line: `hold slat[0] apron_front min 12mm "room for a finger"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoldLine {
    /// The first thing, as written.
    pub first: String,
    /// The second thing, as written.
    pub second: String,
    /// A minimum or a maximum.
    pub bound: Bound,
    /// The value pinned.
    pub value: stave_lang::units::Length,
    /// Why.
    pub reason: String,
}

/// One step of the edit loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Begin a new project: its title and its overall width, height and
    /// depth. `import`, `stock` and `tools` lines add to it; `create`
    /// writes it.
    New {
        /// The title.
        title: String,
        /// Width, height, depth.
        size: [stave_lang::units::Length; 3],
    },
    /// A module the new project imports: `import std:casework`.
    Import(String),
    /// A stock item of the new project.
    Stock(StockLine),
    /// The tools of the new project.
    Tools(Vec<Ident>),
    /// Write the new project into the folder.
    Create,
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
    /// Make the chosen entry so many pieces split along an axis; `repeat
    /// 1` makes it one again.
    RepeatChosen(Option<Repeat>),
    /// Repeat an assembly of the project, or change its count.
    RepeatItem(Ident, RepeatCount, Option<Axis>),
    /// Move the boundary between piece `n` and piece `n + 1` (from 1) of
    /// a split so piece `n` is this long.
    Shares(Ident, usize, stave_lang::units::Length),
    /// Say whether an assembly has a mirror image, and what adding it
    /// would name.
    OfferMirror(Ident),
    /// Add the mirror image of an assembly.
    Mirror(Ident),
    /// Hold a measured distance: the two things as written (a part, a
    /// region, or a face of one), the bound, the value and the reason.
    Hold(HoldLine),
    /// Remove an item.
    Remove(Ident),
    /// Remove what depends on it too.
    IncludeDependants,
    /// Rename an item.
    Rename(Ident, Ident),
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

/// Reads a script: one step a line, `#` starting a comment (outside a
/// quoted string).
///
/// # Errors
///
/// A line that is not a step, with its line number.
pub fn parse_script(text: &str) -> Result<Vec<(String, Step)>> {
    let mut steps = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = without_comment(line).trim();
        if line.is_empty() {
            continue;
        }
        let step = parse_step(line)
            .map_err(|error| anyhow::anyhow!("line {}: {error:#}", index.saturating_add(1)))?;
        steps.push((line.to_owned(), step));
    }
    Ok(steps)
}

/// A line up to its `#` comment, a `#` inside a quoted string kept.
fn without_comment(line: &str) -> &str {
    let mut quoted = false;
    for (at, c) in line.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '#' if !quoted => return line.get(..at).unwrap_or(line),
            _ => {}
        }
    }
    line
}

const STEPS: &str = "new, import, stock, tools, create, select, list, choose, bind, set, name, repeat, shares, offer mirror, mirror, hold, remove, include dependants, rename, fix, show, commit, add or cancel";

fn parse_step(line: &str) -> Result<Step> {
    let words: Vec<&str> = line.split_whitespace().collect();
    match words.as_slice() {
        ["new", ..] => new_step(line),
        ["import", source] => Ok(Step::Import(source.trim_matches('"').to_owned())),
        ["stock", rest @ ..] => Ok(Step::Stock(stock_line(rest)?)),
        ["tools", tools @ ..] if !tools.is_empty() => Ok(Step::Tools(
            tools
                .iter()
                .map(|tool| parse_name(tool))
                .collect::<Result<Vec<_>, _>>()?,
        )),
        ["create"] => Ok(Step::Create),
        ["select", rest @ ..] => Ok(Step::Select(parse_selection(rest)?)),
        ["list"] => Ok(Step::List),
        ["choose", label] => Ok(Step::Choose((*label).to_owned())),
        ["bind", rest @ ..] => Ok(Step::Bind(parse_selection(rest)?)),
        ["set", ..] => {
            let rest = line.trim_start_matches("set").trim();
            setting(rest)
        }
        ["name", name] => Ok(Step::Name((*name).to_owned())),
        ["repeat", count] => Ok(Step::RepeatChosen(chosen_repeat(count, None)?)),
        ["repeat", first, second] if first.parse::<usize>().is_ok() => {
            Ok(Step::RepeatChosen(chosen_repeat(first, Some(second))?))
        }
        ["repeat", item, count, rest @ ..] if rest.len() <= 1 => Ok(Step::RepeatItem(
            parse_name(item)?,
            repeat_count(count)?,
            rest.first().map(|word| axis(word)).transpose()?,
        )),
        ["shares", region, piece, boundary] => Ok(Step::Shares(
            parse_name(region)?,
            count(piece)?,
            parse_length(boundary)?,
        )),
        ["offer", "mirror", item] => Ok(Step::OfferMirror(parse_name(item)?)),
        ["mirror", item] => Ok(Step::Mirror(parse_name(item)?)),
        ["hold", ..] => hold_step(line),
        ["remove", item] => Ok(Step::Remove(parse_name(item)?)),
        ["include", "dependants"] => Ok(Step::IncludeDependants),
        ["rename", item, to] => Ok(Step::Rename(parse_name(item)?, parse_name(to)?)),
        ["fix", problem, fix] => Ok(Step::Fix(count(problem)?, count(fix)?)),
        ["show"] => Ok(Step::Show),
        ["commit" | "add"] => Ok(Step::Commit),
        ["cancel"] => Ok(Step::Cancel),
        _ => bail!("'{line}' is not a step: {STEPS}"),
    }
}

/// `new "Hall cabinet" 600mm 800mm 400mm`.
fn new_step(line: &str) -> Result<Step> {
    let rest = line.trim_start_matches("new").trim();
    let usage = "a new project is written 'new \"<title>\" <width> <height> <depth>'";
    let Some(rest) = rest.strip_prefix('"') else {
        bail!("{usage}");
    };
    let Some((title, sizes)) = rest.split_once('"') else {
        bail!("{usage}");
    };
    let sizes: Vec<&str> = sizes.split_whitespace().collect();
    let [width, height, depth] = sizes.as_slice() else {
        bail!("{usage}");
    };
    Ok(Step::New {
        title: title.to_owned(),
        size: [
            parse_length(width)?,
            parse_length(height)?,
            parse_length(depth)?,
        ],
    })
}

/// `oak_19 white_oak solid 19mm [width 90mm] [max_width 200mm] [max_length
/// 2400mm]`, or `dowel_8 beech dowel 8mm`.
fn stock_line(words: &[&str]) -> Result<StockLine> {
    let usage = "a stock is written 'stock <name> <species> solid|plywood|mdf <thickness> [width <w>] [max_width <w>] [max_length <l>]' or 'stock <name> <species> dowel <diameter>'";
    let [name, species, form, size, options @ ..] = words else {
        bail!("{usage}");
    };
    let name = parse_name(name)?;
    let Some(species) = Species::from_word(species) else {
        bail!("'{species}' is not a species Stave knows");
    };
    let size = parse_length(size)?;
    let board = |form: BoardForm| -> Result<StockCut> {
        let (mut width, mut max_width, mut max_length) = (None, None, None);
        let mut rest = options.iter();
        while let Some(key) = rest.next() {
            let Some(value) = rest.next() else {
                bail!("{usage}");
            };
            let value = parse_length(value)?;
            match *key {
                "width" => width = Some(value),
                "max_width" => max_width = Some(value),
                "max_length" => max_length = Some(value),
                _ => bail!("{usage}"),
            }
        }
        let Some(max_length) = max_length else {
            bail!("say the longest board you can get: 'max_length 2400mm'");
        };
        Ok(StockCut::Board {
            form,
            thickness: size,
            width,
            max_width,
            max_length,
        })
    };
    let cut = match *form {
        "solid" => board(BoardForm::Solid)?,
        "plywood" => board(BoardForm::Plywood)?,
        "mdf" => board(BoardForm::Mdf)?,
        "dowel" if options.is_empty() => StockCut::Dowel { diameter: size },
        _ => bail!("{usage}"),
    };
    Ok(StockLine(NewStock { name, species, cut }))
}

/// `hold <a> <b> min|max <length> "<reason>"`.
fn hold_step(line: &str) -> Result<Step> {
    let usage = "a held distance is written 'hold <a> <b> min|max <length> \"<reason>\"', each of a and b a part, a region or one of their faces";
    let rest = line.trim_start_matches("hold").trim();
    let Some((head, reason)) = rest.split_once('"') else {
        bail!("{usage}");
    };
    let Some(reason) = reason.trim_end().strip_suffix('"') else {
        bail!("{usage}");
    };
    let words: Vec<&str> = head.split_whitespace().collect();
    let [first, second, bound, value] = words.as_slice() else {
        bail!("{usage}");
    };
    let bound = match *bound {
        "min" => Bound::Min,
        "max" => Bound::Max,
        _ => bail!("{usage}"),
    };
    Ok(Step::Hold(HoldLine {
        first: (*first).to_owned(),
        second: (*second).to_owned(),
        bound,
        value: parse_length(value)?,
        reason: reason.to_owned(),
    }))
}

/// A thing a distance is measured to, as written: a part or a region of
/// the design by its whole name (`drawer[top].front` is a part), else a
/// face of one (`apron_front.back`).
fn measured(opened: &Opened, text: &str) -> Measured {
    let named = opened.design.as_ref().is_some_and(|design| {
        design.part_named(text).is_some() || design.regions.iter().any(|r| r.name == text)
    });
    match text.rsplit_once('.') {
        Some((item, word)) if !named => match Face::from_word(word) {
            Some(face) => Measured::Face {
                item: item.to_owned(),
                face,
            },
            None => Measured::Item(text.to_owned()),
        },
        _ => Measured::Item(text.to_owned()),
    }
}

fn axis(word: &str) -> Result<Axis> {
    Axis::from_word(word)
        .ok_or_else(|| anyhow::anyhow!("'{word}' is not an axis: across, up or deep"))
}

fn repeat_count(word: &str) -> Result<RepeatCount> {
    word.parse::<usize>()
        .ok()
        .and_then(RepeatCount::new)
        .ok_or_else(|| anyhow::anyhow!("'{word}' is not a count of two or more"))
}

/// `repeat 3 up` (three pieces split up), `repeat 1` (one again).
fn chosen_repeat(count_word: &str, axis_word: Option<&&str>) -> Result<Option<Repeat>> {
    if count_word == "1" && axis_word.is_none() {
        return Ok(None);
    }
    let Some(axis_word) = axis_word else {
        bail!("say which way to split the space: 'repeat {count_word} up', across or deep");
    };
    Ok(Some(Repeat {
        count: repeat_count(count_word)?,
        axis: axis(axis_word)?,
    }))
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
        session: Session::new(folder.to_path_buf()),
        new: None,
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
    /// The editor's session: every step is a command to it.
    session: Session,
    /// A new project being described, until `create`.
    new: Option<NewProject>,
    /// A script's transcript: each step says where the candidate stands.
    /// Without it (the flags) only failures and the card are printed.
    echo: bool,
}

impl Runner<'_> {
    /// Sends an edit command and gives back what the session said.
    fn send(&mut self, command: EditCommand) -> Vec<EditEvent> {
        self.session
            .apply(Command::Edit(command))
            .into_iter()
            .filter_map(|event| match event {
                Event::Edit(event) => Some(event),
                _ => None,
            })
            .collect()
    }

    fn new_project(&mut self) -> Result<&mut NewProject> {
        match self.new.as_mut() {
            Some(project) => Ok(project),
            None => bail!("start a new project first: 'new \"<title>\" <width> <height> <depth>'"),
        }
    }

    fn step(&mut self, step: &Step, out: &mut String) -> Result<bool> {
        let verbose = self.echo;
        match step {
            Step::New { title, size } => {
                self.new = Some(NewProject {
                    title: title.clone(),
                    size: *size,
                    stock: Vec::new(),
                    tools: Vec::new(),
                    imports: Vec::new(),
                });
                let [width, height, depth] = size;
                writeln!(
                    out,
                    "new project \"{title}\": {width} wide, {height} high, {depth} deep"
                )?;
                Ok(true)
            }
            Step::Import(source) => {
                self.new_project()?.imports.push(source.clone());
                Ok(true)
            }
            Step::Stock(StockLine(stock)) => {
                self.new_project()?.stock.push(stock.clone());
                Ok(true)
            }
            Step::Tools(tools) => {
                self.new_project()?.tools.extend(tools.iter().cloned());
                Ok(true)
            }
            Step::Create => {
                let project = self.new_project()?.clone();
                std::fs::create_dir_all(self.folder)?;
                let file = new_project_file(self.folder, &project)?;
                write_all(std::slice::from_ref(&file), &mut DiskWriter)?;
                self.new = None;
                writeln!(out, "created {}", file.path.display())?;
                let opened = stave_app::open(self.folder)?;
                let errors = opened.error_count();
                writeln!(
                    out,
                    "the project checks with {errors} {}",
                    if errors == 1 { "error" } else { "errors" }
                )?;
                Ok(errors == 0)
            }
            Step::Select(selection) => {
                // The selection's names are looked up in the design as it
                // is on disk now.
                let opened = stave_app::open(self.folder)?;
                let picked = picked(&opened, selection)?;
                let events = self.send(EditCommand::Start(picked));
                if let Some(failed) = rejected(&events) {
                    bail!("{failed}");
                }
                let edit = self.edit()?;
                writeln!(
                    out,
                    "{}: {}",
                    edit.picked().heading().text(),
                    selection_text(selection)
                )?;
                writeln!(out, "{} menu entries", edit.menu().len())?;
                Ok(true)
            }
            Step::List => {
                self.edit()?;
                let events = self.send(EditCommand::CheckMenu);
                if let Some(failed) = rejected(&events) {
                    bail!("{failed}");
                }
                list(self.edit()?, out)?;
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
                let events = self.send(EditCommand::Choose(index));
                self.outcome(&events, verbose, out)
            }
            Step::Bind(selection) => {
                let picked = picked(self.edit()?.base(), selection)?;
                let events = self.send(EditCommand::Bind(picked));
                self.outcome(&events, verbose, out)
            }
            Step::Set(name, value) => {
                self.edit()?;
                let command = EditCommand::Set {
                    name: parse_name(name)?,
                    value: parse_value(value)?,
                };
                let events = self.send(command);
                self.outcome(&events, verbose, out)
            }
            Step::Name(name) => {
                self.edit()?;
                let events = self.send(EditCommand::Rename(parse_name(name)?));
                self.outcome(&events, verbose, out)
            }
            Step::RepeatChosen(repeat) => {
                self.edit()?;
                let events = self.send(EditCommand::Repeat(*repeat));
                self.outcome(&events, verbose, out)
            }
            Step::RepeatItem(item, count, axis) => self.change(
                EditCommand::RepeatItem {
                    item: item.clone(),
                    count: *count,
                    axis: *axis,
                },
                out,
            ),
            Step::Shares(region, piece, boundary) => self.change(
                EditCommand::MoveShares {
                    region: region.clone(),
                    lower: piece.saturating_sub(1),
                    boundary: *boundary,
                },
                out,
            ),
            Step::OfferMirror(item) => {
                match self.session.mirror_offer(item.as_str()) {
                    Some(offer) => writeln!(
                        out,
                        "{} has a mirror image: it becomes {} and {} is added, mirrored about {}",
                        offer.source,
                        offer.renamed.as_str(),
                        offer.image.as_str(),
                        offer.about
                    )?,
                    None => writeln!(out, "{} has no mirror image to add", item.as_str())?,
                }
                Ok(true)
            }
            Step::Mirror(item) => self.change(EditCommand::Mirror(item.clone()), out),
            Step::Hold(line) => {
                let opened = stave_app::open(self.folder)?;
                let request = HoldRequest {
                    first: measured(&opened, &line.first),
                    second: measured(&opened, &line.second),
                    bound: line.bound,
                    value: line.value,
                    reason: line.reason.clone(),
                };
                self.change(EditCommand::Hold(request), out)
            }
            Step::Remove(item) => self.change(EditCommand::Remove(item.clone()), out),
            Step::IncludeDependants => {
                let events = self.send(EditCommand::IncludeDependants);
                self.outcome(&events, verbose, out)
            }
            Step::Rename(item, to) => self.change(
                EditCommand::RenameItem {
                    item: item.clone(),
                    to: to.clone(),
                },
                out,
            ),
            Step::Fix(problem, fix) => {
                if self.session.change().is_none() {
                    self.edit()?;
                }
                let events = self.send(EditCommand::ApplyFix {
                    problem: problem.saturating_sub(1),
                    fix: fix.saturating_sub(1),
                });
                self.outcome(&events, verbose, out)
            }
            Step::Show => {
                if let Some(change) = self.session.change() {
                    change_card(change, out)?;
                } else {
                    card(self.edit()?, out)?;
                }
                Ok(true)
            }
            Step::Commit => {
                if self.session.change().is_none() {
                    self.edit()?;
                }
                let events = self.send(EditCommand::Add);
                if events.contains(&EditEvent::Committed) {
                    for file in self.session.last_commit() {
                        let verb = if file.created { "created" } else { "wrote" };
                        writeln!(out, "{verb} {}", file.path.display())?;
                    }
                    let errors = self
                        .session
                        .latest()
                        .map_or(0, |checked| checked.opened.error_count());
                    writeln!(
                        out,
                        "committed: the design checks with {errors} {}",
                        if errors == 1 { "error" } else { "errors" }
                    )?;
                    return Ok(true);
                }
                for event in &events {
                    if let EditEvent::Rejected(rejection) = event {
                        match rejection {
                            stave_app::editing::Rejection::CantWrite(message) => {
                                bail!("{message}")
                            }
                            other => writeln!(out, "not committed: {}", other.text())?,
                        }
                    }
                }
                Ok(false)
            }
            Step::Cancel => {
                let events = self.send(EditCommand::Cancel);
                if rejected(&events).is_some() {
                    bail!("nothing is being edited");
                }
                writeln!(out, "cancelled: nothing was written")?;
                Ok(true)
            }
        }
    }

    /// Starts a change and says where it stands.
    fn change(&mut self, command: EditCommand, out: &mut String) -> Result<bool> {
        let events = self.send(command);
        if let Some(failed) = rejected(&events) {
            writeln!(out, "rejected: {failed}")?;
            return Ok(false);
        }
        if let Some(change) = self.session.change() {
            writeln!(out, "{}: {}", change.summary(), change_line(change))?;
        }
        Ok(true)
    }

    fn edit(&self) -> Result<&Edit> {
        match self.session.edit() {
            Some(edit) => Ok(edit),
            None => {
                bail!("nothing is being edited: start with a selection ('select region <name>')")
            }
        }
    }

    /// Says what a command did: a rejection fails the step; otherwise,
    /// when `verbose`, the chosen candidate's (or the change's) state in
    /// one line.
    fn outcome(&self, events: &[EditEvent], verbose: bool, out: &mut String) -> Result<bool> {
        if let Some(failed) = rejected(events) {
            writeln!(out, "rejected: {failed}")?;
            return Ok(false);
        }
        if !verbose {
            return Ok(true);
        }
        if let Some(change) = self.session.change() {
            writeln!(out, "{}", change_line(change))?;
        } else if let Some(chosen) = self.session.edit().and_then(Edit::chosen) {
            writeln!(out, "{}", progress_line(chosen.progress()))?;
        }
        Ok(true)
    }
}

/// The words of the first rejection among `events`.
fn rejected(events: &[EditEvent]) -> Option<String> {
    events.iter().find_map(|event| match event {
        EditEvent::Rejected(rejection) => Some(rejection.text()),
        _ => None,
    })
}

/// Where a change stands, in one line; a removal waiting on what uses the
/// item names it.
fn change_line(change: &Change) -> String {
    match (change.detail(), change.progress()) {
        (ChangeDetail::Remove { .. }, Progress::NeedsChoice(names)) => format!(
            "used by {}; 'include dependants' removes them too",
            names_text(names)
        ),
        (_, progress) => progress_line(progress),
    }
}

/// The card of a change: what it does, what it found, its verdict, the
/// checker's words and fixes, and its patch.
fn change_card(change: &Change, out: &mut String) -> Result<()> {
    writeln!(out, "{}", change.summary())?;
    match change.detail() {
        ChangeDetail::Repeat => {}
        ChangeDetail::Mirror {
            renamed,
            image,
            about,
        } => writeln!(
            out,
            "  renames the original to {}, adds {} mirrored about {about}",
            renamed.as_str(),
            image.as_str()
        )?,
        ChangeDetail::Hold { name, condition } => {
            writeln!(out, "  require {} : {condition}", name.as_str())?;
        }
        ChangeDetail::Remove {
            removal,
            dependants,
        } => {
            if removal.dependants.is_empty() {
                writeln!(out, "  nothing uses {}", removal.item.name)?;
            } else {
                let names: Vec<&str> = removal
                    .dependants
                    .iter()
                    .map(|item| item.name.as_str())
                    .collect();
                let taken = match dependants {
                    Dependants::Included => "removed too",
                    Dependants::Refuse => "not removed yet",
                };
                writeln!(out, "  used by {} ({taken})", names.join(", "))?;
            }
        }
        ChangeDetail::Rename { from, to } => {
            writeln!(out, "  renames {from} to {} where it is used", to.as_str())?;
        }
    }
    for path in change.renamed_files() {
        writeln!(out, "  renames in {}", path.display())?;
    }
    writeln!(out, "verdict: {}", change_line(change))?;
    if let Progress::Tried(Verdict::Refused(refusal)) = change.progress() {
        for problem in refusal.problems().iter() {
            if let Some(rendered) = problem.rendered() {
                out.push_str(rendered);
                out.push('\n');
            }
        }
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
    } else if let Some(tried) = change.tried() {
        out.push_str(&render_diagnostics(tried.sources(), &tried.shown()));
    }
    if let Some(patch) = change.patch() {
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
