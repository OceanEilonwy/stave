//! The `.stave` project format (grammar draft 3): parser, formatter,
//! diagnostics, and the typed syntax tree.
//!
//! - [`parse_file`] and [`parse_patch_file`] turn source text into a
//!   [`Parse`]: the lossless concrete syntax tree ([`syntax`]), the typed
//!   tree ([`ast`]) of every well-formed item, and the diagnostics.
//! - [`format_file`] lays a file out canonically ([`format`]).
//! - [`apply_patch`] applies a patch to a module's files, keeping every
//!   comment and line it doesn't touch ([`patch`]).
//! - [`diagnostics`] holds what every later stage reports with: [`FileId`],
//!   [`SourceMap`], [`Diagnostic`] and the compiler-style
//!   [`render_diagnostics`].
//! - [`units`]: exact lengths, angles and fractions, and conversion from
//!   literals.
//! - [`placement`]: the typed placement vocabulary that `place { }` blocks
//!   lower into after name resolution. It has no way to express a
//!   coordinate.
//!
//! # Pipeline
//!
//! ```text
//! text ──lexer──▶ tokens ──parser──▶ green tree (lossless) ──lower──▶ ast::File
//!          └──────────── diagnostics ────────────┘
//! ```
//!
//! The lexer applies the newline and unclosed-bracket rules, the parser
//! (hand-written recursive descent) recovers at the grammar's recovery
//! points, and lowering keeps only the items without errors. See the module
//! docs of `lexer`, `parser` and `lower` for the details and the reasons.
//!
//! ```
//! use stave_lang::{parse_file, render_diagnostics, SourceMap};
//!
//! let text = "stave 1\n\nlet overhang = 20mm\n";
//! let mut sources = SourceMap::new();
//! let file = sources.add("cabinet.stave", text).unwrap();
//! let parse = parse_file(file, text);
//! assert!(parse.diagnostics.is_empty());
//! assert_eq!(parse.syntax().to_string(), text); // lossless
//! assert_eq!(parse.ast.unwrap().items.len(), 1);
//! print!("{}", render_diagnostics(&sources, &parse.diagnostics));
//! ```
//!
//! `docs/grammar.md` is the specification. The word lists below are the
//! grammar's; tests in this file read the document and fail if the two drift
//! apart.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod ast;
pub mod diagnostics;
pub mod format;
mod lexer;
mod lower;
mod parser;
pub mod patch;
pub mod placement;
pub mod syntax;
pub mod units;

pub use diagnostics::{
    Code, Diagnostic, FileId, Label, Severity, SourceMap, render_diagnostic, render_diagnostics,
};
pub use format::{FormatError, format_file};
pub use lower::BrokenItem;
pub use patch::{
    ModuleFile, ModuleKey, PatchError, PatchErrorKind, PatchTarget, PatchedFile, apply_patch,
};
pub use syntax::{GreenNode, SyntaxKind, SyntaxNode, SyntaxToken};

/// The result of parsing one file.
#[derive(Debug, Clone)]
pub struct Parse<T> {
    /// The lossless concrete syntax tree. It is immutable and `Send + Sync`;
    /// [`Parse::syntax`] gives a walkable view of it.
    pub green: GreenNode,
    /// The typed tree of the well-formed items, or `None` when the file has
    /// no readable `stave <version>` header.
    pub ast: Option<T>,
    /// Every problem found, sorted by position.
    pub diagnostics: Vec<Diagnostic>,
    /// Items left out of [`Parse::ast`] because they contain an error.
    pub broken: Vec<BrokenItem>,
}

impl<T> Parse<T> {
    /// The root of the concrete syntax tree. Printing it gives back the
    /// source text exactly.
    pub fn syntax(&self) -> SyntaxNode {
        SyntaxNode::new_root(self.green.clone())
    }

    /// `true` if any diagnostic is an error.
    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(Diagnostic::is_error)
    }
}

/// Parses a design file: header, imports and items.
pub fn parse_file(file: FileId, text: &str) -> Parse<ast::File> {
    parse_with(file, text, parser::Entry::File, |lowering, root| {
        lowering.file(root)
    })
}

