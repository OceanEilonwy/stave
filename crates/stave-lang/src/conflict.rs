//! Conflict markers (grammar, "Conflicts"): two or more versions of the
//! same lines, written between marker lines when two changes clash.
//!
//! [`find_conflicts`] finds the conflicts Stave can read, in jj's
//! `snapshot` style (every side written in full) and git's two-sided style.
//! [`side_text`] gives the file with one side chosen in every conflict, the
//! text each side is checked as; [`resolve`] settles one conflict.
//!
//! ```
//! use stave_lang::conflict::{Resolution, find_conflicts, resolve, side_text};
//!
//! let text = "stave 1\n<<<<<<< mine\nlet a = 1\n=======\nlet a = 2\n>>>>>>> theirs\n";
//! assert_eq!(find_conflicts(text).len(), 1);
//! assert_eq!(side_text(text, 1), "stave 1\nlet a = 2\n");
//! assert_eq!(
//!     resolve(text, 0, &Resolution::Keep(0)).unwrap(),
//!     "stave 1\nlet a = 1\n"
//! );
//! ```

use std::collections::BTreeSet;
use std::ops::Range;

use crate::ast::Span;
use crate::diagnostics::{Code, Diagnostic, FileId, Label};
use crate::lexer::{Token, in_conflict};
use crate::patch::TextEdit;
use crate::syntax::SyntaxKind;
use crate::{FORMAT_VERSION, parse_file};

/// The fewest marker characters in a run that make a marker line.
const SHORTEST_RUN: usize = 7;

/// One conflict: the lines from its opening marker line to its closing
/// marker line, both included.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    /// The bytes of the whole conflict, marker lines and line endings
    /// included.
    pub range: Range<usize>,
    /// The sides, in the order they are written; at least two.
    pub sides: Vec<ConflictSide>,
    /// What the sides were changed from, when the conflict says (the first
    /// one written, when it says several).
    pub base: Option<ConflictSide>,
}

/// One side of a conflict, or its base.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictSide {
    /// The marker line's label, `your saved work`; empty when the marker
    /// has none.
    pub label: String,
    /// The bytes of the side's text: the lines between its marker line and
    /// the next one, line endings included. Empty for an empty side.
    pub range: Range<usize>,
}

/// How one conflict is settled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// Keep this side (an index into [`Conflict::sides`]) and drop the rest.
    Keep(usize),
    /// Keep every side, one after the other, in the order they are written.
    KeepBoth,
    /// Write this text in place of the conflict.
    Text(String),
}

/// A way to settle one conflict that an editor or an AI tool offers beside
/// its `error[conflict]`: a text edit, not a patch, since a conflict is a
/// layer 1 problem (grammar, "Conflicts").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictFix {
    /// The text on its button: "Keep this side", "Keep the other side",
    /// "Keep side 3", "Keep both".
    pub label: String,
    /// What it does, for [`resolve`].
    pub resolution: Resolution,
    /// The same as one edit of the file's text: the conflict's bytes and
    /// what replaces them.
    pub edit: TextEdit,
}

/// Why a conflict can't be resolved.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ResolveError {
    /// The file has fewer conflicts.
    #[error("there is no conflict {conflict}; the file has {count}")]
    NoSuchConflict {
        /// The index asked for.
        conflict: usize,
        /// How many conflicts the file has.
        count: usize,
    },
    /// The conflict has fewer sides.
    #[error("there is no side {side}; the conflict has {count}")]
    NoSuchSide {
        /// The index asked for.
        side: usize,
        /// How many sides the conflict has.
        count: usize,
    },
}

/// The character a marker line repeats, by what it does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MarkerCharacter {
    /// `<`: opens a conflict.
    Open,
    /// `|`: opens the base, git style.
    GitBase,
    /// `=`: opens the second side, git style.
    GitSeparator,
    /// `%`: a jj diff section, which Stave can't read.
    Diff,
    /// `+`: opens a side, snapshot style.
    Side,
    /// `-`: opens the base, snapshot style.
    Base,
    /// `\`: continues a jj diff section's header.
    DiffHeader,
    /// `>`: closes a conflict.
    Close,
}

impl MarkerCharacter {
    fn from_byte(byte: u8) -> Option<Self> {
        Some(match byte {
            b'<' => Self::Open,
            b'|' => Self::GitBase,
            b'=' => Self::GitSeparator,
            b'%' => Self::Diff,
            b'+' => Self::Side,
            b'-' => Self::Base,
            b'\\' => Self::DiffHeader,
            b'>' => Self::Close,
            _ => return None,
        })
    }
}

