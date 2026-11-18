//! Text edits on a file: where lines start and end, removing an entry with
//! its line, inserting a new line, and laying out the lines a patch wrote.

use std::ops::Range;

use crate::diagnostics::FileId;
use crate::format::{laid_out_lines, signature};
use crate::syntax::SyntaxNode;
use crate::{lexer, parse_file};

/// One replacement in a file's text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Edit {
    /// The bytes replaced.
    pub range: Range<usize>,
    /// What replaces them.
    pub text: String,
    /// Lay out the lines this edit touches as the formatter would. Off for
    /// an edit that only deletes whole lines.
    pub relayout: bool,
}

impl Edit {
    /// Inserts `text` at `at`; the lines it writes are laid out.
    pub fn insert(at: usize, text: String) -> Self {
        Self {
            range: at..at,
            text,
            relayout: true,
        }
    }

    /// Replaces `range` with `text`; the lines it touches are laid out.
    pub fn replace(range: Range<usize>, text: String) -> Self {
        Self {
            range,
            text,
            relayout: true,
        }
    }
}

/// The byte range of a node.
pub(super) fn range(node: &SyntaxNode) -> Range<usize> {
    let range = node.text_range();
    usize::from(range.start())..usize::from(range.end())
}

/// Applies non-overlapping edits. Returns the new text and, in it, the
/// ranges whose lines are to be laid out; `None` if two edits overlap or an
/// edit is out of bounds.
pub(super) fn apply(text: &str, mut edits: Vec<Edit>) -> Option<(String, Vec<Range<usize>>)> {
    edits.sort_by_key(|edit| (edit.range.start, edit.range.end));
    let mut out = String::with_capacity(text.len());
    let mut touched = Vec::new();
    let mut copied = 0;
    for edit in edits {
        if edit.range.start < copied || edit.range.end < edit.range.start {
            return None;
        }
        out.push_str(text.get(copied..edit.range.start)?);
        let start = out.len();
        out.push_str(&edit.text);
        if edit.relayout {
            touched.push(start..out.len());
        }
        copied = edit.range.end;
    }
    out.push_str(text.get(copied..)?);
    Some((out, touched))
}

/// The offset where the line holding `at` starts.
pub(super) fn line_start(text: &str, at: usize) -> usize {
    text.get(..at)
        .and_then(|before| before.rfind('\n'))
        .map_or(0, |newline| newline + 1)
}

/// The offset of the newline ending the line holding `at`, or the end of
/// the text.
pub(super) fn line_end(text: &str, at: usize) -> usize {
    text.get(at..)
        .and_then(|after| after.find('\n'))
        .map_or(text.len(), |newline| at + newline)
}

/// `true` when `text` holds only spaces, tabs and carriage returns.
pub(super) fn is_space(text: &str) -> bool {
    text.chars().all(|c| matches!(c, ' ' | '\t' | '\r'))
}

/// The indentation of the line holding `at`.
pub(super) fn indentation(text: &str, at: usize) -> &str {
    let start = line_start(text, at);
    let line = text.get(start..line_end(text, start)).unwrap_or("");
    let width = line.len() - line.trim_start_matches([' ', '\t']).len();
    line.get(..width).unwrap_or("")
}

/// Text cut from a patch, moved to a new indentation: its first line is
/// taken as is (the caller writes the indentation before it); each later
/// line loses `from` (the indentation of the line it started on in the
/// patch) and gains `to`. Trailing spaces go, and runs of blank lines
/// become one.
pub(super) fn reindent(snippet: &str, from: &str, to: &str) -> String {
    let mut out = String::with_capacity(snippet.len());
    let mut blank_run = false;
    for (index, line) in snippet.lines().enumerate() {
        let line = line.trim_end();
        if index == 0 {
            out.push_str(line);
            continue;
        }
        if line.is_empty() {
            if !blank_run {
                out.push('\n');
            }
            blank_run = true;
            continue;
        }
        blank_run = false;
        out.push('\n');
        out.push_str(to);
        out.push_str(line.strip_prefix(from).unwrap_or(line.trim_start()));
    }
    out
}

