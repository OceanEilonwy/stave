//! Param kinds (grammar, "`param`" and its "Param kinds"): `param`,
//! `locked param` and `solved param`, with `private` first; the syntax
//! errors around them; the parser's recovery at `locked` and `solved`; and
//! the formatter's alignment of the word `param` across prefixes.

use stave_lang::ast::{DesignItem, File, Item, ItemKind, KindEntry, Param, ParamKind, ParamMark};
use stave_lang::{Code, Diagnostic, Parse, SourceMap, format_file, parse_file};

fn parse(text: &str) -> Parse<File> {
    let mut sources = SourceMap::new();
    let file = sources.add("test.stave", text).unwrap();
    let parse = parse_file(file, text);
    assert_eq!(parse.syntax().to_string(), text, "lossless");
    parse
}

fn clean(text: &str) -> File {
    let parse = parse(text);
    assert!(parse.diagnostics.is_empty(), "{:#?}", parse.diagnostics);
    parse.ast.unwrap()
}

/// The one diagnostic of a file with one mistake.
fn only(text: &str) -> Diagnostic {
    let parse = parse(text);
    let [only] = parse.diagnostics.as_slice() else {
        panic!("expected one diagnostic: {:#?}", parse.diagnostics);
    };
    only.clone()
}

#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "a test picks out the variant it checks, and any other is a failure or not what it counts"
)]
fn param(item: &Item) -> &Param {
    match &item.kind {
        ItemKind::Design(DesignItem::Param(param)) => param,
        other => panic!("not a param: {other:?}"),
    }
}

fn format(text: &str) -> String {
    let mut sources = SourceMap::new();
    let file = sources.add("test.stave", text).unwrap();
    let formatted = format_file(file, text).unwrap_or_else(|error| panic!("{error}"));
    let again = format_file(file, &formatted).unwrap();
    assert_eq!(again, formatted, "formatting twice changed it");
    formatted
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

#[test]
fn a_param_is_plain_locked_or_solved() {
    let file = clean(
        "stave 1\n\
         param overall_depth = 560mm within 550mm..650mm\n\
         locked param reveal = 1.5mm\n\
         solved param overall_width = 900mm within 600mm..1200mm\n\
         private solved param gap = 2mm within 1mm..3mm\n\
         private locked param inset : Length = 3mm\n\
         private param hidden = 1\n",
    );
    let kinds: Vec<(&str, ParamKind, bool)> = file
        .items
        .iter()
        .map(|item| {
            let param = param(item);
            (param.name.as_str(), param.kind(), item.private.is_some())
        })
        .collect();
    assert_eq!(
        kinds,
        [
            ("overall_depth", ParamKind::Plain, false),
            ("reveal", ParamKind::Locked, false),
            ("overall_width", ParamKind::Solved, false),
            ("gap", ParamKind::Solved, true),
            ("inset", ParamKind::Locked, true),
            ("hidden", ParamKind::Plain, true),
        ]
    );
}

#[test]
fn the_kind_keeps_the_span_of_its_word_and_the_item_starts_at_it() {
    let text = "stave 1\nlocked param reveal = 1.5mm\n";
    let file = clean(text);
    let item = &file.items[0];
    let mark = param(item).mark.expect("a mark");
    assert_eq!(mark.value, ParamMark::Locked);
    assert_eq!(mark.value.word(), "locked");
    let start = usize::try_from(mark.span.start()).unwrap();
    let end = usize::try_from(mark.span.end()).unwrap();
    assert_eq!(&text[start..end], "locked");
    assert_eq!(item.span.start(), mark.span.start());
    assert_eq!(param(&clean("stave 1\nparam a = 1\n").items[0]).mark, None);
}

#[test]
fn param_kinds_inside_components_and_kinds() {
    let file = clean(
        "stave 1\n\
         component Drawer {\n\
         \x20 /// The gap round the front.\n\
         \x20 locked param reveal = 2mm\n\
         \x20 solved param box_height = 150mm within 100mm..200mm\n\
         \x20 param opening : Region\n\
         }\n\
         kind k : joint {\n\
         \x20 roles { piece: Part }\n\
         \x20 locked param depth = 3mm\n\
         \x20 solved param width = 6mm within 4mm..8mm\n\
         }\n",
    );
    let ItemKind::Component(component) = &file.items[0].kind else {
        panic!()
    };
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "a test picks out the variant it checks, and any other is a failure or not what it counts"
    )]
    let kinds: Vec<ParamKind> = component
        .items
        .iter()
        .map(|item| match &item.node {
            DesignItem::Param(param) => param.kind(),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(
        kinds,
        [ParamKind::Locked, ParamKind::Solved, ParamKind::Plain]
    );
    assert_eq!(component.items[0].docs, ["The gap round the front."]);
    let ItemKind::Kind(kind) = &file.items[1].kind else {
        panic!()
    };
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "a test picks out the variant it checks, and any other is a failure or not what it counts"
    )]
    let kinds: Vec<ParamKind> = kind
        .body
        .entries
        .iter()
        .filter_map(|entry| match entry {
            KindEntry::Param(param) => Some(param.node.kind()),
            _ => None,
        })
        .collect();
    assert_eq!(kinds, [ParamKind::Locked, ParamKind::Solved]);
}

