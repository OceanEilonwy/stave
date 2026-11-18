//! The broken-design corpus (`tests/broken/`): every folder is a small
//! project whose first line is `// expect: error[<code>] <message>`.
//!
//! This harness checks layer 1, syntax, with the parser alone: the parser
//! must produce exactly the expected diagnostic and nothing else, and a
//! project that expects a later layer's code must parse without any
//! diagnostic. The same corpus runs through the whole pipeline (layers 1 to
//! 3) in `crates/stave-app/tests/pipeline.rs`.

use std::path::{Path, PathBuf};

use stave_lang::{Code, Diagnostic, FileId, Severity, SourceMap, parse_file, render_diagnostics};

/// A checking stage: the layers whose codes it reports, and how to run it
/// on one file.
struct Stage {
    layers: std::ops::RangeInclusive<u8>,
    run: fn(FileId, &str) -> Vec<Diagnostic>,
}

/// The stages that exist so far, in pipeline order.
const STAGES: &[Stage] = &[Stage {
    layers: 1..=1,
    run: |file, text| parse_file(file, text).diagnostics,
}];

struct Expectation {
    severity: Severity,
    code: Code,
    message: String,
    /// `// pending: <reason>`: the check isn't built yet, so the design
    /// must check clean until it is.
    pending: Option<String>,
}

/// Layer 1 codes a later layer reports too: a design that expects one of
/// them and parses clean is for the later layer (`missing-value` for a
/// `locked` param with no default is the resolver's).
const ALSO_LATER: &[Code] = &[Code::MissingValue];

/// `true` for a design whose layer 1 code a later layer reports.
fn for_a_later_layer(expected: &Expectation, text: &str) -> bool {
    let mut sources = SourceMap::new();
    let Ok(file) = sources.add("project.stave", text) else {
        return false;
    };
    ALSO_LATER.contains(&expected.code) && parse_file(file, text).diagnostics.is_empty()
}

/// Reads the `// expect:` line (and the optional `// pending:` line) of a
/// broken design.
fn expectation(path: &Path, text: &str) -> Expectation {
    let mut lines = text.lines();
    let first = lines.next().unwrap_or("");
    let (severity, rest) = if let Some(rest) = first.strip_prefix("// expect: error[") {
        (Severity::Error, rest)
    } else if let Some(rest) = first.strip_prefix("// expect: warning[") {
        (Severity::Warning, rest)
    } else {
        panic!("{}: bad expect line", path.display())
    };
    let (code, message) = rest.split_once("] ").unwrap();
    let code = Code::from_str_code(code).unwrap_or_else(|| panic!("unknown code {code}"));
    let pending = lines
        .next()
        .and_then(|line| line.strip_prefix("// pending: "))
        .map(str::to_owned);
    Expectation {
        severity,
        code,
        message: message.to_owned(),
        pending,
    }
}

fn corpus() -> Vec<(PathBuf, String, Expectation)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/broken");
    let mut folders: Vec<PathBuf> = std::fs::read_dir(&root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_dir())
        .collect();
    folders.sort();
    folders
        .into_iter()
        .map(|folder| {
            let path = folder.join("project.stave");
            let text = std::fs::read_to_string(&path).unwrap();
            let expected = expectation(&path, &text);
            let name = format!(
                "tests/broken/{}/project.stave",
                folder.file_name().unwrap().to_string_lossy()
            );
            (PathBuf::from(name), text, expected)
        })
        .collect()
}

#[test]
fn broken_designs_produce_their_expected_diagnostic() {
    let implemented = STAGES
        .iter()
        .map(|stage| *stage.layers.end())
        .max()
        .unwrap_or(0);
    let mut checked = 0;
    for (path, text, expected) in corpus() {
        let mut sources = SourceMap::new();
        let file = sources.add(path.to_string_lossy(), text.as_str()).unwrap();
        let mut diagnostics = Vec::new();
        for stage in STAGES {
            diagnostics.extend((stage.run)(file, &text));
            if stage.layers.contains(&expected.code.layer()) || !diagnostics.is_empty() {
                break;
            }
        }
        let rendered = render_diagnostics(&sources, &diagnostics);
        if for_a_later_layer(&expected, &text) {
            continue;
        }
        if expected.code.layer() <= implemented && expected.pending.is_none() {
            checked += 1;
            let [only] = diagnostics.as_slice() else {
                panic!(
                    "{}: expected exactly one diagnostic, got:\n{rendered}",
                    path.display()
                );
            };
            assert_eq!(only.code, expected.code, "{}:\n{rendered}", path.display());
            assert_eq!(
                only.severity,
                expected.severity,
                "{}:\n{rendered}",
                path.display()
            );
            assert_eq!(
                only.message,
                expected.message,
                "{}:\n{rendered}",
                path.display()
            );
        } else {
            assert!(
                diagnostics.is_empty(),
                "{} expects a layer {} error, but earlier layers report:\n{rendered}",
                path.display(),
                expected.code.layer()
            );
        }
    }
    assert!(
        checked >= 6,
        "every syntax-level code needs a broken design"
    );
}

#[test]
fn the_rest_of_a_broken_file_still_parses() {
    let mut sources = SourceMap::new();
    for (path, text, expected) in corpus() {
        if expected.code.layer() != 1
            || expected.pending.is_some()
            || for_a_later_layer(&expected, &text)
        {
            continue;
        }
        let file = sources.add(path.to_string_lossy(), text.as_str()).unwrap();
        let parse = parse_file(file, &text);
        let ast = parse.ast.expect("a header");
        assert_eq!(
            parse.broken.len(),
            1,
            "{}: {:?}",
            path.display(),
            parse.broken
        );
        // The project and the stock come first in every broken design and
        // are always intact.
        assert!(ast.items.len() >= 2, "{}", path.display());
    }
}