/// Removes an entry (a field, a placement term, an item, an import) given
/// its byte range. When the entry has its line to itself, the whole line
/// goes, the comment at its end included, and a blank line left doubled or
/// at the edge of a block goes with it. Otherwise the entry and one comma
/// next to it go.
pub(super) fn remove_entry(text: &str, entry: Range<usize>) -> Edit {
    let start = line_start(text, entry.start);
    let end = line_end(text, entry.end);
    let before = text.get(start..entry.start).unwrap_or("");
    let after = text
        .get(entry.end..end)
        .unwrap_or("")
        .trim_start_matches([' ', '\t']);
    let rest_of_line = |after: &str| is_space(after) || after.starts_with("//");
    let alone = is_space(before)
        && (rest_of_line(after)
            || after
                .strip_prefix(',')
                .is_some_and(|after| rest_of_line(after.trim_start_matches([' ', '\t']))));
    if alone {
        let mut from = start;
        let mut to = (end + 1).min(text.len());
        let previous = (start > 0).then(|| {
            let previous_start = line_start(text, start - 1);
            text.get(previous_start..start - 1)
                .map(|line| (previous_start, line))
        });
        let next = text.get(to..line_end(text, to)).unwrap_or("");
        let at_end = to >= text.len();
        let next_blank = !at_end && is_space(next);
        let next_closes = next.trim_start().starts_with('}');
        match previous.flatten() {
            Some((previous_start, line)) if is_space(line) => {
                if next_blank || next_closes || at_end {
                    from = previous_start;
                }
            }
            Some((_, line)) if line.trim_end().ends_with('{') && next_blank => {
                to = (line_end(text, to) + 1).min(text.len());
            }
            Some(_) | None => {}
        }
        return Edit {
            range: from..to,
            text: String::new(),
            relayout: false,
        };
    }
    let bytes = text.as_bytes();
    let mut back = entry.start;
    while back > start && matches!(bytes.get(back - 1), Some(b' ' | b'\t')) {
        back -= 1;
    }
    if back > start && bytes.get(back - 1) == Some(&b',') {
        return Edit::replace(back - 1..entry.end, String::new());
    }
    let mut forward = entry.end;
    while matches!(bytes.get(forward), Some(b' ' | b'\t')) {
        forward += 1;
    }
    if bytes.get(forward) == Some(&b',') {
        forward += 1;
        while matches!(bytes.get(forward), Some(b' ' | b'\t')) {
            forward += 1;
        }
        return Edit::replace(entry.start..forward, String::new());
    }
    Edit::replace(back..entry.end, String::new())
}

/// The significant tokens of a piece of text, to tell whether two values
/// say the same thing however they are laid out.
pub(super) fn tokens(text: &str) -> Vec<(crate::SyntaxKind, &str)> {
    lexer::lex(FileId::DETACHED, text)
        .tokens
        .iter()
        .filter(|token| !token.kind.is_trivia())
        .filter_map(|token| {
            let range = usize::try_from(token.span.start()).ok()?
                ..usize::try_from(token.span.end()).ok()?;
            Some((token.kind, text.get(range)?))
        })
        .collect()
}

/// Lays out, as the formatter would, every line that overlaps one of the
/// `touched` ranges, and the other lines of an aligned run such a line is
/// in (a longer name added to a run of regions moves every `=` of the
/// run), and leaves every other line byte for byte as it is. A file that
/// doesn't parse is returned unchanged.
pub(super) fn relayout(text: &str, touched: &[Range<usize>]) -> String {
    if touched.is_empty() {
        return text.to_owned();
    }
    let parse = parse_file(FileId::DETACHED, text);
    if parse.has_errors() {
        return text.to_owned();
    }
    let root = parse.syntax();
    let overlaps = |start: usize, end: usize| {
        touched.iter().any(|range| {
            if range.is_empty() {
                start < range.start && range.start <= end
            } else {
                range.start < end && start < range.end
            }
        })
    };
    let lines = laid_out_lines(&root, text);
    let runs: std::collections::BTreeSet<usize> = lines
        .iter()
        .filter(|line| overlaps(line.start, line.end))
        .filter_map(|line| line.run)
        .collect();
    let edits: Vec<Edit> = lines
        .into_iter()
        .filter(|line| {
            overlaps(line.start, line.end) || line.run.is_some_and(|run| runs.contains(&run))
        })
        .filter(|line| text.get(line.start..line.end) != Some(line.text.as_str()))
        .map(|line| Edit::replace(line.start..line.end, line.text))
        .collect();
    let Some((laid_out, _)) = apply(text, edits) else {
        return text.to_owned();
    };
    let check = parse_file(FileId::DETACHED, &laid_out);
    if check.has_errors() || signature(&check.syntax()) != signature(&root) {
        return text.to_owned();
    }
    laid_out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn removed(text: &str, entry: &str) -> String {
        let start = text.find(entry).unwrap();
        let edit = remove_entry(text, start..start + entry.len());
        apply(text, vec![edit]).unwrap().0
    }

    #[test]
    fn removes_a_line_and_a_doubled_blank_line() {
        assert_eq!(
            removed("a\n\nb // c\n\nd\n", "b"),
            "a\n\nd\n",
            "the comment at the end of the line goes with it"
        );
        assert_eq!(removed("x {\n  a\n  b\n}\n", "b"), "x {\n  a\n}\n");
        assert_eq!(removed("x {\n\n  a\n\n  b\n}\n", "b"), "x {\n\n  a\n}\n");
        assert_eq!(removed("x {\n  a\n\n  b\n}\n", "a"), "x {\n  b\n}\n");
    }

    #[test]
    fn removes_an_entry_sharing_its_line() {
        assert_eq!(removed("f(a, b: 1)", "b: 1"), "f(a)");
        assert_eq!(removed("f(a: 1, b)", "a: 1"), "f(b)");
        assert_eq!(removed("x { a: 1 }", "a: 1"), "x { }");
        assert_eq!(removed("f(\n  a,\n  b: 1,\n)", "b: 1"), "f(\n  a,\n)");
    }

    #[test]
    fn reindents_later_lines() {
        assert_eq!(
            reindent("part a {\n      x: 1\n\n\n    }  ", "    ", "  "),
            "part a {\n    x: 1\n\n  }"
        );
    }
}