/// A marker line read: its character, how many times it is repeated, and
/// its label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Marker<'a> {
    pub character: MarkerCharacter,
    pub run: usize,
    pub label: &'a str,
}

/// Reads `line` (a line without its line feed) as a marker line: a run of
/// seven or more of one marker character at its start, then a space or the
/// end of the line. A carriage return at the end counts as the end.
pub(crate) fn marker(line: &str) -> Option<Marker<'_>> {
    let line = line.strip_suffix('\r').unwrap_or(line);
    let first = *line.as_bytes().first()?;
    let character = MarkerCharacter::from_byte(first)?;
    let run = line.bytes().take_while(|&byte| byte == first).count();
    if run < SHORTEST_RUN {
        return None;
    }
    let rest = line.get(run..)?;
    let label = if rest.is_empty() {
        rest
    } else {
        rest.strip_prefix(' ')?.trim()
    };
    Some(Marker {
        character,
        run,
        label,
    })
}

/// One line of a text: where it starts, where its content ends (before the
/// line feed) and where the next line starts.
#[derive(Debug, Clone, Copy)]
struct Line {
    start: usize,
    content_end: usize,
    end: usize,
}

fn lines(text: &str) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut start = 0;
    while start < text.len() {
        let rest = text.get(start..).unwrap_or("");
        let (content_end, end) = match rest.find('\n') {
            Some(found) => (start + found, start + found + 1),
            None => (text.len(), text.len()),
        };
        lines.push(Line {
            start,
            content_end,
            end,
        });
        start = end;
    }
    lines
}

/// Every conflict Stave can read in `text`, in the order they are written.
/// Marker lines outside them (a conflict never closed, a jj diff section,
/// a stray separator) are left for the caller to report.
pub fn find_conflicts(text: &str) -> Vec<Conflict> {
    let lines = lines(text);
    let content = |line: &Line| text.get(line.start..line.content_end).unwrap_or("");
    let mut conflicts = Vec::new();
    let mut index = 0;
    while let Some(line) = lines.get(index) {
        let opening =
            marker(content(line)).filter(|found| found.character == MarkerCharacter::Open);
        match opening.and_then(|opening| read_conflict(text, &lines, index, opening)) {
            Some((conflict, next)) => {
                conflicts.push(conflict);
                index = next;
            }
            None => index += 1,
        }
    }
    conflicts
}

