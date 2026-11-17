//! Executable spec: the lexical and syntactic rules of `docs/grammar.md`
//! that `parser.rs` and the lexer's unit tests don't already pin down:
//! comments, block separators, doc comments, strings, and the notes and
//! help lines of the recovery rules.

use stave_lang::ast::{ExprKind, Ident, ItemKind, KindEntry, StringPart};
use stave_lang::{Diagnostic, Parse, SourceMap, ast::File, parse_file};

fn parse_text(text: &str) -> Parse<File> {
    let mut sources = SourceMap::new();
    let file = sources.add("test.stave", text).unwrap();
    parse_file(file, text)
}

fn messages(parse: &Parse<File>) -> Vec<String> {
    parse
        .diagnostics
        .iter()
        .map(|d| format!("{}: {}", d.code, d.message))
        .collect()
}

fn only(parse: &Parse<File>) -> &Diagnostic {
    let [only] = parse.diagnostics.as_slice() else {
        panic!("expected one diagnostic: {:?}", messages(parse));
    };
    assert!(
        only.help.is_some(),
        "every diagnostic has a help line: {only:?}"
    );
    only
}

fn item_names(file: &File) -> Vec<&str> {
    file.items
        .iter()
        .filter_map(|item| item.kind.name())
        .map(Ident::as_str)
        .collect()
}

// ---------------------------------------------------------------------------
// Comments
// ---------------------------------------------------------------------------

#[test]
fn block_comments_do_not_nest() {
    // The first `*/` ends the comment; ` c */` is then code.
    let parse = parse_text("stave 1\n/* a /* b */ c */\nlet after = 1\n");
    assert_eq!(
        messages(&parse),
        ["syntax: expected a declaration, found 'c'"]
    );
    assert_eq!(item_names(parse.ast.as_ref().unwrap()), ["after"]);
}

#[test]
fn comments_are_kept_but_never_parsed() {
    let parse = parse_text(
        "stave 1\n// a line comment\nlet a = 1 /* inline */ + 2 // trailing\n/* a\n   block */\nlet b = a\n",
    );
    assert!(parse.diagnostics.is_empty(), "{:?}", messages(&parse));
    assert_eq!(item_names(parse.ast.as_ref().unwrap()), ["a", "b"]);
}

#[test]
fn an_unclosed_block_comment_is_reported_at_its_start() {
    let parse = parse_text("stave 1\nlet a = 1\n/* never closed\n");
    let diagnostic = only(&parse);
    assert_eq!(diagnostic.message, "expected '*/', found end of file");
}

// ---------------------------------------------------------------------------
// Blocks
// ---------------------------------------------------------------------------

#[test]
fn block_entries_are_separated_by_newlines_or_commas() {
    for text in [
        "part p : s { length: 1mm, width: 2mm, place { ground } }\n",
        "part p : s {\n  length: 1mm\n  width: 2mm,\n  place {\n    ground\n  }\n}\n",
        "part p : s {,\n  length: 1mm,,\n  width: 2mm,\n  place { ,ground, }\n,}\n",
    ] {
        let parse = parse_text(&format!("stave 1\n{text}"));
        assert!(
            parse.diagnostics.is_empty(),
            "{text}: {:?}",
            messages(&parse)
        );
        let ItemKind::Design(stave_lang::ast::DesignItem::Part(part)) =
            &parse.ast.as_ref().unwrap().items[0].kind
        else {
            panic!("{text}");
        };
        assert!(part.name.as_str() == "p", "{text}");
    }
}

#[test]
fn an_empty_block_is_fine() {
    let parse =
        parse_text("stave 1\nscene closed \"Closed\" { }\ncomponent C {}\nkind k : joint {\n}\n");
    assert!(parse.diagnostics.is_empty(), "{:?}", messages(&parse));
    assert_eq!(
        item_names(parse.ast.as_ref().unwrap()),
        ["closed", "C", "k"]
    );
}

// ---------------------------------------------------------------------------
// Doc comments
// ---------------------------------------------------------------------------

#[test]
fn doc_comments_attach_to_the_next_item() {
    let parse = parse_text("stave 1\n/// The overhang\n/// at the front.\nlet overhang = 20mm\n");
    assert!(parse.diagnostics.is_empty(), "{:?}", messages(&parse));
    assert_eq!(
        parse.ast.unwrap().items[0].docs,
        ["The overhang", "at the front."]
    );
}

