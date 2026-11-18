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
//! set drawer.top_gap = 20mm        # a param of an assembly, as a grip
//! mirror door                      # offer mirror door: only say if it has one
//! mirror door across               # a door filling a space: a mirrored pair
//! hold slat[0] apron_front min 12mm "room for a finger"
//! hold slat[0] apron_front min "room for a finger"   # as it is now
//! measure slat[0] apron_front      # the distance now
//! spaced slat 5 across from apron_left.right to apron_right.left
//! remove divider                   # then: include dependants
//! rename shelf hat_shelf
//! # grips and the solver
//! grips assembly divider           # or: grips envelope, grips split drawer_bay[bottom]
//! stops 1                          # how far grip 1 can go each way
//! drag 1 420mm                     # tried at once; drag again to go on
//! grip 1 = 450mm                   # a value typed for grip 1
//! pick overall_width               # which param a dragged size changes
//! solved overall_width within 600mm..1200mm
//! plain overall_width
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
use stave_app::gripping::{GripChoice, GripDetail, GripList, GripStops};
use stave_app::grips::{FollowKind, GripAction, GripSelection};
use stave_app::hold::{HoldRequest, Measured};
use stave_app::naming::{Bound, RepeatCount};
use stave_app::remove::Dependants;
use stave_app::scratch::DiskWriter;
use stave_app::session::{Command, Event, Session};
use stave_app::stops::{Side, SideEnd, amount_text};
use stave_lang::ast::Ident;
use stave_lang::placement::{Axis, Face};
use stave_lang::render_diagnostics;
use stave_model::nonempty::NonEmpty;
use stave_model::trace::{Amount, Freedom};
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

/// What a `grips` step lists the grips of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GripsOf {
    /// `grips envelope`: the project's three overall sizes.
    Envelope,
    /// `grips assembly divider`: an assembly's params and parts' sizes.
    Assembly(String),
    /// `grips split drawer_bay[bottom]`: the boundaries of a split.
    Split(String),
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
    /// The value pinned; `None` for the distance as it is now.
    pub value: Option<stave_lang::units::Length>,
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
    /// Change: set a param of an assembly of the project, as a grip does
    /// (`set apron_front.setback = 8mm`): the assembly, the param and the
    /// value as typed.
    SetParam(Ident, Ident, String),
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
    /// Make an assembly that fills a whole space a mirrored pair: split
    /// the space in two along the axis, the assembly in the first piece
    /// and its mirror image in the other.
    MirrorSplit(Ident, Axis),
    /// Hold a measured distance: the two things as written (a part, a
    /// region, or a face of one), the bound, the value (the distance as
    /// it is now when left out) and the reason.
    Hold(HoldLine),
    /// Say the distance between two things as the design has it now.
    Measure(String, String),
    /// Repeat a part spaced evenly along a span: the part, how many, the
    /// axis, and the anchors the span runs from and to.
    Spaced {
        /// The part.
        part: Ident,
        /// How many.
        count: RepeatCount,
        /// The axis they are spaced along.
        axis: Axis,
        /// The anchor the span starts at.
        from: String,
        /// The anchor it ends at.
        to: String,
    },
    /// List the grips of something.
    Grips(GripsOf),
    /// Search the stops of the listed grips: of one (counted from 1), or
    /// of all.
    Stops(Option<usize>),
    /// Drag a listed grip (counted from 1) to a value as typed; a drag on
    /// the same grip goes on from where it is.
    Drag(usize, String),
    /// Type a value for a listed grip (counted from 1).
    TypeGrip(usize, String),
    /// Pick the param a dragged size changes, by its name.
    Pick(String),
    /// Make a param of the project solved, with its range as typed.
    Solved(Ident, String, String),
    /// Make a solved param of the project plain again.
    Plain(Ident),
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

