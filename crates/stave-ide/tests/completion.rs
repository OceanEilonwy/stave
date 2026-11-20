//! Completion and signature help, driven by fixtures.
//!
//! Each `tests/completion/*.stave` file is a project with a cursor marker
//! `$0` and, in comments at its end, the exact list of words completion
//! offers there:
//!
//! ```text
//! // expect: ground, against, flush
//! ```
//!
//! `// expect: -` means nothing is offered. Several `// expect:` lines
//! join into one list. A fixture may hold several files, each after a
//! line `//- path/name.stave`; the marker is in one of them.
//!
//! Each `tests/signature/*.stave` file has a marker and the signature help
//! expected there: `// label: ...` and `// active: <name>` (or `-`).

use std::path::{Path, PathBuf};

use stave_ide::{Analysis, CompletionKind, Document, FilePosition, Overlay};

const MARKER: &str = "$0";

struct Fixture {
    analysis: Analysis,
    position: FilePosition,
    /// Every `// key: value` comment line, in order.
    expectations: Vec<(String, String)>,
}

fn fixtures(folder: &str) -> Vec<PathBuf> {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join(folder);
    let mut files: Vec<PathBuf> = std::fs::read_dir(&folder)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|e| e == "stave"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no fixtures in {}", folder.display());
    files
}

