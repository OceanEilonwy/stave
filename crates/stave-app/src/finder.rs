//! The finder (Ctrl+K): one search over the design's parts and
//! assemblies, the project's files and names, its lines of text and its
//! problems, worked out without any UI.
//!
//! Each [`Hit`] says where choosing it goes ([`Goal`]): selecting a part,
//! an assembly or a problem (the 3D view frames it), or opening a file at
//! a line in the editor.

use crate::i18n::tr;
use stave_ide::{Analysis, Location};
use stave_lang::Severity;
use stave_model::solved::SolvedDesign;

use crate::check::Checked;
use crate::session::{LineNumber, ProblemIndex, Selection, SourceLocation};

/// The most hits of each kind the finder lists.
pub const LIMIT_PER_KIND: usize = 8;

/// What a hit is, which is also how the finder groups and filters them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    /// A part, a piece of hardware or an assembly of the design.
    Part,
    /// A file of the project.
    File,
    /// A name declared in the text: a component, a stock, a kind.
    Name,
    /// A line of the project's own text.
    Line,
    /// One of the current problems.
    Problem,
}

impl Kind {
    /// Every kind, in the order the finder lists them.
    pub const ALL: [Kind; 5] = [
        Kind::Part,
        Kind::File,
        Kind::Name,
        Kind::Line,
        Kind::Problem,
    ];

    /// Its group's heading and filter: "Parts", "Files", "Names", "Lines",
    /// "Problems".
    pub fn title(self) -> String {
        match self {
            Kind::Part => tr("Parts"),
            Kind::File => tr("Files"),
            Kind::Name => tr("Names"),
            Kind::Line => tr("Lines"),
            Kind::Problem => tr("Problems"),
        }
    }
}

/// Where choosing a hit goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Goal {
    /// Select it: the 3D view frames a part or an assembly and poses a
    /// problem.
    Select(Selection),
    /// Open this place in the editor.
    Open(SourceLocation),
}

/// One thing the finder found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    /// What it is.
    pub kind: Kind,
    /// Its name, file or line of text.
    pub label: String,
    /// Dim text after it: the stock, "assembly", `doors.stave:11`.
    pub detail: String,
    /// Where choosing it goes.
    pub goal: Goal,
    /// A problem's severity, for its dot.
    pub severity: Option<Severity>,
}

/// What the finder searches: the result the 3D view shows (its parts),
/// the latest result (its problems and text) and the editor's analysis
/// (files and names). Any of them may be missing before the first check.
#[derive(Debug, Clone, Copy, Default)]
pub struct Sources<'a> {
    /// The result whose design the 3D view shows.
    pub shown: Option<&'a Checked>,
    /// The latest result: the current problems and the text checked.
    pub latest: Option<&'a Checked>,
    /// The editor's analysis of the project.
    pub analysis: Option<&'a Analysis>,
}

/// Everything matching `query` (ignoring case), grouped in [`Kind::ALL`]
/// order, at most [`LIMIT_PER_KIND`] of each, names starting with the
/// query before names merely containing it. An empty query finds nothing.
pub fn search(sources: Sources<'_>, query: &str) -> Vec<Hit> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }
    let mut hits = Vec::new();
    if let Some(design) = sources.shown.and_then(Checked::design) {
        hits.extend(parts(design, &query));
    }
    if let Some(analysis) = sources.analysis {
        hits.extend(files_and_names(analysis, &query));
    }
    if let Some(latest) = sources.latest {
        hits.extend(lines(latest, &query));
        hits.extend(problems(latest, &query));
    }
    hits
}

/// Ranks a name for a query: 0 when it starts with it, 1 when it contains
/// it, `None` otherwise.
fn rank(name: &str, query: &str) -> Option<u8> {
    let lower = name.to_lowercase();
    if lower.starts_with(query) {
        Some(0)
    } else if lower.contains(query) {
        Some(1)
    } else {
        None
    }
}

/// The best few of `ranked`, by rank and then by label.
fn best(mut ranked: Vec<(u8, Hit)>) -> Vec<Hit> {
    ranked.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.label.cmp(&b.1.label)));
    ranked
        .into_iter()
        .take(LIMIT_PER_KIND)
        .map(|(_, hit)| hit)
        .collect()
}

/// A part's or hardware's last name, so `hinge` finds `door_l.hinges[upper]`
/// by its own name as well as its full one.
fn rank_full(full: &str, query: &str) -> Option<u8> {
    let last = full.rsplit('.').next().unwrap_or(full);
    rank(last, query).or_else(|| rank(full, query).map(|_| 1))
}

