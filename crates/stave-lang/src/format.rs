//! The formatter: canonical layout, applied to the lossless tree.
//!
//! `stave fmt` normalises how a file is laid out without touching what it
//! says. It works on the concrete syntax tree, so every comment and doc
//! comment survives, and it keeps the author's line breaks: a newline is
//! meaningful in `.stave` (it ends a field), so the formatter never joins or
//! splits lines. The rules:
//!
//! 1. **Indentation** is two spaces per open `{`. A line that continues an
//!    expression (after an operator, a comma or an open bracket) is indented
//!    by the bracket it is in: one step in from the line that opened the
//!    bracket when the bracket ends that line (and its closer goes back to
//!    that line's indentation), otherwise just after the bracket. A
//!    continuation outside brackets lines up with the start of the value
//!    after the line's first `:` or `=`.
//! 2. **Spacing** inside a line is one space between tokens, except: none
//!    inside `( )` and `[ ]`, before `,`, around `.` and `..`, after a
//!    prefix `-` or `!`, before the `(` of a call or the `[` of an index or
//!    array, and before the `:` of a field, argument, role or station. The
//!    `:` of a declaration (`part side_l : oak_19`) has a space on each side.
//! 3. **Alignment**: consecutive lines that each start with a field
//!    (`name:`), a named argument or a profile station, in the same block or
//!    argument list, line their values up one space after the longest key.
//!    A blank line or another entry in the same block ends the run; comment
//!    lines and the continuation lines of a value don't.
//! 4. **Comments**: a comment at the end of a line keeps its column when
//!    the code before it leaves room, so aligned comments stay aligned;
//!    otherwise it moves to one space after the code.
//! 5. **Blank lines**: at most one in a row, none at the start or end of a
//!    block or of the file. The file ends with one newline; trailing spaces
//!    and carriage returns are removed.
//!
//! The rules depend only on the formatted text, so formatting is
//! idempotent. As a safety net, the result is parsed again and must have the
//! same tokens and comments as the input; otherwise the input is returned
//! as an error instead of a wrong layout.

use std::collections::BTreeMap;

use rowan::WalkEvent;

use crate::diagnostics::{Diagnostic, FileId};
use crate::syntax::{SyntaxKind, SyntaxNode, SyntaxToken};
use crate::{is_patch_source, parse_file, parse_patch_file};

/// Why a file was not formatted.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FormatError {
    /// The file has syntax errors; it is formatted once it parses.
    #[error("the file has {} syntax error(s); fix them before formatting", .0.len())]
    Syntax(Vec<Diagnostic>),
    /// The layout would have changed the file's tokens. This is a bug in the
    /// formatter; the file is left alone.
    #[error("formatting would change the meaning of the file; please report this as a bug")]
    WouldChangeMeaning,
}

/// Formats a design file or a patch file (see the module docs for the
/// rules). `file` is used for the diagnostics of a file that doesn't parse.
pub fn format_file(file: FileId, text: &str) -> Result<String, FormatError> {
    let root = parse_root(file, text)?;
    let formatted = layout(&root, text);
    let check = parse_root(file, &formatted).map_err(|_| FormatError::WouldChangeMeaning)?;
    if signature(&root) != signature(&check) {
        return Err(FormatError::WouldChangeMeaning);
    }
    Ok(formatted)
}

fn parse_root(file: FileId, text: &str) -> Result<SyntaxNode, FormatError> {
    let (root, diagnostics) = if is_patch_source(text) {
        let parse = parse_patch_file(file, text);
        (parse.syntax(), parse.diagnostics)
    } else {
        let parse = parse_file(file, text);
        (parse.syntax(), parse.diagnostics)
    };
    if diagnostics.iter().any(Diagnostic::is_error) {
        return Err(FormatError::Syntax(diagnostics));
    }
    Ok(root)
}