/// Loads a fixture as a project in a scratch folder of its own, its text
/// given as unsaved buffers.
fn load(path: &Path) -> Fixture {
    let source = std::fs::read_to_string(path).unwrap();
    let name = path.file_stem().unwrap().to_string_lossy().into_owned();
    let folder = std::env::temp_dir().join(format!(
        "stave-ide-fixture-{}-{}-{name}",
        path.parent()
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy(),
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    let folder = folder.canonicalize().unwrap();

    let mut expectations = Vec::new();
    let mut files: Vec<(String, String)> = vec![("main.stave".to_owned(), String::new())];
    for line in source.lines() {
        if let Some(file) = line.strip_prefix("//- ") {
            files.push((file.trim().to_owned(), String::new()));
            continue;
        }
        if let Some(rest) = line.strip_prefix("// ")
            && let Some((key, value)) = rest.split_once(": ")
            && matches!(key, "expect" | "label" | "active")
        {
            expectations.push((key.to_owned(), value.trim().to_owned()));
            continue;
        }
        let text = &mut files.last_mut().unwrap().1;
        text.push_str(line);
        text.push('\n');
    }
    // A fixture that starts with a file line has no text of its own.
    files.retain(|(name, text)| name != "main.stave" || !text.trim().is_empty());

    let mut overlay = Overlay::new();
    let mut marked: Option<(PathBuf, u32)> = None;
    for (name, text) in &files {
        let file = folder.join(name);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let text = match text.find(MARKER) {
            Some(at) => {
                marked = Some((file.clone(), u32::try_from(at).unwrap()));
                text.replacen(MARKER, "", 1)
            }
            None => text.clone(),
        };
        overlay.insert(&file, text);
    }
    let (marked, offset) = marked.unwrap_or_else(|| panic!("{name} has no {MARKER}"));
    let analysis = Analysis::load(&folder, &overlay).unwrap();
    let file = analysis
        .file(&Document::File(marked.clone()))
        .unwrap_or_else(|| panic!("{} isn't loaded", marked.display()));
    Fixture {
        analysis,
        position: FilePosition { file, offset },
        expectations,
    }
}

fn expected(fixture: &Fixture, key: &str) -> Option<String> {
    let values: Vec<&str> = fixture
        .expectations
        .iter()
        .filter(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
        .collect();
    (!values.is_empty()).then(|| values.join(", "))
}

#[test]
fn completion_fixtures() {
    let mut failures = Vec::new();
    for path in fixtures("completion") {
        let fixture = load(&path);
        let Some(expect) = expected(&fixture, "expect") else {
            panic!("{} has no // expect:", path.display());
        };
        let actual: Vec<String> = fixture
            .analysis
            .completions(fixture.position)
            .into_iter()
            .map(|completion| completion.label)
            .collect();
        let actual = if actual.is_empty() {
            "-".to_owned()
        } else {
            actual.join(", ")
        };
        if actual != expect {
            failures.push(format!(
                "{}\n  expected: {expect}\n  actual:   {actual}",
                path.file_name().unwrap().to_string_lossy()
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn signature_fixtures() {
    let mut failures = Vec::new();
    for path in fixtures("signature") {
        let fixture = load(&path);
        let help = fixture.analysis.signature_help(fixture.position);
        let label = help.as_ref().map_or("-".to_owned(), |h| h.label.clone());
        let active = help
            .as_ref()
            .and_then(|h| h.active.and_then(|a| h.parameters.get(a)))
            .map_or("-".to_owned(), |p| p.name.clone());
        let expect_label = expected(&fixture, "label").unwrap_or_else(|| "-".to_owned());
        let expect_active = expected(&fixture, "active").unwrap_or_else(|| "-".to_owned());
        if label != expect_label || active != expect_active {
            failures.push(format!(
                "{}\n  expected: {expect_label} [{expect_active}]\n  actual:   {label} [{active}]",
                path.file_name().unwrap().to_string_lossy()
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn completions_carry_kinds_details_and_spans() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/completion/place_prefix.stave");
    let fixture = load(&path);
    let completions = fixture.analysis.completions(fixture.position);
    let [flush] = completions.as_slice() else {
        panic!("{completions:?}");
    };
    assert_eq!(flush.kind, CompletionKind::PlacementTerm);
    assert_eq!(flush.detail, "flush(face, inset:)");
    // The typed `fl` is replaced.
    assert_eq!(flush.replace.end() - flush.replace.start(), 2);
    assert_eq!(flush.replace.end(), fixture.position.offset);

    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/completion/module_items.stave");
    let fixture = load(&path);
    let completions = fixture.analysis.completions(fixture.position);
    let knob = completions
        .iter()
        .find(|c| c.label == "brass_knob")
        .unwrap();
    assert_eq!(knob.kind, CompletionKind::Item(stave_ide::SymbolKind::Kind));
    assert_eq!(knob.detail, "kind brass_knob : pull");
    assert_eq!(knob.docs.as_deref(), Some("A small round brass knob."));

    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/completion/units.stave");
    let fixture = load(&path);
    let completions = fixture.analysis.completions(fixture.position);
    assert!(completions.iter().all(|c| c.kind == CompletionKind::Unit));
    assert_eq!(
        completions.first().map(|c| c.detail.as_str()),
        Some("millimetres")
    );
}

#[test]
fn signature_help_gives_types_and_defaults() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/signature/joint_positional.stave");
    let fixture = load(&path);
    let help = fixture.analysis.signature_help(fixture.position).unwrap();
    assert_eq!(help.callee, "housing");
    assert_eq!(help.docs.as_deref(), Some("A housing for a shelf."));
    let depth = help.parameters.iter().find(|p| p.name == "depth").unwrap();
    assert_eq!(depth.ty.as_deref(), Some("Length"));
    assert_eq!(depth.default.as_deref(), Some("into.thickness / 3"));
    assert_eq!(depth.docs.as_deref(), Some("How deep the housing is."));
    assert!(!depth.required);
    let piece = help.parameters.first().unwrap();
    assert_eq!(piece.kind, stave_ide::ParameterKind::Role);
    assert_eq!(piece.ty.as_deref(), Some("Part"));
}

#[test]
fn completion_on_every_offset_of_the_examples_does_not_panic() {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/clothing_cabinet")
        .canonicalize()
        .unwrap();
    let analysis = Analysis::load(&folder, &Overlay::new()).unwrap();
    for (file, _) in analysis.project_files() {
        let length = u32::try_from(analysis.text(file).unwrap().len()).unwrap();
        for offset in 0..=length {
            let position = FilePosition { file, offset };
            let _ = analysis.completions(position);
            let _ = analysis.signature_help(position);
        }
    }
}