/// Parses a patch file: header and patches.
pub fn parse_patch_file(file: FileId, text: &str) -> Parse<ast::PatchFile> {
    parse_with(file, text, parser::Entry::PatchFile, |lowering, root| {
        lowering.patch_file(root)
    })
}

/// `true` if `text` is a patch file: its first declaration after the
/// `stave` header is a `patch`.
pub fn is_patch_source(text: &str) -> bool {
    if u32::try_from(text.len()).is_err() {
        return false;
    }
    lexer::lex(FileId::DETACHED, text)
        .tokens
        .iter()
        .filter(|token| !token.kind.is_trivia() && token.kind != SyntaxKind::NEWLINE)
        .nth(2)
        .is_some_and(|token| token.kind == SyntaxKind::KW_PATCH)
}

fn parse_with<T>(
    file: FileId,
    text: &str,
    entry: parser::Entry,
    lower: impl FnOnce(&mut lower::Lowering, &SyntaxNode) -> Option<T>,
) -> Parse<T> {
    if u32::try_from(text.len()).is_err() {
        let mut builder = rowan::GreenNodeBuilder::new();
        builder.start_node(SyntaxKind::ERROR.into());
        builder.finish_node();
        return Parse {
            green: builder.finish(),
            ast: None,
            diagnostics: vec![
                Diagnostic::error(
                    Code::Syntax,
                    format!(
                        "expected a file of at most 4 GiB, found {} bytes",
                        text.len()
                    ),
                    Label::new(file, ast::Span::default()),
                )
                .with_help("split the design into several files in its folder"),
            ],
            broken: Vec::new(),
        };
    }
    let lexed = lexer::lex(file, text);
    let (green, parse_diagnostics) = parser::parse(file, text, &lexed.tokens, entry);
    let mut diagnostics = lexed.diagnostics;
    diagnostics.extend(parse_diagnostics);
    let root = SyntaxNode::new_root(green.clone());
    let mut lowering = lower::Lowering::new(file, &diagnostics);
    let ast = lower(&mut lowering, &root);
    diagnostics.extend(lowering.diagnostics);
    diagnostics.sort_by_key(|diagnostic| {
        diagnostic
            .primary()
            .map(|label| (label.span.start(), label.span.end()))
    });
    Parse {
        green,
        ast,
        diagnostics,
        broken: lowering.broken,
    }
}

/// Reserved words: the grammar's `KEYWORD`. A keyword can't name a new thing,
/// but it may follow a `.` or precede a `:` (see [`ast::Name`]).
///
/// Face and direction words such as `top`, `left` or `up`, type names and
/// built-in function names are deliberately not reserved, so they can be used
/// as part names and labels.
pub const KEYWORDS: &[&str] = &[
    "stave",
    "project",
    "library",
    "import",
    "private",
    "kind",
    "recipe",
    "param",
    "let",
    "region",
    "stock",
    "part",
    "joint",
    "hardware",
    "assembly",
    "profile",
    "component",
    "scene",
    "require",
    "patch",
    "add",
    "set",
    "remove",
    "place",
    "pose",
    "at",
    "within",
    "locked",
    "solved",
    "fit",
    "true",
    "false",
];

/// Words that are ordinary identifiers except in one grammar position each:
/// `as` after an import's string, `same` as a whole index, and so on. The
/// lexer must not treat them specially; the parser checks for them by
/// position. `joint` is a base kind too, but it is a keyword and so is not
/// listed here.
pub const CONTEXTUAL_WORDS: &[&str] = &[
    "as",
    "mirror",
    "about",
    "slide",
    "hinge",
    "pull",
    "fastener",
    "tool",
    "roles",
    "cut",
    "makes",
    "member",
    "mechanism",
    "mount",
    "shape",
    "pivot",
    "slot",
    "rail",
    "couple",
    "adjust",
    "on",
    "from",
    "along",
    "mark",
    "same",
    "to",
    "in",
    "replace",
    "unset",
    "rename",
];

/// The tokens after which the lexer drops a newline, besides dropping every
/// newline inside `( )` and `[ ]`. `DOC` stands for a `///` doc comment.
/// Nothing else continues a line: a newline after `:`, `=` or `{` is kept.
pub const LINE_CONTINUATION_TOKENS: &[&str] = &[
    ",", "(", "[", "+", "-", "*", "/", "==", "!=", "<", "<=", ">", ">=", "&&", "||", "!", "..",
    "within", "DOC",
];