/// The meaning-carrying content of a tree: its tokens and comments, with
/// runs of newlines counted once.
fn signature(root: &SyntaxNode) -> Vec<(SyntaxKind, String)> {
    let mut out: Vec<(SyntaxKind, String)> = Vec::new();
    for token in root
        .descendants_with_tokens()
        .filter_map(|element| element.into_token())
    {
        match token.kind() {
            SyntaxKind::WHITESPACE => {}
            SyntaxKind::NEWLINE => {
                if out
                    .last()
                    .is_none_or(|(kind, _)| *kind != SyntaxKind::NEWLINE)
                {
                    out.push((SyntaxKind::NEWLINE, String::new()));
                }
            }
            kind => out.push((kind, token.text().trim_end().to_owned())),
        }
    }
    while out
        .last()
        .is_some_and(|(kind, _)| *kind == SyntaxKind::NEWLINE)
    {
        out.pop();
    }
    if out
        .first()
        .is_some_and(|(kind, _)| *kind == SyntaxKind::NEWLINE)
    {
        out.remove(0);
    }
    out
}

/// One printable unit: a token, or a whole string literal.
#[derive(Debug, Clone)]
struct Atom {
    kind: SyntaxKind,
    text: String,
    /// The kind of the node the token belongs to (for strings: `STRING`).
    parent: SyntaxKind,
    /// The token (for strings: the opening quote), to look at ancestors.
    token: SyntaxToken,
    /// Column in the input, in characters.
    column: usize,
}

impl Atom {
    fn is_comment(&self) -> bool {
        matches!(
            self.kind,
            SyntaxKind::LINE_COMMENT | SyntaxKind::BLOCK_COMMENT
        )
    }
}

/// One line of the input, as atoms.
#[derive(Debug, Default)]
struct Line {
    atoms: Vec<Atom>,
    /// The line continues an expression: the newline before it was dropped.
    continuation: bool,
}

impl Line {
    fn is_blank(&self) -> bool {
        self.atoms.is_empty()
    }

    fn is_comment_only(&self) -> bool {
        !self.atoms.is_empty() && self.atoms.iter().all(Atom::is_comment)
    }

    fn first_kind(&self) -> Option<SyntaxKind> {
        self.atoms.first().map(|atom| atom.kind)
    }

    /// The last atom that is not a comment.
    fn last_code(&self) -> Option<&Atom> {
        self.atoms.iter().rev().find(|atom| !atom.is_comment())
    }
}

fn width(text: &str) -> usize {
    text.chars().count()
}

/// Splits the tree into lines of atoms.
fn lines(root: &SyntaxNode, text: &str) -> Vec<Line> {
    let mut lines = vec![Line::default()];
    let mut line_start: usize = 0;
    let mut last_code: Option<SyntaxKind> = None;
    let mut preorder = root.preorder_with_tokens();
    let column = |line_start: usize, at: usize| width(text.get(line_start..at).unwrap_or(""));

    while let Some(event) = preorder.next() {
        let WalkEvent::Enter(element) = event else {
            continue;
        };
        let (kind, atom_text, parent, token, start) = match element {
            rowan::NodeOrToken::Node(node) => {
                if node.kind() != SyntaxKind::STRING {
                    continue;
                }
                preorder.skip_subtree();
                let Some(first) = node.first_token() else {
                    continue;
                };
                let start = usize::from(node.text_range().start());
                (
                    SyntaxKind::STRING,
                    node.text().to_string(),
                    SyntaxKind::STRING,
                    first,
                    start,
                )
            }
            rowan::NodeOrToken::Token(token) => {
                let start = usize::from(token.text_range().start());
                match token.kind() {
                    SyntaxKind::WHITESPACE | SyntaxKind::NEWLINE => {
                        let dropped = token.kind() == SyntaxKind::WHITESPACE;
                        for (offset, _) in token.text().match_indices('\n') {
                            line_start = start + offset + 1;
                            lines.push(Line {
                                atoms: Vec::new(),
                                continuation: dropped && last_code != Some(SyntaxKind::DOC),
                            });
                        }
                        continue;
                    }
                    kind => {
                        let parent = token.parent().map_or(SyntaxKind::ERROR, |p| p.kind());
                        (kind, token.text().to_owned(), parent, token, start)
                    }
                }
            }
        };
        let atom = Atom {
            kind,
            column: column(line_start, start),
            text: atom_text,
            parent,
            token,
        };
        if let Some(newline) = atom.text.rfind('\n') {
            line_start = start + newline + 1;
        }
        if !atom.is_comment() {
            last_code = Some(atom.kind);
        }
        if let Some(line) = lines.last_mut() {
            line.atoms.push(atom);
        }
    }
    lines
}