#[test]
fn the_trailing_locked_is_a_syntax_error_with_the_help_to_write_it_first() {
    let error = only("stave 1\nparam reveal = 1.5mm locked\nlet a = 1\n");
    assert_eq!(error.code, Code::Syntax);
    assert_eq!(error.message, "expected end of line, found 'locked'");
    assert_eq!(
        error.help.as_deref(),
        Some("write 'locked' first: locked param reveal = 1.5mm")
    );
    let error = only("stave 1\nparam b : Count = 3 within [1, 2, 3]   locked\n");
    assert_eq!(
        error.help.as_deref(),
        Some("write 'locked' first: locked param b : Count = 3 within [1, 2, 3]")
    );
    let error = only("stave 1\nlocked param reveal = 1.5mm locked\n");
    assert_eq!(error.message, "expected end of line, found 'locked'");
    assert_eq!(
        error.help.as_deref(),
        Some("a param's kind is written once, before 'param'")
    );
}

#[test]
fn a_param_kind_is_followed_only_by_param() {
    let error = only("stave 1\nlocked solved param w = 6mm within 5mm..7mm\n");
    assert_eq!(error.code, Code::Syntax);
    assert_eq!(error.message, "expected 'param', found 'solved'");
    assert_eq!(
        error.help.as_deref(),
        Some("a param is locked or solved, not both")
    );

    let error = only("stave 1\nlocked locked param reveal = 1.5mm\n");
    assert_eq!(
        error.help.as_deref(),
        Some("write 'locked' once: locked param")
    );

    let error = only("stave 1\nlocked private param reveal = 1.5mm\n");
    assert_eq!(error.message, "expected 'param', found 'private'");
    assert_eq!(
        error.help.as_deref(),
        Some("write 'private' first: private locked param")
    );

    let error = only("stave 1\nsolved let a = 1\n");
    assert_eq!(error.message, "expected 'param', found 'let'");
    assert_eq!(
        error.help.as_deref(),
        Some("only a param can be solved: solved param reveal = 1.5mm")
    );
}

#[test]
fn a_broken_param_kind_costs_one_error_and_the_next_line_still_parses() {
    let parse = parse(
        "stave 1\n\
         locked let a = 1\n\
         solved param b = 2mm within 1mm..3mm\n",
    );
    assert_eq!(parse.diagnostics.len(), 1, "{:#?}", parse.diagnostics);
    let file = parse.ast.unwrap();
    assert_eq!(file.items.len(), 1);
    assert_eq!(param(&file.items[0]).kind(), ParamKind::Solved);
}

#[test]
fn solved_is_a_keyword() {
    let error = only("stave 1\nparam solved = 1mm\n");
    assert_eq!(error.code, Code::ReservedWord);
}

