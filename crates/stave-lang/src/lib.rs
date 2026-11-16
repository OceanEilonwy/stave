//! The `.stave` project format (grammar draft 3).
//!
//! - [`ast`]: the syntax tree produced by the parser. Every node carries a
//!   [`ast::Span`] so errors can point at source text.
//! - [`units`]: exact lengths, angles and fractions, and conversion from literals.
//! - [`placement`]: the typed placement vocabulary that `place { }` blocks
//!   lower into after name resolution. It has no way to express a coordinate.
//!
//! `docs/grammar.md` is the specification. The word lists below are the
//! grammar's; tests in this file read the document and fail if the two drift
//! apart.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod ast;
pub mod placement;
pub mod units;

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
    "as", "mirror", "about", "slide", "hinge", "pull", "fastener", "tool", "roles", "cut", "makes",
    "mark", "same", "to", "in", "replace", "unset", "rename",
];

/// The tokens after which the lexer drops a newline, besides dropping every
/// newline inside `( )` and `[ ]`. `DOC` stands for a `///` doc comment.
/// Nothing else continues a line: a newline after `:`, `=` or `{` is kept.
pub const LINE_CONTINUATION_TOKENS: &[&str] = &[
    ",", "(", "[", "+", "-", "*", "/", "==", "!=", "<", "<=", ">", ">=", "&&", "||", "!", "..",
    "within", "DOC",
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
        let sentence = between(GRAMMAR, "is one of the ten operations: ", ".\n");
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
}