/// `true` if there is a space between two atoms on one line.
fn space_between(previous: &Atom, next: &Atom) -> bool {
    use SyntaxKind::*;
    if next.is_comment() || previous.kind == BLOCK_COMMENT {
        return true;
    }
    if matches!(next.kind, R_PAREN | R_BRACK | COMMA | DOT)
        || matches!(previous.kind, L_PAREN | L_BRACK | DOT)
        || previous.kind == DOT2
        || next.kind == DOT2
    {
        return false;
    }
    if matches!(previous.kind, MINUS | BANG) && previous.parent == UNARY_EXPR {
        return false;
    }
    match next.kind {
        COLON => !matches!(next.parent, FIELD | ARG | ROLE | STATION | MIRROR),
        L_PAREN => !matches!(next.parent, ARG_LIST | MIRROR),
        L_BRACK => !matches!(next.parent, INDEX | ARRAY_SPEC),
        _ => true,
    }
}

/// The number of leading atoms that form an alignable key (`name:`,
/// `at 34mm:`), and the block or argument list the key belongs to.
fn key(line: &Line) -> Option<(usize, u32)> {
    let first = line.atoms.first()?;
    let (colon_index, colon) =
        if first.kind == SyntaxKind::KW_AT && first.parent == SyntaxKind::STATION {
            line.atoms.iter().enumerate().find(|(_, atom)| {
                atom.kind == SyntaxKind::COLON && atom.parent == SyntaxKind::STATION
            })?
        } else {
            let second = line.atoms.get(1)?;
            let keyed = matches!(
                second.parent,
                SyntaxKind::FIELD | SyntaxKind::ARG | SyntaxKind::ROLE
            );
            if !(first.kind.is_word() && second.kind == SyntaxKind::COLON && keyed) {
                return None;
            }
            (1, second)
        };
    // The key needs a value after it on the same line to be aligned.
    line.atoms
        .get(colon_index + 1)
        .filter(|atom| !atom.is_comment())?;
    let container = colon.token.parent()?.parent()?;
    Some((colon_index + 1, container.text_range().start().into()))
}

/// The block or argument list a line's first atom belongs to.
fn container(line: &Line) -> Option<u32> {
    let first = line.atoms.first()?;
    first
        .token
        .parent_ancestors()
        .find(|node| {
            matches!(
                node.kind(),
                SyntaxKind::BLOCK | SyntaxKind::ARG_LIST | SyntaxKind::LIST_EXPR
            )
        })
        .map(|node| node.text_range().start().into())
}

/// The width of atoms laid out on one line with the spacing rules.
fn atoms_width(atoms: &[Atom]) -> usize {
    let mut total = 0;
    for (index, atom) in atoms.iter().enumerate() {
        if index > 0
            && atoms
                .get(index - 1)
                .is_some_and(|previous| space_between(previous, atom))
        {
            total += 1;
        }
        total += width(&atom.text);
    }
    total
}

/// For every line that starts an aligned run member: the width its key is
/// padded to (the longest key in its run).
fn alignment(lines: &[Line]) -> Vec<Option<(usize, usize)>> {
    let mut result: Vec<Option<(usize, usize)>> = vec![None; lines.len()];
    // Open runs by container: the line indexes in the run.
    let mut open: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
    let mut runs: Vec<Vec<usize>> = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if line.is_blank() {
            runs.extend(std::mem::take(&mut open).into_values());
            continue;
        }
        if line.is_comment_only() {
            continue;
        }
        if let Some((_, container)) = key(line) {
            open.entry(container).or_default().push(index);
        } else if let Some(container) = container(line)
            && let Some(run) = open.remove(&container)
        {
            runs.push(run);
        }
    }
    runs.extend(open.into_values());
    for run in runs {
        let keys: Vec<(usize, usize)> = run
            .iter()
            .filter_map(|&index| {
                let line = lines.get(index)?;
                let (count, _) = key(line)?;
                Some((index, atoms_width(line.atoms.get(..count)?)))
            })
            .collect();
        let longest = keys
            .iter()
            .map(|(_, key_width)| *key_width)
            .max()
            .unwrap_or(0);
        for (index, _) in keys {
            if let (Some(slot), Some(line)) = (result.get_mut(index), lines.get(index)) {
                *slot = key(line).map(|(count, _)| (count, longest));
            }
        }
    }
    result
}