#[test]
fn an_unclosed_bracket_is_closed_before_a_line_that_starts_with_locked_or_solved() {
    for prefix in ["locked", "solved"] {
        let parse = parse(&format!(
            "stave 1\n\
             let a = max(1mm,\n\
             {prefix} param b = 2mm within 1mm..3mm\n"
        ));
        let messages: Vec<String> = parse
            .diagnostics
            .iter()
            .map(|diagnostic| format!("{}: {}", diagnostic.code, diagnostic.message))
            .collect();
        assert_eq!(
            messages,
            ["unclosed-bracket: this \"(\" is never closed"],
            "{prefix}"
        );
        let file = parse.ast.unwrap();
        assert_eq!(file.items.len(), 1, "{prefix}");
        assert_eq!(param(&file.items[0]).name.as_str(), "b");
    }
}

#[test]
fn a_missing_brace_ends_a_part_at_a_locked_param() {
    let parse = parse(
        "stave 1\n\
         part side : oak {\n\
         \x20 length: 3mm\n\
         locked param reveal = 1.5mm\n",
    );
    assert_eq!(parse.diagnostics.len(), 1, "{:#?}", parse.diagnostics);
    assert_eq!(parse.diagnostics[0].code, Code::MissingBrace);
    let file = parse.ast.unwrap();
    assert_eq!(param(&file.items[0]).kind(), ParamKind::Locked);
}

// ---------------------------------------------------------------------------
// Formatting
// ---------------------------------------------------------------------------

#[test]
fn the_word_param_lines_up_across_prefixes() {
    assert_eq!(
        format(
            "stave 1\n\
             locked param reveal = 1.5mm\n\
             param overall_depth = 560mm within 550mm..650mm\n\
             solved param overall_width = 900mm within 600mm..1200mm\n"
        ),
        "stave 1\n\
         locked param reveal        = 1.5mm\n\
         \x20      param overall_depth = 560mm within 550mm..650mm\n\
         solved param overall_width = 900mm within 600mm..1200mm\n"
    );
}

#[test]
fn private_and_a_kind_together_make_the_longest_prefix() {
    assert_eq!(
        format(
            "stave 1\n\
             private locked param a = 1mm\n\
             private param bb = 2mm\n\
             solved param ccc = 3mm within 1mm..4mm\n\
             param d = 4mm\n"
        ),
        "stave 1\n\
         private locked param a   = 1mm\n\
         \x20      private param bb  = 2mm\n\
         \x20       solved param ccc = 3mm within 1mm..4mm\n\
         \x20              param d   = 4mm\n"
    );
}

#[test]
fn param_alignment_is_kept_inside_a_component_and_a_run_ends_at_a_blank_line() {
    assert_eq!(
        format(
            "stave 1\n\
             component C {\n\
             locked param reveal = 2mm\n\
             param opening : Region\n\
             \n\
             param alone = 1mm\n\
             }\n"
        ),
        "stave 1\n\
         component C {\n\
         \x20 locked param reveal  = 2mm\n\
         \x20        param opening : Region\n\
         \n\
         \x20 param alone = 1mm\n\
         }\n"
    );
}

#[test]
fn a_run_of_plain_params_is_laid_out_as_before() {
    let text = "stave 1\n\
                param a   = 1mm // one\n\
                param bbb = 2mm // two\n";
    assert_eq!(format(text), text);
}

#[test]
fn comments_after_aligned_params_stay_put() {
    let text = "stave 1\n\
                \x20      param cabinet_width = 900mm within 600mm..1200mm // wide\n\
                locked param reveal        = 1.5mm                     // gap\n";
    assert_eq!(format(text), text);
}

#[test]
fn a_param_with_one_word_too_many_is_broken_under_its_own_name() {
    for text in [
        "stave 1\nlocked solved param w = 6mm within 5mm..7mm\nlet x = w\n",
        "stave 1\nsolved private param w = 6mm within 5mm..7mm\nlet x = w\n",
    ] {
        let parse = parse(text);
        assert_eq!(parse.diagnostics.len(), 1, "{:#?}", parse.diagnostics);
        let [broken] = parse.broken.as_slice() else {
            panic!("{:#?}", parse.broken);
        };
        assert_eq!(broken.name.as_deref(), Some("w"));
        assert_eq!(parse.ast.unwrap().items.len(), 1);
    }
}