/// Reads the conflict opened at line `open`; returns it and the index of
/// the line after it.
fn read_conflict(
    text: &str,
    lines: &[Line],
    open: usize,
    opening: Marker<'_>,
) -> Option<(Conflict, usize)> {
    let content = |line: &Line| text.get(line.start..line.content_end).unwrap_or("");
    let mut inner: Vec<(usize, Marker<'_>)> = Vec::new();
    let mut close = None;
    for (index, line) in lines.iter().enumerate().skip(open + 1) {
        let Some(found) = marker(content(line)).filter(|found| found.run == opening.run) else {
            continue;
        };
        match found.character {
            MarkerCharacter::Close => {
                close = Some((index, found));
                break;
            }
            MarkerCharacter::Open => return None,
            MarkerCharacter::GitBase
            | MarkerCharacter::GitSeparator
            | MarkerCharacter::Diff
            | MarkerCharacter::Side
            | MarkerCharacter::Base
            | MarkerCharacter::DiffHeader => inner.push((index, found)),
        }
    }
    let (close, closing) = close?;
    // The text between marker line `from` and the next marker line `to`.
    let between = |from: usize, to: usize| -> Option<Range<usize>> {
        Some(lines.get(from)?.end..lines.get(to)?.start)
    };
    let side = |label: &str, range: Range<usize>| ConflictSide {
        label: label.to_owned(),
        range,
    };
    let characters: Vec<MarkerCharacter> = inner.iter().map(|(_, found)| found.character).collect();
    let snapshot = inner.first().is_some_and(|&(index, _)| index == open + 1)
        && characters
            .iter()
            .all(|&character| matches!(character, MarkerCharacter::Side | MarkerCharacter::Base));
    let (sides, base) = if snapshot {
        let mut sides = Vec::new();
        let mut base = None;
        for (position, &(index, found)) in inner.iter().enumerate() {
            let next = inner.get(position + 1).map_or(close, |&(next, _)| next);
            let section = side(found.label, between(index, next)?);
            if found.character == MarkerCharacter::Side {
                sides.push(section);
            } else if base.is_none() {
                base = Some(section);
            }
        }
        (sides, base)
    } else {
        match inner.as_slice() {
            [(separator, found)] if found.character == MarkerCharacter::GitSeparator => (
                vec![
                    side(opening.label, between(open, *separator)?),
                    side(closing.label, between(*separator, close)?),
                ],
                None,
            ),
            [(base, base_marker), (separator, found)]
                if base_marker.character == MarkerCharacter::GitBase
                    && found.character == MarkerCharacter::GitSeparator =>
            {
                (
                    vec![
                        side(opening.label, between(open, *base)?),
                        side(closing.label, between(*separator, close)?),
                    ],
                    Some(side(base_marker.label, between(*base, *separator)?)),
                )
            }
            _ => return None,
        }
    };
    if sides.len() < 2 {
        return None;
    }
    let range = lines.get(open)?.start..lines.get(close)?.end;
    Some((Conflict { range, sides, base }, close + 1))
}

/// `text` with side `side` chosen in every conflict: the text that side is
/// checked as. A conflict with fewer sides gives its last one.
pub fn side_text(text: &str, side: usize) -> String {
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    for conflict in find_conflicts(text) {
        out.push_str(text.get(copied..conflict.range.start).unwrap_or(""));
        if let Some(chosen) = conflict.sides.get(side).or(conflict.sides.last()) {
            out.push_str(text.get(chosen.range.clone()).unwrap_or(""));
        }
        copied = conflict.range.end;
    }
    out.push_str(text.get(copied..).unwrap_or(""));
    out
}

/// `text` with conflict number `conflict` (an index into
/// [`find_conflicts`]) settled by `resolution`; every other conflict is
/// left as it is.
///
/// # Errors
///
/// No such conflict, or no such side to keep.
pub fn resolve(
    text: &str,
    conflict: usize,
    resolution: &Resolution,
) -> Result<String, ResolveError> {
    let conflicts = find_conflicts(text);
    let count = conflicts.len();
    let found = conflicts
        .get(conflict)
        .ok_or(ResolveError::NoSuchConflict { conflict, count })?;
    let replacement = replacement(text, found, resolution)?;
    let mut out = String::with_capacity(text.len());
    out.push_str(text.get(..found.range.start).unwrap_or(""));
    out.push_str(&replacement);
    out.push_str(text.get(found.range.end..).unwrap_or(""));
    Ok(out)
}

/// What takes the place of `conflict` when it is settled by `resolution`.
fn replacement(
    text: &str,
    conflict: &Conflict,
    resolution: &Resolution,
) -> Result<String, ResolveError> {
    let side_text = |side: &ConflictSide| text.get(side.range.clone()).unwrap_or("");
    Ok(match resolution {
        Resolution::Keep(side) => {
            let kept = conflict.sides.get(*side).ok_or(ResolveError::NoSuchSide {
                side: *side,
                count: conflict.sides.len(),
            })?;
            side_text(kept).to_owned()
        }
        Resolution::KeepBoth => {
            let mut joined = String::new();
            for side in &conflict.sides {
                push_line(&mut joined, side_text(side));
            }
            joined
        }
        Resolution::Text(new) => {
            let ends_line = text
                .get(..conflict.range.end)
                .is_some_and(|before| before.ends_with('\n'));
            if ends_line {
                let mut line = String::with_capacity(new.len() + 1);
                push_line(&mut line, new);
                line
            } else {
                new.clone()
            }
        }
    })
}

/// The ways to settle conflict number `conflict` (an index into
/// [`find_conflicts`]): keep each side ("Keep this side" for the first,
/// "Keep the other side" for the second, "Keep side 3" and on), then "Keep
/// both" when every side is whole items, at least one, and no two sides
/// declare the same name. Empty when there is no such conflict.
pub fn fixes(text: &str, conflict: usize) -> Vec<ConflictFix> {
    let conflicts = find_conflicts(text);
    let Some(found) = conflicts.get(conflict) else {
        return Vec::new();
    };
    let mut resolutions: Vec<(String, Resolution)> = (0..found.sides.len())
        .map(|side| {
            let label = match side {
                0 => "Keep this side".to_owned(),
                1 => "Keep the other side".to_owned(),
                _ => format!("Keep side {}", side + 1),
            };
            (label, Resolution::Keep(side))
        })
        .collect();
    if keeping_both_is_whole(text, found) {
        resolutions.push(("Keep both".to_owned(), Resolution::KeepBoth));
    }
    resolutions
        .into_iter()
        .filter_map(|(label, resolution)| {
            let replaced = replacement(text, found, &resolution).ok()?;
            Some(ConflictFix {
                label,
                resolution,
                edit: TextEdit {
                    range: found.range.clone(),
                    text: replaced,
                },
            })
        })
        .collect()
}

/// `true` if every side of `conflict` is whole items, at least one, and no
/// two sides declare the same name.
fn keeping_both_is_whole(text: &str, conflict: &Conflict) -> bool {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for side in &conflict.sides {
        let Some(names) = text.get(side.range.clone()).and_then(item_names) else {
            return false;
        };
        for name in names {
            if seen.contains(&name) {
                return false;
            }
            seen.insert(name);
        }
    }
    true
}

/// The names `side` declares when it is whole items, at least one, read as
/// a file of its own; `None` otherwise.
fn item_names(side: &str) -> Option<BTreeSet<String>> {
    let text = format!("stave {FORMAT_VERSION}\n{side}");
    let parse = parse_file(FileId::DETACHED, &text);
    if parse.has_errors() || !parse.broken.is_empty() {
        return None;
    }
    let file = parse.ast?;
    if file.items.is_empty() || !file.imports.is_empty() {
        return None;
    }
    Some(
        file.items
            .iter()
            .filter_map(|item| item.kind.name())
            .map(|name| name.as_str().to_owned())
            .collect(),
    )
}

/// `error[conflict]` for each conflict in `text`, at its opening marker
/// line, and for each marker line outside them.
pub(crate) fn diagnostics(
    file: FileId,
    text: &str,
    conflicts: &[Conflict],
    tokens: &[Token],
) -> Vec<Diagnostic> {
    let line_span = |start: usize| {
        let line = text
            .get(start..)
            .unwrap_or("")
            .split('\n')
            .next()
            .unwrap_or("");
        let end = start + line.strip_suffix('\r').unwrap_or(line).len();
        Span::new(offset(start), offset(end))
    };
    let mut out: Vec<Diagnostic> = conflicts
        .iter()
        .map(|conflict| {
            let names: Vec<String> = conflict
                .sides
                .iter()
                .enumerate()
                .map(|(index, side)| {
                    if side.label.is_empty() {
                        format!("side {}", index + 1)
                    } else {
                        format!("'{}'", side.label)
                    }
                })
                .collect();
            let help = match conflict.sides.len() {
                2 => "This file holds two versions of the same lines. Keep one, keep both, or edit between the marker lines.".to_owned(),
                count => format!(
                    "This file holds {count} versions of the same lines. Keep one, keep them all, or edit between the marker lines."
                ),
            };
            Diagnostic::error(
                Code::Conflict,
                format!("{} change the same lines", and_list(&names)),
                Label::new(file, line_span(conflict.range.start)),
            )
            .with_help(help)
        })
        .collect();
    out.extend(
        tokens
            .iter()
            .filter(|token| {
                token.kind == SyntaxKind::CONFLICT_MARKER && !in_conflict(conflicts, token.span)
            })
            .map(|token| {
                Diagnostic::error(
                    Code::Conflict,
                    "this conflict marker line is not part of a conflict Stave can read",
                    Label::new(file, token.span),
                )
                .with_help(
                    "Delete the line, or write the whole conflict: a <<<<<<< line, each side after a +++++++ line, and a >>>>>>> line",
                )
            }),
    );
    out
}

/// `a`, `a and b`, `a, b and c`.
fn and_list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [only] => only.clone(),
        [first @ .., last] => format!("{} and {last}", first.join(", ")),
    }
}

fn offset(position: usize) -> u32 {
    u32::try_from(position).unwrap_or(u32::MAX)
}

/// Appends `text` to `out`, ending it with a line feed unless it is empty
/// or already ends with one.
fn push_line(out: &mut String, text: &str) {
    out.push_str(text);
    if !text.is_empty() && !text.ends_with('\n') {
        out.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_lines() {
        let found = marker("<<<<<<< conflict 1 of 1").unwrap();
        assert_eq!(found.character, MarkerCharacter::Open);
        assert_eq!(found.run, 7);
        assert_eq!(found.label, "conflict 1 of 1");
        assert_eq!(marker("=======").unwrap().label, "");
        assert_eq!(marker("=======\r").unwrap().label, "");
        assert_eq!(
            marker("\\\\\\\\\\\\\\ to: side").unwrap().character,
            MarkerCharacter::DiffHeader
        );
        assert_eq!(marker("%%%%%%%%%").unwrap().run, 9);
        assert!(marker("======").is_none());
        assert!(marker("=======x").is_none());
        assert!(marker("=======\tx").is_none());
        assert!(marker("////////").is_none());
        assert!(marker("").is_none());
    }
}