/// An open `(` or `[` while laying out.
struct Frame {
    /// Indentation of the lines inside it.
    indent: usize,
    /// Indentation of a line that starts with its closer.
    closer: usize,
}

fn layout(root: &SyntaxNode, text: &str) -> String {
    let lines = lines(root, text);
    let aligned = alignment(&lines);
    let mut out = String::with_capacity(text.len());
    let mut depth: usize = 0;
    let mut frames: Vec<Frame> = Vec::new();
    let mut anchor: usize = 0;
    let mut pending_blank = false;
    let mut previous_opens_block = true; // the start of the file

    for (index, line) in lines.iter().enumerate() {
        if line.is_blank() {
            pending_blank = true;
            continue;
        }
        let closes_block = line.first_kind() == Some(SyntaxKind::R_BRACE);
        if pending_blank && !previous_opens_block && !closes_block {
            out.push('\n');
        }
        pending_blank = false;

        let indent = if line.continuation {
            match frames.last() {
                Some(frame) => {
                    if matches!(
                        line.first_kind(),
                        Some(SyntaxKind::R_PAREN | SyntaxKind::R_BRACK)
                    ) {
                        frame.closer
                    } else {
                        frame.indent
                    }
                }
                None => anchor,
            }
        } else {
            2 * depth.saturating_sub(usize::from(closes_block))
        };

        let original_indent = line.atoms.first().map_or(0, |atom| atom.column);
        let mut rendered = " ".repeat(indent);
        let mut column = indent;
        let alignment = aligned.get(index).copied().flatten();
        let mut statement_anchor: Option<usize> = None;
        let last_code_index = line.atoms.iter().rposition(|atom| !atom.is_comment());

        for (position, atom) in line.atoms.iter().enumerate() {
            if let Some(previous) = position.checked_sub(1).and_then(|p| line.atoms.get(p)) {
                let target = match alignment {
                    Some((count, key_width)) if position == count => indent + key_width + 1,
                    // Keep the comment's column, moved with the line's
                    // indentation.
                    _ if atom.kind == SyntaxKind::LINE_COMMENT => (atom.column + indent)
                        .saturating_sub(original_indent)
                        .max(column + 1),
                    _ if space_between(previous, atom) => column + 1,
                    _ => column,
                };
                while column < target {
                    rendered.push(' ');
                    column += 1;
                }
                if !line.continuation
                    && statement_anchor.is_none()
                    && matches!(previous.kind, SyntaxKind::COLON | SyntaxKind::EQ)
                {
                    statement_anchor = Some(column);
                }
            }
            rendered.push_str(&atom.text);
            column = match atom.text.rfind('\n') {
                Some(newline) => width(atom.text.get(newline + 1..).unwrap_or("")),
                None => column + width(&atom.text),
            };
            match atom.kind {
                SyntaxKind::L_BRACE => depth += 1,
                SyntaxKind::R_BRACE => depth = depth.saturating_sub(1),
                SyntaxKind::L_PAREN | SyntaxKind::L_BRACK => {
                    let opener = column.saturating_sub(1);
                    frames.push(if Some(position) == last_code_index {
                        Frame {
                            indent: indent + 2,
                            closer: indent,
                        }
                    } else {
                        Frame {
                            indent: column,
                            closer: opener,
                        }
                    });
                }
                SyntaxKind::R_PAREN | SyntaxKind::R_BRACK => {
                    frames.pop();
                }
                _ => {}
            }
        }
        if !line.continuation && !line.is_comment_only() {
            anchor = statement_anchor.unwrap_or(indent + 4);
        }
        out.push_str(rendered.trim_end());
        out.push('\n');
        previous_opens_block = line
            .last_code()
            .is_some_and(|atom| atom.kind == SyntaxKind::L_BRACE);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::SourceMap;

    fn format(text: &str) -> String {
        let mut sources = SourceMap::new();
        let file = sources.add("test.stave", text).unwrap();
        let formatted = format_file(file, text).unwrap_or_else(|error| panic!("{error}"));
        let again = format_file(file, &formatted).unwrap();
        assert_eq!(again, formatted, "not idempotent");
        formatted
    }

    #[test]
    fn indents_and_spaces() {
        assert_eq!(
            format(
                "stave   1\n\n\n\npart  side_r=mirror( side_l ,about :mid(base,across) )\npart a:s{\n\n    length : -x+2*( y )\n\twidth:a . b[ 0 ]\n\n  place{ground}\n\n}\n\n\n"
            ),
            "stave 1\n\npart side_r = mirror(side_l, about: mid(base, across))\npart a : s {\n  length: -x + 2 * (y)\n  width:  a.b[0]\n\n  place { ground }\n}\n"
        );
    }

    #[test]
    fn aligns_fields_in_runs() {
        assert_eq!(
            format(
                "stave 1\nproject \"P\" {\n  goal: \"g\"\n  // a comment\n  resolution: 0.5mm\n\n  units: mm\n  tools: [a,\n b]\n}\n"
            ),
            "stave 1\nproject \"P\" {\n  goal:       \"g\"\n  // a comment\n  resolution: 0.5mm\n\n  units: mm\n  tools: [a,\n          b]\n}\n"
        );
        assert_eq!(
            format(
                "stave 1\nregion o = opening(\nleft: a,\n    bottom: b,\n)\nprofile p {\n  measure: diameter\n  at 0mm: 70mm\n  at 34mm: 70mm\n}\n"
            ),
            "stave 1\nregion o = opening(\n  left:   a,\n  bottom: b,\n)\nprofile p {\n  measure: diameter\n  at 0mm:  70mm\n  at 34mm: 70mm\n}\n"
        );
    }

    #[test]
    fn keeps_comments_and_their_columns() {
        assert_eq!(
            format(
                "stave 1\n// head\nlet a = 1      // aligned\nlet bb  =  2   // aligned\nlet ccccccccccc = 3 // pushed\n/// doc\n   part x : s { length: 1mm } /* b */ // end\n"
            ),
            "stave 1\n// head\nlet a = 1      // aligned\nlet bb = 2     // aligned\nlet ccccccccccc = 3 // pushed\n/// doc\npart x : s { length: 1mm } /* b */ // end\n"
        );
    }

    #[test]
    fn continuation_lines() {
        assert_eq!(
            format(
                "stave 1\nrequire square : a == b &&\n c == d \"m\"\nkind k : joint {\n  insert: [\n      push(a),\n        push(b),\n  ]\n}\n"
            ),
            "stave 1\nrequire square : a == b &&\n                 c == d \"m\"\nkind k : joint {\n  insert: [\n    push(a),\n    push(b),\n  ]\n}\n"
        );
    }

    #[test]
    fn refuses_files_with_errors() {
        let mut sources = SourceMap::new();
        let file = sources.add("bad.stave", "stave 1\npart a : s {\n").unwrap();
        assert!(matches!(
            format_file(file, "stave 1\npart a : s {\n"),
            Err(FormatError::Syntax(diagnostics)) if diagnostics.len() == 1
        ));
    }

    #[test]
    fn formats_patch_files() {
        assert_eq!(
            format("stave 1\npatch \"p\" {\n set a.b=1mm\n    rename part a to b\n}\n"),
            "stave 1\npatch \"p\" {\n  set a.b = 1mm\n  rename part a to b\n}\n"
        );
    }
}
