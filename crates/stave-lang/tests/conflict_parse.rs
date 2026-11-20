//! Conflicted files (grammar, "Conflicts"): the tree, `error[conflict]`,
//! its fixes, the formatter, and checking each side. The files are
//! `tests/conflict_*.stave`.

use stave_lang::conflict::{Resolution, find_conflicts, fixes, resolve, side_text};
use stave_lang::{
    Code, FormatError, Parse, SourceMap, SyntaxKind, apply_text_edits, ast, format_file,
    parse_file, render_diagnostics,
};

/// Each corpus file and the number of marker lines its one conflict has.
const CORPUS: [(&str, usize); 3] = [
    ("conflict_snapshot.stave", 5),
    ("conflict_git.stave", 4),
    ("conflict_in_block.stave", 3),
];

fn corpus(name: &str) -> String {
    let path = format!("{}/tests/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{path}: {error}"))
}

fn parse(name: &str, text: &str) -> Parse<ast::File> {
    let mut sources = SourceMap::new();
    let file = sources.add(name, text).unwrap();
    parse_file(file, text)
}

fn render(name: &str, text: &str) -> String {
    let mut sources = SourceMap::new();
    let file = sources.add(name, text).unwrap();
    render_diagnostics(&sources, &parse_file(file, text).diagnostics)
}

#[test]
fn conflicted_file_round_trips() {
    for (name, marker_count) in CORPUS {
        let text = corpus(name);
        let root = parse(name, &text).syntax();
        assert_eq!(root.to_string(), text, "{name}");
        let conflicts: Vec<_> = root
            .descendants()
            .filter(|node| node.kind() == SyntaxKind::CONFLICT)
            .collect();
        let [node] = conflicts.as_slice() else {
            panic!("{name}: one CONFLICT node, found {}", conflicts.len());
        };
        let found = &find_conflicts(&text)[0];
        let range = node.text_range();
        assert_eq!(
            usize::from(range.start())..usize::from(range.end()),
            found.range,
            "{name}"
        );
        let sides: Vec<String> = node
            .children()
            .filter(|child| child.kind() == SyntaxKind::CONFLICT_SIDE)
            .map(|child| child.to_string())
            .collect();
        let mut expected: Vec<_> = found.sides.iter().chain(&found.base).collect();
        expected.sort_by_key(|side| side.range.start);
        let expected: Vec<&str> = expected
            .iter()
            .map(|side| &text[side.range.clone()])
            .collect();
        assert_eq!(sides, expected, "{name}");
        let markers = node
            .children_with_tokens()
            .filter(|element| element.kind() == SyntaxKind::CONFLICT_MARKER)
            .count();
        assert_eq!(markers, marker_count, "{name}");
    }
}

#[test]
fn an_empty_side_gets_an_empty_node() {
    let text = "stave 1\n<<<<<<< a\nlet x = 1\n=======\n>>>>>>> b\nlet y = 2\n";
    let root = parse("a", text).syntax();
    assert_eq!(root.to_string(), text);
    let sides: Vec<String> = root
        .descendants()
        .filter(|node| node.kind() == SyntaxKind::CONFLICT_SIDE)
        .map(|node| node.to_string())
        .collect();
    assert_eq!(sides, ["let x = 1\n", ""]);
}

#[test]
fn the_rest_of_a_conflicted_file_parses() {
    for (name, _) in CORPUS {
        let text = corpus(name);
        let parsed = parse(name, &text);
        let codes: Vec<Code> = parsed.diagnostics.iter().map(|d| d.code).collect();
        assert_eq!(codes, [Code::Conflict], "{name}: {:?}", parsed.diagnostics);
        assert!(parsed.ast.is_some(), "{name}");
    }
    let text = corpus("conflict_snapshot.stave");
    let parsed = parse("a", &text);
    let names: Vec<&str> = parsed
        .ast
        .as_ref()
        .unwrap()
        .items
        .iter()
        .filter_map(|item| item.kind.name())
        .map(ast::Ident::as_str)
        .collect();
    assert_eq!(names, ["shelf_length", "oak_18", "pine_18", "long_enough"]);
}