const STEPS: &str = "new, import, stock, tools, create, select, list, choose, bind, set, name, repeat, spaced, shares, offer mirror, mirror, measure, hold, remove, include dependants, rename, grips, stops, drag, grip, pick, solved, plain, fix, show, commit, add or cancel";

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
        ["mirror", item, way] => Ok(Step::MirrorSplit(parse_name(item)?, axis(way)?)),
        ["hold", ..] => hold_step(line),
        ["measure", first, second] => Ok(Step::Measure((*first).to_owned(), (*second).to_owned())),
        ["spaced", part, count, way, "from", from, "to", to] => Ok(Step::Spaced {
            part: parse_name(part)?,
            count: repeat_count(count)?,
            axis: axis(way)?,
            from: (*from).to_owned(),
            to: (*to).to_owned(),
        }),
        ["grips", "envelope"] => Ok(Step::Grips(GripsOf::Envelope)),
        ["grips", "assembly", name] => Ok(Step::Grips(GripsOf::Assembly((*name).to_owned()))),
        ["grips", "split", piece] => Ok(Step::Grips(GripsOf::Split((*piece).to_owned()))),
        ["stops"] => Ok(Step::Stops(None)),
        ["stops", grip] => Ok(Step::Stops(Some(count(grip)?))),
        ["drag", grip, value] => Ok(Step::Drag(count(grip)?, (*value).to_owned())),
        ["grip", grip, "=", value @ ..] if !value.is_empty() => {
            Ok(Step::TypeGrip(count(grip)?, value.join(" ")))
        }
        ["pick", name] => Ok(Step::Pick((*name).to_owned())),
        ["solved", param, "within", range] => match range.split_once("..") {
            Some((low, high)) if !low.is_empty() && !high.is_empty() => Ok(Step::Solved(
                parse_name(param)?,
                low.to_owned(),
                high.to_owned(),
            )),
            _ => bail!("a solved param needs a range: 'solved {param} within 600mm..1200mm'"),
        },
        ["plain", param] => Ok(Step::Plain(parse_name(param)?)),
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
    let usage = "a held distance is written 'hold <a> <b> min|max [<length>] \"<reason>\"', each of a and b a part, a region or one of their faces; without a length it holds the distance as it is now";
    let rest = line.trim_start_matches("hold").trim();
    let Some((head, reason)) = rest.split_once('"') else {
        bail!("{usage}");
    };
    let Some(reason) = reason.trim_end().strip_suffix('"') else {
        bail!("{usage}");
    };
    let words: Vec<&str> = head.split_whitespace().collect();
    let (first, second, bound, value) = match words.as_slice() {
        [first, second, bound] => (first, second, bound, None),
        [first, second, bound, value] => (first, second, bound, Some(value)),
        _ => bail!("{usage}"),
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
        value: value.map(|value| parse_length(value)).transpose()?,
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
    let (name, value) = (name.trim(), value.trim());
    if let Some((item, param)) = name.split_once('.') {
        return Ok(Step::SetParam(
            parse_name(item)?,
            parse_name(param)?,
            value.to_owned(),
        ));
    }
    Ok(Step::Set(name.to_owned(), value.to_owned()))
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
            Step::SetParam(item, param, value) => self.change(
                EditCommand::SetParam {
                    item: item.clone(),
                    param: param.clone(),
                    value: parse_value(value)?,
                },
                out,
            ),
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
            Step::MirrorSplit(item, axis) => self.change(
                EditCommand::MirrorSplit {
                    item: item.clone(),
                    axis: *axis,
                },
                out,
            ),
            Step::Hold(line) => {
                let opened = stave_app::open(self.folder)?;
                let (first, second) = (
                    measured(&opened, &line.first),
                    measured(&opened, &line.second),
                );
                let value = match line.value {
                    Some(value) => value,
                    None => {
                        let now = self
                            .session
                            .measure(&first, &second)
                            .map_err(anyhow::Error::msg)?;
                        writeln!(out, "the distance is {now} now")?;
                        now
                    }
                };
                let request = HoldRequest {
                    first,
                    second,
                    bound: line.bound,
                    value,
                    reason: line.reason.clone(),
                };
                self.change(EditCommand::Hold(request), out)
            }
            Step::Measure(first, second) => {
                let opened = stave_app::open(self.folder)?;
                let distance = self
                    .session
                    .measure(&measured(&opened, first), &measured(&opened, second))
                    .map_err(anyhow::Error::msg)?;
                writeln!(out, "{first} to {second}: {distance}")?;
                Ok(true)
            }
            Step::Spaced {
                part,
                count,
                axis,
                from,
                to,
            } => self.change(
                EditCommand::RepeatPart {
                    part: part.clone(),
                    count: *count,
                    axis: *axis,
                    from: from.clone(),
                    to: to.clone(),
                },
                out,
            ),
            Step::Grips(of) => {
                let opened = stave_app::open(self.folder)?;
                let selection = grip_selection(&opened, of)?;
                let events = self.send(EditCommand::ListGrips(selection));
                if let Some(failed) = rejected(&events) {
                    writeln!(out, "rejected: {failed}")?;
                    return Ok(false);
                }
                if let Some(grips) = self.session.grips() {
                    grips_list(grips, out)?;
                }
                Ok(true)
            }
            Step::Stops(grip) => {
                let never = std::sync::atomic::AtomicBool::new(false);
                let events = self
                    .session
                    .find_grip_stops(grip.map(|n| n.saturating_sub(1)), &never);
                let events: Vec<EditEvent> = events
                    .into_iter()
                    .filter_map(|event| match event {
                        Event::Edit(event) => Some(event),
                        _ => None,
                    })
                    .collect();
                if let Some(failed) = rejected(&events) {
                    writeln!(out, "rejected: {failed}")?;
                    return Ok(false);
                }
                if let Some(grips) = self.session.grips() {
                    stops_list(grips, *grip, out)?;
                }
                Ok(true)
            }
            Step::Drag(grip, value) => {
                let index = grip.saturating_sub(1);
                let value = self.grip_value(index, value)?;
                let dragging = self.session.change().is_some_and(|change| {
                    matches!(change.detail(), ChangeDetail::Grip(detail) if detail.index == index)
                });
                if !dragging {
                    let events = self.send(EditCommand::BeginDrag(index));
                    if let Some(failed) = rejected(&events) {
                        writeln!(out, "rejected: {failed}")?;
                        return Ok(false);
                    }
                }
                self.drag_step(EditCommand::DragTo(value), out)
            }
            Step::TypeGrip(grip, value) => {
                let index = grip.saturating_sub(1);
                let value = self.grip_value(index, value)?;
                self.drag_step(EditCommand::TypeGripValue { grip: index, value }, out)
            }
            Step::Pick(name) => {
                let choice = self
                    .session
                    .change()
                    .and_then(|change| match change.detail() {
                        ChangeDetail::Grip(detail) => detail
                            .choices
                            .iter()
                            .find(|choice| choice_named(choice, name))
                            .map(|choice| choice.param),
                        _ => None,
                    });
                let Some(param) = choice else {
                    bail!("'{name}' is not one of the sizes the dragged grip could change");
                };
                self.drag_step(EditCommand::PickParam(param), out)
            }
            Step::Solved(param, low, high) => self.change(
                EditCommand::MakeSolved {
                    param: param.clone(),
                    low: parse_value(low)?,
                    high: parse_value(high)?,
                },
                out,
            ),
            Step::Plain(param) => self.change(EditCommand::MakePlain(param.clone()), out),
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

    /// Sends a drag command and says where the drag stands, with what it
    /// changes, what to pick from and what stops it.
    fn drag_step(&mut self, command: EditCommand, out: &mut String) -> Result<bool> {
        let events = self.send(command);
        if let Some(failed) = rejected(&events) {
            writeln!(out, "rejected: {failed}")?;
            return Ok(false);
        }
        if let Some(change) = self.session.change() {
            writeln!(out, "{}: {}", change.summary(), change_line(change))?;
            if let ChangeDetail::Grip(detail) = change.detail() {
                grip_detail(detail, out)?;
            }
        }
        Ok(true)
    }

    /// A value typed for the listed grip at `index`, of the grip's kind.
    fn grip_value(&self, index: usize, text: &str) -> Result<Amount> {
        let Some(grips) = self.session.grips() else {
            bail!("list the grips of something first: 'grips assembly <name>'");
        };
        let Some(listed) = grips.grips().get(index) else {
            bail!("there is no grip {}", index.saturating_add(1));
        };
        let now = match &listed.grip.action {
            GripAction::Moves(movable) => movable.value,
            GripAction::Follows(follows) => follows.value,
        };
        amount_like(now, text)
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
        (ChangeDetail::Grip(_), Progress::NeedsChoice(names)) => {
            format!("pick which size changes: {}", names_text(names))
        }
        (ChangeDetail::Grip(_), Progress::Unwritable(reason)) => format!("refused: {reason}"),
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
        ChangeDetail::Grip(detail) => {
            writeln!(out, "  grip {}: {}", detail.index + 1, detail.label)?;
            if let Some(target) = detail.target {
                writeln!(out, "  moved to {}", amount(target))?;
            }
            grip_detail(detail, out)?;
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

/// What a drag found, indented: what changes (when the solver chose it,
/// or after a pick), what to pick from, what stops it.
fn grip_detail(detail: &GripDetail, out: &mut String) -> Result<()> {
    if (detail.solved || detail.picked.is_some()) && !detail.changes.is_empty() {
        let changes: Vec<String> = detail
            .changes
            .iter()
            .map(|change| {
                format!(
                    "{} {} -> {}",
                    change.path,
                    amount(change.from),
                    amount(change.to)
                )
            })
            .collect();
        writeln!(out, "  changes {}", changes.join(", "))?;
    }
    if let Some(reached) = detail.reached {
        writeln!(out, "  reaches {} on the resolution", amount(reached))?;
    }
    if detail.picked.is_none() {
        for choice in &detail.choices {
            match &choice.change {
                Ok(change) => writeln!(
                    out,
                    "  could change {}: {} -> {}",
                    choice.name,
                    amount(change.from),
                    amount(change.to)
                )?,
                Err(why) => writeln!(out, "  could not change {}: {why}", choice.name)?,
            }
        }
    }
    Ok(())
}

/// An amount as `.stave` writes it.
fn amount(value: Amount) -> String {
    amount_text(value).unwrap_or_else(|| "?".to_owned())
}

/// The value a grip of the kind of `now` is moved to, as typed: a length
/// (`400mm`), an angle (`95deg`) or a plain number (`0.37`, `3`).
fn amount_like(now: Amount, text: &str) -> Result<Amount> {
    let expr = parse_value(text)?;
    let wrong = || anyhow::anyhow!("'{text}' is not the same kind of value as {}", amount(now));
    Ok(match (now, &expr.kind) {
        (Amount::Length(_), stave_lang::ast::ExprKind::Quantity(quantity)) => {
            Amount::Length(stave_lang::units::quantity_to_length(*quantity).map_err(|_| wrong())?)
        }
        (Amount::Angle(_), stave_lang::ast::ExprKind::Quantity(quantity)) => {
            Amount::Angle(stave_lang::units::quantity_to_angle(*quantity).map_err(|_| wrong())?)
        }
        (Amount::Count(_), stave_lang::ast::ExprKind::Number(number)) => {
            Amount::Count(stave_lang::units::number_to_rational(*number).map_err(|_| wrong())?)
        }
        (Amount::Ratio(_), stave_lang::ast::ExprKind::Number(number)) => {
            Amount::Ratio(stave_lang::units::number_to_rational(*number).map_err(|_| wrong())?)
        }
        _ => return Err(wrong()),
    })
}

/// Whether a choice is the one named: by its name as listed
/// (`case.overhang`), or the param's own name (`overhang`).
fn choice_named(choice: &GripChoice, name: &str) -> bool {
    choice.name == name
        || choice
            .name
            .rsplit('.')
            .next()
            .and_then(|tail| tail.split('[').next())
            == Some(name)
}

/// The grips of a selection by name.
fn grip_selection(opened: &Opened, of: &GripsOf) -> Result<GripSelection> {
    let Some(model) = opened.model.as_ref() else {
        bail!("the project's names and types don't resolve; see stave check");
    };
    Ok(match of {
        GripsOf::Envelope => GripSelection::Envelope,
        GripsOf::Assembly(name) => match model.assemblies.iter().find(|a| a.name == *name) {
            Some(assembly) => GripSelection::Assembly(assembly.id),
            None => bail!("the design has no assembly called '{name}'"),
        },
        GripsOf::Split(name) => match model.regions.iter().find(|r| r.name == *name) {
            Some(region) => GripSelection::Split(region.id),
            None => bail!("the design has no region called '{name}'"),
        },
    })
}

/// The listed grips, one a line, numbered from 1: the size, its value,
/// and what it moves through or what it follows.
fn grips_list(grips: &GripList, out: &mut String) -> Result<()> {
    if grips.grips().is_empty() {
        writeln!(out, "no grips")?;
    }
    let width = grips
        .grips()
        .iter()
        .map(|listed| listed.grip.label.len())
        .max()
        .unwrap_or(0);
    for (index, listed) in grips.grips().iter().enumerate() {
        let grip = &listed.grip;
        let pad = " ".repeat(width - grip.label.len());
        let axis = grip
            .axis
            .map_or(String::new(), |axis| format!(" ({})", axis.word()));
        let what = match &grip.action {
            GripAction::Moves(movable) => {
                let range = format!("{}..{}", amount(movable.low), amount(movable.high));
                let kind = match movable.freedom {
                    Freedom::Solved => ", solved",
                    Freedom::Plain | Freedom::Locked | Freedom::StockField | Freedom::KindSize => {
                        ""
                    }
                };
                format!("{}  moves {range}{kind}", amount(movable.value))
            }
            GripAction::Follows(follows) => {
                let names: Vec<String> = follows
                    .follows
                    .iter()
                    .map(|f| match f.kind {
                        FollowKind::Plain => f.name.clone(),
                        kind => format!("{} ({})", f.name, kind.words()),
                    })
                    .collect();
                if names.is_empty() {
                    format!("{}  is fixed", amount(follows.value))
                } else {
                    format!("{}  follows {}", amount(follows.value), names.join(", "))
                }
            }
        };
        writeln!(out, "{:>3}. {}{pad}  {what}{axis}", index + 1, grip.label)?;
    }
    Ok(())
}

/// The stops found: for each searched grip (or the one asked for), its
/// range and what ends it each way.
fn stops_list(grips: &GripList, only: Option<usize>, out: &mut String) -> Result<()> {
    for (index, listed) in grips.grips().iter().enumerate() {
        if only.is_some_and(|n| n.saturating_sub(1) != index) {
            continue;
        }
        let side = |side: &Side| match &side.end {
            SideEnd::Stopped { at, diagnostic } => format!(
                "{} ({} breaks it: {}[{}]: {})",
                amount(side.last_good),
                amount(*at),
                diagnostic.severity,
                diagnostic.code,
                diagnostic.message
            ),
            SideEnd::Domain => format!("{} (the end of its range)", amount(side.last_good)),
            SideEnd::Limit => format!("{} (as far as the search looks)", amount(side.last_good)),
        };
        match &listed.stops {
            GripStops::Found { below, above } => {
                writeln!(out, "{:>3}. {}", index + 1, listed.grip.label)?;
                writeln!(out, "       down to {}", side(below))?;
                writeln!(out, "       up to {}", side(above))?;
            }
            GripStops::Failed(why) => {
                writeln!(out, "{:>3}. {}: {why}", index + 1, listed.grip.label)?;
            }
            GripStops::NotSearched | GripStops::NotSearchable => {
                if only.is_some() {
                    writeln!(
                        out,
                        "{:>3}. {}: no stops to search",
                        index + 1,
                        listed.grip.label
                    )?;
                }
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
                SettingValue::Default => default_text(setting),
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

/// A setting left at its default, as the card shows it: the value it
/// works out to, with the default as written when that says more (`220mm
/// (default: opening.up - bottom_gap - top_gap)`, `dowel(count: 4)
/// (default)`).
fn default_text(setting: &stave_app::edit::SettingParam) -> String {
    match (&setting.default_value, &setting.default_text) {
        (Some(value), Some(written)) if value != written => {
            format!("{value} (default: {written})")
        }
        (Some(value), _) | (None, Some(value)) => format!("{value} (default)"),
        (None, None) => "the default".to_owned(),
    }
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
