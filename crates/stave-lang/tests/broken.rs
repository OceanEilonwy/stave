//! The broken-design corpus (`tests/broken/`): every folder is a small
//! project whose first line is `// expect: error[<code>] <message>`.
//!
//! The harness runs every stage that exists so far over each project and
//! checks the expectation for the codes those stages own. Today that is
//! layer 1, syntax: the parser must produce exactly the expected diagnostic
//! and nothing else. A project that expects a later layer's code must parse
//! without any diagnostic. When a later stage lands, add it to [`STAGES`]
//! (or move this harness to the crate that runs the whole pipeline).

use std::path::{Path, PathBuf};

use stave_lang::{Code, Diagnostic, FileId, SourceMap, parse_file, render_diagnostics};

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
    code: Code,
    message: String,
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
            let first = text.lines().next().unwrap_or("");
            let rest = first
                .strip_prefix("// expect: error[")
                .unwrap_or_else(|| panic!("{}: bad expect line", path.display()));
            let (code, message) = rest.split_once("] ").unwrap();
            let code = Code::from_str_code(code).unwrap_or_else(|| panic!("unknown code {code}"));
            let name = format!(
                "tests/broken/{}/project.stave",
                folder.file_name().unwrap().to_string_lossy()
            );
            (
                PathBuf::from(name),
                text.clone(),
                Expectation {
                    code,
                    message: message.to_owned(),
                },
            )
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
        if expected.code.layer() <= implemented {
            checked += 1;
            let [only] = diagnostics.as_slice() else {
                panic!(
                    "{}: expected exactly one diagnostic, got:\n{rendered}",
                    path.display()
                );
            };
            assert_eq!(only.code, expected.code, "{}:\n{rendered}", path.display());
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
        if expected.code.layer() != 1 {
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
