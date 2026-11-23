//! A file's outline and folds (grammar, "Tooling and editors": document
//! symbols and folding, "items, and the blocks inside them"), read off the
//! lossless tree, so they are there for a half-typed file too.

use stave_lang::ast::Span;
use stave_lang::{FileId, SyntaxKind, SyntaxNode};

use crate::analysis::Analysis;
use crate::index::{FileData, SymbolKind, declared_name, span_of};

/// What an outline entry is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OutlineKind {
    /// The file's `project` or `library` item.
    Header,
    /// A declaration.
    Item(SymbolKind),
    /// A block inside a declaration: `place { }`, `roles { }`,
    /// `cut dado: crossing { }`, `mechanism { }`.
    Block,
}

/// One entry of a file's outline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineEntry {
    /// Its name: `side_l`, `place`, `cut dado: crossing`.
    pub name: String,
    /// More about it: a declaration's line (`part side_l : oak_19`).
    pub detail: String,
    /// What it is.
    pub kind: OutlineKind,
    /// The whole entry, doc comments included.
    pub span: Span,
    /// Its name (the keyword for a block).
    pub name_span: Span,
    /// The declarations and blocks inside it, in order.
    pub children: Vec<OutlineEntry>,
}

/// What a fold folds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FoldKind {
    /// A `{ }` block, or a `( )` or `[ ]` written over several lines.
    Block,
    /// Comment or doc comment lines in a row.
    Comment,
    /// Imports in a row.
    Imports,
}

/// A range an editor can fold: it spans more than one line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fold {
    /// From the opening bracket (or the first line) to the closing
    /// bracket (or the end of the last line).
    pub span: Span,
    /// What it folds.
    pub kind: FoldKind,
}

/// The blocks inside declarations that the outline lists.
fn is_outlined_block(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::PLACE
            | SyntaxKind::ROLES
            | SyntaxKind::CUT
            | SyntaxKind::MECHANISM
            | SyntaxKind::STEP_BLOCK
            | SyntaxKind::SHAPE
            | SyntaxKind::MOUNT
            | SyntaxKind::ADJUST
            | SyntaxKind::CLAMP
    )
}

impl Analysis {
    /// A file's outline: its header, its declarations and the blocks
    /// inside them, nested as written.
    pub fn outline(&self, file: FileId) -> Vec<OutlineEntry> {
        let Some(root) = self.index.files.get(&file).map(FileData::root) else {
            return Vec::new();
        };
        let mut entries = Vec::new();
        outline_children(&root, &mut entries);
        entries
    }

    /// A file's folds, in order of where they start: every block over
    /// several lines, every run of comment lines and every run of imports.
    pub fn folds(&self, file: FileId) -> Vec<Fold> {
        let Some(root) = self.index.files.get(&file).map(FileData::root) else {
            return Vec::new();
        };
        let text = root.to_string();
        let several_lines = |span: Span| {
            let start = usize::try_from(span.start()).unwrap_or(0);
            let end = usize::try_from(span.end()).unwrap_or(0);
            text.get(start..end)
                .is_some_and(|inside| inside.contains('\n'))
        };
        let mut folds = Vec::new();
        let mut open: Vec<(SyntaxKind, u32)> = Vec::new();
        // Comment lines in a row: the first's start, the last's end and
        // how many lines.
        let mut comments: Option<(u32, u32, usize)> = None;
        // Line ends since the last comment: two end a run (a blank line).
        let mut line_ends: usize = 0;
        // Code is written on this line before the token: a comment after
        // it is the line's own, not part of a run of comment lines.
        let mut code_on_line = false;
        let close_comments = |comments: &mut Option<(u32, u32, usize)>, folds: &mut Vec<Fold>| {
            if let Some((start, end, lines)) = comments.take()
                && lines > 1
            {
                folds.push(Fold {
                    span: Span::new(start, end),
                    kind: FoldKind::Comment,
                });
            }
        };
        for token in root
            .descendants_with_tokens()
            .filter_map(|e| e.into_token())
        {
            let range = token.text_range();
            let (start, end) = (u32::from(range.start()), u32::from(range.end()));
            #[expect(
                clippy::wildcard_enum_match_arm,
                reason = "SyntaxKind is a flat set of token and node tags: this folds on the brackets and comments and passes over the rest"
            )]
            match token.kind() {
                SyntaxKind::L_BRACE | SyntaxKind::L_PAREN | SyntaxKind::L_BRACK => {
                    close_comments(&mut comments, &mut folds);
                    code_on_line = true;
                    open.push((token.kind(), start));
                }
                SyntaxKind::R_BRACE | SyntaxKind::R_PAREN | SyntaxKind::R_BRACK => {
                    close_comments(&mut comments, &mut folds);
                    code_on_line = true;
                    let opening = match token.kind() {
                        SyntaxKind::R_BRACE => SyntaxKind::L_BRACE,
                        SyntaxKind::R_PAREN => SyntaxKind::L_PAREN,
                        _ => SyntaxKind::L_BRACK,
                    };
                    // An unmatched bracket in broken text closes nothing.
                    if let Some(at) = open.iter().rposition(|(kind, _)| *kind == opening) {
                        let (_, from) = open.remove(at);
                        open.truncate(at);
                        let span = Span::new(from, end);
                        if several_lines(span) {
                            folds.push(Fold {
                                span,
                                kind: FoldKind::Block,
                            });
                        }
                    }
                }
                SyntaxKind::LINE_COMMENT | SyntaxKind::DOC => {
                    if code_on_line {
                        close_comments(&mut comments, &mut folds);
                    } else {
                        comments = Some(match comments {
                            Some((first, _, lines)) => (first, end, lines.saturating_add(1)),
                            None => (start, end, 1),
                        });
                    }
                    line_ends = 0;
                }
                // A line end inside brackets is lexed as whitespace.
                SyntaxKind::NEWLINE | SyntaxKind::WHITESPACE => {
                    let ends = token.text().matches('\n').count();
                    if ends > 0 {
                        code_on_line = false;
                        line_ends = line_ends.saturating_add(ends);
                        if line_ends > 1 {
                            close_comments(&mut comments, &mut folds);
                        }
                    }
                }
                _ => {
                    close_comments(&mut comments, &mut folds);
                    code_on_line = true;
                }
            }
        }
        close_comments(&mut comments, &mut folds);
        // Imports in a row.
        let imports: Vec<Span> = root
            .children()
            .filter(|node| node.kind() == SyntaxKind::IMPORT)
            .map(|node| span_of(&node))
            .collect();
        if let (Some(first), Some(last)) = (imports.first(), imports.last())
            && imports.len() > 1
        {
            folds.push(Fold {
                span: Span::new(first.start(), last.end()),
                kind: FoldKind::Imports,
            });
        }
        folds.sort_by_key(|fold| (fold.span.start(), std::cmp::Reverse(fold.span.end())));
        folds
    }
}