#[test]
fn conflict_diagnostic_has_help_and_fixes() {
    let mut out = String::new();
    for (name, _) in CORPUS {
        let text = corpus(name);
        out.push_str(&render(name, &text));
        for fix in fixes(&text, 0) {
            out.push_str(&format!("fix: {} ({:?})\n", fix.label, fix.resolution));
            let fixed = apply_text_edits(&text, std::slice::from_ref(&fix.edit)).unwrap();
            assert_eq!(Ok(fixed.clone()), resolve(&text, 0, &fix.resolution));
            assert!(find_conflicts(&fixed).is_empty(), "{name}: {}", fix.label);
            let parsed = parse(name, &fixed);
            assert!(
                parsed.diagnostics.is_empty(),
                "{name}: {}: {:?}",
                fix.label,
                parsed.diagnostics
            );
        }
        out.push('\n');
    }
    insta::assert_snapshot!(out);
}

#[test]
fn keep_both_is_offered_only_for_whole_items_with_distinct_names() {
    let labels =
        |text: &str| -> Vec<String> { fixes(text, 0).into_iter().map(|fix| fix.label).collect() };
    // Both sides declare `shelf`.
    assert_eq!(
        labels(&corpus("conflict_snapshot.stave")),
        ["Keep this side", "Keep the other side"]
    );
    assert_eq!(
        labels(&corpus("conflict_git.stave")),
        ["Keep this side", "Keep the other side", "Keep both"]
    );
    // Fields, not items.
    assert_eq!(
        labels(&corpus("conflict_in_block.stave")),
        ["Keep this side", "Keep the other side"]
    );
    // An empty side is not whole items.
    let text = "stave 1\n<<<<<<< a\nlet x = 1\n=======\n>>>>>>> b\n";
    assert_eq!(labels(text), ["Keep this side", "Keep the other side"]);
    assert!(fixes(text, 1).is_empty());
    let three = "stave 1\n<<<<<<< c\n+++++++ a\nlet x = 1\n+++++++ b\nlet y = 1\n+++++++ c\nlet z = 1\n>>>>>>> end\n";
    assert_eq!(
        labels(three),
        [
            "Keep this side",
            "Keep the other side",
            "Keep side 3",
            "Keep both"
        ]
    );
    assert_eq!(
        resolve(three, 0, &Resolution::KeepBoth),
        Ok("stave 1\nlet x = 1\nlet y = 1\nlet z = 1\n".to_owned())
    );
}

#[test]
fn stray_marker_lines_are_each_a_conflict_error() {
    let text = "stave 1\nlet a = 1\n=======\nlet b = 2\n>>>>>>> theirs\n";
    insta::assert_snapshot!(render("stray.stave", text));
    let parsed = parse("stray.stave", text);
    assert_eq!(parsed.ast.unwrap().items.len(), 2);
}

#[test]
fn format_leaves_conflicted_file_alone() {
    for (name, _) in CORPUS {
        let text = corpus(name);
        let file = SourceMap::new().add(name, &*text).unwrap();
        match format_file(file, &text) {
            Err(FormatError::Syntax(diagnostics)) => {
                assert!(
                    diagnostics.iter().any(|d| d.code == Code::Conflict),
                    "{name}"
                );
            }
            other => panic!("{name}: the formatter must leave it alone, got {other:?}"),
        }
    }
}

#[test]
fn each_side_parses_cleanly() {
    for (name, _) in CORPUS {
        let text = corpus(name);
        for side in 0..2 {
            let chosen = side_text(&text, side);
            assert!(find_conflicts(&chosen).is_empty());
            let parsed = parse(name, &chosen);
            assert!(
                parsed.diagnostics.is_empty(),
                "{name}, side {side}: {:?}",
                parsed.diagnostics
            );
            let file = SourceMap::new().add(name, &*chosen).unwrap();
            assert!(format_file(file, &chosen).is_ok(), "{name}, side {side}");
        }
    }
}
