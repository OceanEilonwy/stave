//! Every `.stave` file in `examples/` and `std/` parses cleanly, the tree is
//! lossless, and the formatter is idempotent and keeps the meaning.

use std::path::{Path, PathBuf};

use stave_lang::{SourceMap, format_file, parse_file, render_diagnostics};

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn collect(dir: &Path, files: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("{}: {error}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect(&path, files);
        } else if path
            .extension()
            .is_some_and(|extension| extension == "stave")
        {
            files.push(path);
        }
    }
}

fn corpus() -> Vec<(String, String)> {
    let root = workspace();
    let mut files = Vec::new();
    collect(&root.join("examples"), &mut files);
    collect(&root.join("std"), &mut files);
    assert!(files.len() >= 30, "the corpus went missing");
    files
        .into_iter()
        .map(|path| {
            let text = std::fs::read_to_string(&path).unwrap();
            let name = path
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            (name, text)
        })
        .collect()
}

/// The `Debug` text of a tree with every span blanked, so trees parsed from
/// differently laid out text can be compared.
pub fn without_spans(debug: &str) -> String {
    let mut out = String::with_capacity(debug.len());
    let mut rest = debug;
    while let Some(start) = rest.find("Span { start: ") {
        out.push_str(&rest[..start]);
        let after = &rest[start..];
        let end = after.find('}').expect("end of span") + 1;
        out.push_str("Span");
        rest = &after[end..];
    }
    out.push_str(rest);
    out
}

#[test]
fn every_corpus_file_parses_without_diagnostics() {
    for (name, text) in corpus() {
        let mut sources = SourceMap::new();
        let file = sources.add(&name, text.as_str()).unwrap();
        let parse = parse_file(file, &text);
        assert!(
            parse.diagnostics.is_empty(),
            "{name}:\n{}",
            render_diagnostics(&sources, &parse.diagnostics)
        );
        assert!(parse.broken.is_empty(), "{name}: {:?}", parse.broken);
        let ast = parse.ast.unwrap_or_else(|| panic!("{name}: no typed tree"));
        assert_eq!(ast.header.version, 1, "{name}");
    }
}

#[test]
fn the_tree_is_lossless() {
    for (name, text) in corpus() {
        let mut sources = SourceMap::new();
        let file = sources.add(&name, text.as_str()).unwrap();
        assert_eq!(parse_file(file, &text).syntax().to_string(), text, "{name}");
    }
}

#[test]
fn formatting_is_idempotent_and_keeps_the_meaning() {
    for (name, text) in corpus() {
        let mut sources = SourceMap::new();
        let file = sources.add(&name, text.as_str()).unwrap();
        let formatted = format_file(file, &text).unwrap_or_else(|error| panic!("{name}: {error}"));
        let again = format_file(file, &formatted).unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(again, formatted, "{name}: formatting twice changed it");

        let before = parse_file(file, &text);
        let after = parse_file(file, &formatted);
        assert!(
            after.diagnostics.is_empty(),
            "{name}: {:?}",
            after.diagnostics
        );
        assert_eq!(
            without_spans(&format!("{:?}", after.ast)),
            without_spans(&format!("{:?}", before.ast)),
            "{name}: formatting changed the meaning"
        );
    }
}