fn parts(design: &SolvedDesign, query: &str) -> Vec<Hit> {
    let part = |rank: u8, label: &str, detail: String, selection: Selection| {
        (
            rank,
            Hit {
                kind: Kind::Part,
                label: label.to_owned(),
                detail,
                goal: Goal::Select(selection),
                severity: None,
            },
        )
    };
    let mut ranked = Vec::new();
    for assembly in &design.assemblies {
        if let Some(r) = rank_full(&assembly.name, query) {
            ranked.push(part(
                r,
                &assembly.name,
                "assembly".to_owned(),
                Selection::Assembly(assembly.id),
            ));
        }
    }
    for item in &design.parts {
        if let Some(r) = rank_full(&item.name, query) {
            ranked.push(part(
                r,
                &item.name,
                item.stock.name.clone(),
                Selection::Part(item.id),
            ));
        }
    }
    for hardware in &design.hardware {
        if let Some(r) = rank_full(&hardware.name, query) {
            ranked.push(part(
                r,
                &hardware.name,
                hardware.kind.clone(),
                Selection::Hardware(hardware.id),
            ));
        }
    }
    best(ranked)
}

/// A place the editor can open: a location of the analysis.
fn source_location(location: &Location) -> Option<SourceLocation> {
    Some(SourceLocation {
        path: location.document.source_path(),
        line: LineNumber::from_usize(location.line)?,
    })
}

fn files_and_names(analysis: &Analysis, query: &str) -> Vec<Hit> {
    let mut files = Vec::new();
    let mut names = Vec::new();
    // The analysis ranks them itself: files, then names, the project's
    // own first.
    for found in analysis.search(query, LIMIT_PER_KIND.saturating_mul(4)) {
        let Some(location) = source_location(&found.location) else {
            continue;
        };
        let is_file = found.detail == "file";
        let hit = Hit {
            kind: if is_file { Kind::File } else { Kind::Name },
            label: found.label,
            detail: if is_file {
                if found.location.document.is_read_only() {
                    tr("read only")
                } else {
                    String::new()
                }
            } else {
                found.detail
            },
            goal: Goal::Open(location),
            severity: None,
        };
        let list = if is_file { &mut files } else { &mut names };
        if list.len() < LIMIT_PER_KIND {
            list.push(hit);
        }
    }
    files.into_iter().chain(names).collect()
}

fn lines(latest: &Checked, query: &str) -> Vec<Hit> {
    let mut hits = Vec::new();
    let mut files: Vec<_> = latest
        .opened
        .sources
        .files()
        .filter(|(_, file)| std::path::Path::new(file.path()).starts_with(&latest.folder))
        .collect();
    files.sort_by(|a, b| a.1.path().cmp(b.1.path()));
    for (_, file) in files {
        let path = std::path::Path::new(file.path());
        let shown = path.strip_prefix(&latest.folder).unwrap_or(path);
        for (index, text) in file.text().lines().enumerate() {
            if hits.len() >= LIMIT_PER_KIND {
                return hits;
            }
            if !text.to_lowercase().contains(query) {
                continue;
            }
            let Some(line) = LineNumber::from_usize(index.saturating_add(1)) else {
                continue;
            };
            hits.push(Hit {
                kind: Kind::Line,
                label: text.trim().to_owned(),
                detail: format!("{}:{line}", shown.display()),
                goal: Goal::Open(SourceLocation {
                    path: file.path().to_owned(),
                    line,
                }),
                severity: None,
            });
        }
    }
    hits
}

fn problems(latest: &Checked, query: &str) -> Vec<Hit> {
    latest
        .problems
        .iter()
        .enumerate()
        .filter(|(_, problem)| {
            problem.message.to_lowercase().contains(query)
                || problem.code.to_lowercase().contains(query)
                || problem
                    .location
                    .as_ref()
                    .is_some_and(|l| l.to_lowercase().contains(query))
        })
        .take(LIMIT_PER_KIND)
        .map(|(index, problem)| Hit {
            kind: Kind::Problem,
            label: problem.message.clone(),
            detail: problem.location.clone().unwrap_or_default(),
            goal: Goal::Select(Selection::Problem(ProblemIndex(index))),
            severity: Some(problem.severity),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_starting_with_the_query_rank_first() {
        assert_eq!(rank("hinges", "hinge"), Some(0));
        assert_eq!(rank("stile[hinge]", "hinge"), Some(1));
        assert_eq!(rank("panel", "hinge"), None);
        assert_eq!(rank_full("door_l.hinges[upper]", "hinge"), Some(0));
        assert_eq!(rank_full("door_l.stile[hinge]", "hinge"), Some(1));
        assert_eq!(rank_full("door_l.stile[hinge]", "door"), Some(1));
    }
}