/// The outline entries among a node's descendants, nearest first: a
/// declaration or a listed block is an entry with its own entries inside;
/// any other node passes its entries up.
fn outline_children(node: &SyntaxNode, entries: &mut Vec<OutlineEntry>) {
    for child in node.children() {
        match outline_entry(&child) {
            Some(entry) => entries.push(entry),
            None => outline_children(&child, entries),
        }
    }
}

/// The entry a node is, if it is one.
fn outline_entry(node: &SyntaxNode) -> Option<OutlineEntry> {
    let kind = node.kind();
    let (name, name_span, kind) = if let Some(symbol) = SymbolKind::of_node(kind) {
        let (name, name_span) = declared_name(node)?;
        (name, name_span, OutlineKind::Item(symbol))
    } else if matches!(kind, SyntaxKind::PROJECT | SyntaxKind::LIBRARY) {
        let span = span_of(node);
        let word = if kind == SyntaxKind::PROJECT {
            "project"
        } else {
            "library"
        };
        let title = node
            .children()
            .find(|c| c.kind() == SyntaxKind::STRING)
            .map(|s| s.to_string())
            .unwrap_or_default();
        let name = format!("{word} {title}").trim().to_owned();
        (name, span, OutlineKind::Header)
    } else if is_outlined_block(kind) {
        let label = head(node);
        if label.is_empty() {
            return None;
        }
        let start = node
            .first_token()
            .map_or(span_of(node).start(), |t| u32::from(t.text_range().start()));
        let keyword_end = node
            .first_token()
            .map_or(start, |t| u32::from(t.text_range().end()));
        (label, Span::new(start, keyword_end), OutlineKind::Block)
    } else {
        return None;
    };
    let mut children = Vec::new();
    outline_children(node, &mut children);
    Some(OutlineEntry {
        detail: match kind {
            OutlineKind::Item(_) => head(node),
            OutlineKind::Header | OutlineKind::Block => String::new(),
        },
        name,
        kind,
        span: span_of(node),
        name_span,
        children,
    })
}

/// A node's text up to its block, on one line, comments and doc comments
/// left out: `part side_l : oak_19`, `cut dado: crossing`.
fn head(node: &SyntaxNode) -> String {
    let mut out = String::new();
    for element in node.descendants_with_tokens() {
        let Some(token) = element.into_token() else {
            continue;
        };
        #[expect(
            clippy::wildcard_enum_match_arm,
            reason = "SyntaxKind is a flat set of token tags: this stops at a block, drops comments and keeps the rest"
        )]
        match token.kind() {
            SyntaxKind::L_BRACE => break,
            SyntaxKind::LINE_COMMENT | SyntaxKind::BLOCK_COMMENT | SyntaxKind::DOC => {}
            SyntaxKind::NEWLINE => {
                if !out.trim().is_empty() {
                    break;
                }
            }
            _ => out.push_str(token.text()),
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}