/// Built-in functions (grammar, "Built-in functions"): looked up when a bare
/// name is called.
pub const BUILTIN_FUNCTIONS: &[&str] = &[
    "between", "extent", "mid", "opening", "split", "cover", "overlay", "inset", "turned",
    "joined", "spliced", "box", "cylinder", "model", "ends", "pitch", "gap", "contact", "wall",
];

/// Current version written in the `stave <version>` header.
pub const FORMAT_VERSION: u32 = 1;

/// `true` if `word` is a reserved word.
pub fn is_keyword(word: &str) -> bool {
    KEYWORDS.contains(&word)
}

/// `true` if a newline directly after `token` is dropped by the lexer.
/// `token` is the token's text, or `"DOC"` for a doc comment.
pub fn continues_line(token: &str) -> bool {
    LINE_CONTINUATION_TOKENS.contains(&token)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{BaseKind, ItemTag, Operation, TypeName, Unit};
    use crate::placement::{Axis, Corner, Direction, Face, PartAxis};

    const GRAMMAR: &str = include_str!("../../../docs/grammar.md");

    /// The text between `start` and the next `end` after it.
    fn between<'a>(text: &'a str, start: &str, end: &str) -> &'a str {
        let from = text
            .find(start)
            .unwrap_or_else(|| panic!("grammar.md lost {start:?}"));
        let rest = &text[from + start.len()..];
        &rest[..rest
            .find(end)
            .unwrap_or_else(|| panic!("grammar.md lost {end:?} after {start:?}"))]
    }

    fn words(text: &str) -> Vec<&str> {
        text.split_whitespace().collect()
    }

    fn sorted<'a>(words: impl IntoIterator<Item = &'a str>) -> Vec<&'a str> {
        let mut words: Vec<_> = words.into_iter().collect();
        words.sort_unstable();
        words
    }

    #[test]
    fn the_document_is_draft_3() {
        assert!(GRAMMAR.contains("Version 1, draft 3."));
        assert_eq!(FORMAT_VERSION, 1);
    }

    #[test]
    fn keywords_match_the_grammar() {
        let listed = between(
            GRAMMAR,
            "**Keywords** (reserved; an `IDENT` can't be one of these):\n\n```\n",
            "```",
        );
        assert_eq!(words(listed), KEYWORDS);
        assert!(is_keyword("joint") && is_keyword("fit"));
        for free in [
            "top", "left", "up", "back", "as", "in", "same", "mirror", "Length", "opening",
        ] {
            assert!(!is_keyword(free), "{free} must stay usable as a name");
        }
    }

    #[test]
    fn keywords_and_contextual_words_are_distinct_words() {
        for word in KEYWORDS.iter().chain(CONTEXTUAL_WORDS) {
            assert!(ast::is_word(word), "{word}");
        }
        assert_eq!(
            sorted(KEYWORDS.iter().copied())
                .windows(2)
                .filter(|pair| pair[0] == pair[1])
                .count(),
            0
        );
        for word in CONTEXTUAL_WORDS {
            assert!(!is_keyword(word), "{word} is contextual, not reserved");
            assert!(ast::Ident::new(*word, ast::Span::default()).is_ok());
        }
    }

    #[test]
    fn contextual_words_match_the_grammar() {
        let table = between(GRAMMAR, "| Word | Special only |\n|---|---|\n", "\n\n");
        let mut listed = Vec::new();
        for row in table.lines() {
            let cell = between(row, "| ", " |");
            listed.extend(cell.split(", ").map(|word| word.trim_matches('`')));
        }
        // `joint` is listed with the base kinds in the table, and is a keyword.
        listed.retain(|word| !is_keyword(word));
        let mut listed = sorted(listed);
        listed.dedup(); // `cut` is special in two positions
        assert_eq!(listed, sorted(CONTEXTUAL_WORDS.iter().copied()));
    }

    #[test]
    fn line_continuation_tokens_match_the_grammar() {
        let listed = between(
            GRAMMAR,
            "(comments in between don't count):\n\n   ```\n",
            "```",
        );
        assert_eq!(words(listed), LINE_CONTINUATION_TOKENS);
        assert!(continues_line("&&") && continues_line("within") && continues_line("DOC"));
        for kept in [":", "=", "{", "}", ")", "]", "."] {
            assert!(
                !continues_line(kept),
                "a newline after {kept:?} must be kept"
            );
        }
    }

    #[test]
    fn builtin_functions_match_the_grammar() {
        let table = between(
            GRAMMAR,
            "| Function | Returns | Meaning |\n|---|---|---|\n",
            "\n\n",
        );
        let mut listed = Vec::new();
        for row in table.lines() {
            let cell = between(row, "| ", " |");
            for call in cell.split("`, `") {
                let call = call.trim_matches('`');
                listed.push(&call[..call.find('(').unwrap()]);
            }
        }
        assert_eq!(listed, BUILTIN_FUNCTIONS);
    }

    #[test]
    fn base_kinds_match_the_grammar() {
        let rule = between(GRAMMAR, "base_kind     = ", " ;");
        let listed: Vec<_> = rule
            .split(" | ")
            .map(|word| word.trim_matches('"'))
            .collect();
        assert_eq!(listed, BaseKind::ALL.map(BaseKind::word));
    }

    #[test]
    fn item_tags_match_the_grammar() {
        let rule = between(GRAMMAR, "item_tag      = ", " ;");
        let listed: Vec<_> = rule
            .split('|')
            .map(|word| word.trim().trim_matches('"'))
            .collect();
        assert_eq!(listed, ItemTag::ALL.map(ItemTag::keyword));
    }

    #[test]
    fn units_match_the_grammar() {
        let rule = between(GRAMMAR, "unit          = ", " ;");
        let listed: Vec<_> = rule
            .split(" | ")
            .map(|word| word.trim_matches('"'))
            .collect();
        assert_eq!(listed, Unit::ALL.map(Unit::suffix));
    }

    #[test]
    fn types_match_the_grammar() {
        let table = between(GRAMMAR, "| Type | Values |\n|---|---|\n", "\n\n");
        let mut listed = Vec::new();
        for row in table.lines() {
            let cell = between(row, "| ", " |");
            listed.extend(cell.split(", ").map(|word| word.trim_matches('`')));
        }
        assert_eq!(listed, TypeName::ALL.map(TypeName::word));
    }

    #[test]
    fn operations_match_the_grammar() {
        let sentence = between(GRAMMAR, "is one of the eleven operations: ", ".\n");
        let listed: Vec<_> = sentence
            .split(',')
            .map(|word| word.trim().trim_matches('`'))
            .collect();
        assert_eq!(
            listed,
            Operation::all().map(Operation::word).collect::<Vec<_>>()
        );

        let features = between(
            GRAMMAR,
            "| Operation | Features | Measurements |\n|---|---|---|\n",
            "\n\n",
        );
        let rows: Vec<_> = features
            .lines()
            .map(|row| between(row, "| `", "` |"))
            .collect();
        assert_eq!(
            rows, listed,
            "every operation needs a row in the features table"
        );
    }

    #[test]
    fn features_and_measurements_never_share_a_name() {
        let table = between(
            GRAMMAR,
            "| Operation | Features | Measurements |\n|---|---|---|\n",
            "\n\n",
        );
        let words = |cell: &str| -> Vec<String> {
            cell.split(',')
                .map(|word| word.trim().trim_matches('`').to_owned())
                .filter(|word| !word.is_empty())
                .collect()
        };
        for row in table.lines() {
            let cells: Vec<_> = row.split('|').map(str::trim).collect();
            let [_, operation, features, measurements, _] = cells.as_slice() else {
                panic!("malformed features row: {row}");
            };
            let measurements = words(measurements);
            for feature in words(features) {
                assert!(
                    !measurements.contains(&feature),
                    "{operation}: '{feature}' is both a feature and a measurement"
                );
            }
        }
    }

    #[test]
    fn placement_words_match_the_grammar() {
        let axes = between(
            GRAMMAR,
            "| Axis | Directions | Meaning |\n|---|---|---|\n",
            "\n\n",
        );
        let mut directions = Vec::new();
        for (row, axis) in axes.lines().zip(Axis::ALL) {
            let cells: Vec<_> = row.split('|').map(str::trim).collect();
            assert_eq!(cells[1].trim_matches('`'), axis.word());
            for word in cells[2].split(", ").map(|word| word.trim_matches('`')) {
                let direction = Direction::from_word(word).unwrap();
                assert_eq!(direction.axis(), axis);
                directions.push(direction);
            }
        }
        assert_eq!(
            sorted(directions.iter().map(|d| d.word())),
            sorted(Direction::ALL.map(Direction::word))
        );

        let faces = between(
            GRAMMAR,
            "named by the world direction they face once placed:\n",
            ". They are",
        );
        let listed: Vec<_> = faces
            .split(',')
            .map(|word| word.trim().trim_matches('`'))
            .collect();
        assert_eq!(listed, Face::ALL.map(Face::word));

        let part_axes = between(GRAMMAR, "A part also has three axes of its own: ", ".\n");
        let listed: Vec<_> = part_axes
            .split([',', ' '])
            .map(|word| word.trim_matches('`'))
            .filter(|word| PartAxis::from_word(word).is_some())
            .collect();
        assert_eq!(listed, PartAxis::ALL.map(PartAxis::word));

        for corner in Corner::ALL {
            assert!(
                GRAMMAR.contains(&format!("`{}`", corner.word())),
                "{}",
                corner.word()
            );
        }
    }

    /// Codes in the "Diagnostics" table, e.g. `syntax`, `axis-not-set`.
    fn diagnostic_codes() -> Vec<&'static str> {
        let table = between(
            GRAMMAR,
            "| Layer | Code | Message |\n|---|---|---|\n",
            "\n\n",
        );
        table
            .lines()
            .map(|row| {
                let cells: Vec<_> = row.split('|').map(str::trim).collect();
                cells[2].trim_matches('`')
            })
            .collect()
    }

    #[test]
    fn the_broken_corpus_expects_documented_diagnostics() {
        let codes = diagnostic_codes();
        assert!(codes.contains(&"syntax") && codes.contains(&"order"));

        let corpus = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/broken");
        let mut expected = Vec::new();
        for entry in std::fs::read_dir(&corpus).unwrap() {
            let folder = entry.unwrap().path();
            let source = std::fs::read_to_string(folder.join("project.stave"))
                .unwrap_or_else(|error| panic!("{}: {error}", folder.display()));
            let first = source.lines().next().unwrap();
            let rest = first
                .strip_prefix("// expect: error[")
                .or_else(|| first.strip_prefix("// expect: warning["))
                .unwrap_or_else(|| {
                    panic!(
                        "{}: first line must be `// expect: error[code] message`",
                        folder.display()
                    )
                });
            if let Some(second) = source.lines().nth(1)
                && let Some(reason) = second.strip_prefix("// pending: ")
            {
                assert!(!reason.trim().is_empty(), "{}", folder.display());
            }
            let (code, message) = rest.split_once("] ").unwrap();
            // Warnings are listed after the table; `Code::ALL` is the table
            // and that list (see `codes_match_the_grammar_table`).
            assert!(
                codes.contains(&code) || crate::Code::from_str_code(code).is_some(),
                "{}: '{code}' is not in the Diagnostics table",
                folder.display()
            );
            assert!(!message.trim().is_empty(), "{}", folder.display());
            assert!(source.contains("\nstave 1\n"), "{}", folder.display());
            assert!(source.contains("\nproject \""), "{}", folder.display());
            expected.push(code.to_owned());
        }
        expected.sort_unstable();
        assert!(
            expected.len() >= 10,
            "the corpus needs at least ten broken designs"
        );
        for required in [
            "syntax",
            "missing-value",
            "reserved-word",
            "unknown-name",
            "unit-mismatch",
            "axis-set-twice",
            "axis-not-set",
            "require-range",
            "adjust",
            "order",
        ] {
            assert!(
                expected.iter().any(|code| code == required),
                "no broken design expects '{required}'"
            );
        }
    }
}