#[test]
fn in_a_kind_doc_comments_may_precede_a_param_a_require_or_a_makes() {
    let parse = parse_text(
        "stave 1\nkind k : joint {\n  /// How deep.\n  param depth = 3mm\n  /// A rule.\n  require ok : depth > 0mm \"m\"\n  /// A key.\n  makes part key : s { length: 1mm }\n}\n",
    );
    assert!(parse.diagnostics.is_empty(), "{:?}", messages(&parse));
    let ItemKind::Kind(kind) = &parse.ast.unwrap().items[0].kind else {
        panic!()
    };
    assert_eq!(kind.body.entries.len(), 3);
    assert!(matches!(kind.body.entries[2], KindEntry::Makes(_)));
}

#[test]
fn in_a_hardware_kind_doc_comments_may_precede_a_member() {
    let parse = parse_text(
        "stave 1\nkind k : slide {\n  /// Screwed to the cabinet.\n  member cabinet_member\n}\n",
    );
    assert!(parse.diagnostics.is_empty(), "{:?}", messages(&parse));
    let ItemKind::Kind(kind) = &parse.ast.unwrap().items[0].kind else {
        panic!()
    };
    let KindEntry::Member(member) = &kind.body.entries[0] else {
        panic!()
    };
    assert_eq!(member.docs, ["Screwed to the cabinet."]);
}

#[test]
fn a_doc_comment_anywhere_else_is_a_syntax_error() {
    let parse = parse_text("stave 1\nkind k : joint {\n  /// doc\n  note: \"x\"\n}\n");
    assert_eq!(
        only(&parse).message,
        "expected 'param' or 'require' or 'makes' or 'member', found a doc comment"
    );
}

// ---------------------------------------------------------------------------
// Strings
// ---------------------------------------------------------------------------

#[test]
fn a_string_ends_on_its_line() {
    let parse = parse_text("stave 1\nlet a = \"abc\nlet b = 1\n");
    let diagnostic = only(&parse);
    assert_eq!(diagnostic.message, "expected '\"', found end of line");
    assert_eq!(item_names(parse.ast.as_ref().unwrap()), ["b"]);
}

#[test]
fn escapes_are_a_closed_list() {
    let unknown = parse_text("stave 1\nlet a = \"\\q\"\n");
    assert_eq!(
        only(&unknown).message,
        "expected an escape (\\\" \\\\ \\n \\t \\{ or \\u{...}), found '\\q'"
    );
    let unicode = parse_text("stave 1\nlet a = \"\\u{zz}\"\n");
    assert!(
        only(&unicode)
            .message
            .starts_with("expected a character code such as \\u{e9}"),
        "{:?}",
        messages(&unicode)
    );
}

#[test]
fn a_string_with_every_escape_parses_to_its_text() {
    let parse =
        parse_text("stave 1\nlet a = \"tab\\t nl\\n quote\\\" back\\\\ brace\\{ u\\u{41}\"\n");
    assert!(parse.diagnostics.is_empty(), "{:?}", messages(&parse));
    let ItemKind::Design(stave_lang::ast::DesignItem::Let(binding)) =
        &parse.ast.unwrap().items[0].kind
    else {
        panic!()
    };
    let ExprKind::String(string) = &binding.value.kind else {
        panic!()
    };
    let text: String = string
        .parts
        .iter()
        .map(|part| match part {
            StringPart::Text { text, .. } => text.as_str(),
            StringPart::Interp { .. } => "{interp}",
        })
        .collect();
    assert_eq!(text, "tab\t nl\n quote\" back\\ brace{ uA");
}

// ---------------------------------------------------------------------------
// Recovery notes and help lines
// ---------------------------------------------------------------------------

#[test]
fn a_closing_brace_closes_brackets_left_open_inside_its_block() {
    let parse = parse_text("stave 1\npart p : s {\n  length: f(1mm,\n}\nlet after = 1\n");
    let diagnostic = only(&parse);
    assert_eq!(diagnostic.message, "this \"(\" is never closed");
    assert_eq!(diagnostic.notes, ["the block it is in ends on line 4"]);
    assert_eq!(
        diagnostic.help.as_deref(),
        Some("add \")\" at the end of line 3")
    );
}

#[test]
fn a_bracket_is_closed_where_two_values_meet_across_a_line_break() {
    // Line 4 ends with a value and line 5 starts with one: the `(` of line
    // 4 was forgotten there, so `against(...)` is still read as a term.
    let parse = parse_text(
        "stave 1\npart p : s {\n  place {\n    flush(a.left, inset: 10mm\n    against(b.top)\n  }\n}\nlet after = 1\n",
    );
    let diagnostic = only(&parse);
    assert_eq!(diagnostic.message, "this \"(\" is never closed");
    assert_eq!(
        diagnostic.help.as_deref(),
        Some("add \")\" at the end of line 4")
    );
    assert_eq!(item_names(parse.ast.as_ref().unwrap()), ["after"]);
    // A forgotten comma is not a forgotten bracket: the `)` comes later.
    let comma = parse_text("stave 1\nlet a = f(x: 1\n  y: 2)\n");
    assert_eq!(messages(&comma), ["syntax: expected ',' or ')', found 'y'"]);
}

#[test]
fn the_end_of_the_file_closes_brackets_still_open() {
    let parse = parse_text("stave 1\nlet a = [1, 2,\n");
    let diagnostic = only(&parse);
    assert_eq!(diagnostic.message, "this \"[\" is never closed");
    assert_eq!(diagnostic.notes, ["the file ends before it is closed"]);
}

#[test]
fn a_brace_missing_at_the_end_of_the_file() {
    let parse = parse_text("stave 1\npart p : s {\n  length: 1mm\n");
    let diagnostic = only(&parse);
    assert_eq!(
        diagnostic.message,
        "the body of part 'p' is missing its closing \"}\""
    );
    assert_eq!(
        diagnostic.help.as_deref(),
        Some("add \"}\" at the end of the file")
    );
}

#[test]
fn a_line_ending_in_an_operator_does_not_swallow_the_next_declaration() {
    // The newline after `+` would be dropped, but no expression continues
    // with a declaration: the one error is the missing operand, and the
    // next item is still declared.
    let parse = parse_text("stave 1\nlet a = 1 +\nlet b = 2\n");
    assert_eq!(
        messages(&parse),
        ["syntax: expected an expression, found end of line"]
    );
    assert_eq!(item_names(parse.ast.as_ref().unwrap()), ["b"]);
    let part = parse_text("stave 1\nlet a = 1 *\npart p : s {\n  length: 1mm\n}\n");
    assert_eq!(
        messages(&part),
        ["syntax: expected an expression, found end of line"]
    );
    assert_eq!(item_names(part.ast.as_ref().unwrap()), ["p"]);
}

#[test]
fn a_doc_comment_still_attaches_across_its_newline() {
    let parse = parse_text("stave 1\n/// The base.\nlet a = 1\n");
    assert!(parse.diagnostics.is_empty(), "{:?}", messages(&parse));
    assert_eq!(parse.ast.unwrap().items[0].docs, ["The base."]);
}

#[test]
fn a_keyword_argument_name_at_the_start_of_a_line_keeps_brackets_open() {
    // `stock:` and `part:` start lines inside the call but are followed by
    // `:`, so they don't close the bracket.
    let parse = parse_text("stave 1\nlet a = f(\n  stock: 1,\n  part: 2\n)\n");
    assert!(parse.diagnostics.is_empty(), "{:?}", messages(&parse));
}

#[test]
fn every_syntax_error_has_a_help_line() {
    for text in [
        "stave 1\n@\n",
        "stave 1\nlet a = \n",
        "stave 1\npart p : s { length 3mm }\n",
        "stave 1\nlet a = (1\nlet b = 2\n",
        "stave 1\nkind k : knob { }\n",
        "stave 1\nregion r[a,] = x\n",
        "stave 1\npart p = mirror(a)\n",
        "stave 1\nlet a = 3qq\n",
        "stave 1\nimport x\n",
        "stave 1\nlet a = f(x: 1, 2)\n",
    ] {
        let parse = parse_text(text);
        assert!(!parse.diagnostics.is_empty(), "{text:?}");
        for diagnostic in &parse.diagnostics {
            assert!(
                diagnostic.help.as_deref().is_some_and(|h| !h.is_empty()),
                "{text:?}: {diagnostic:?}"
            );
        }
    }
}
