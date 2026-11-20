//! Outlines, folds, help-line fixes and worked-out values on the examples
//! and on half-typed text.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use stave_ide::{
    Analysis, Document, FilePosition, FoldKind, OutlineEntry, OutlineKind, Overlay, ValueKind,
    ValueNote, WorkedValues,
};
use stave_lang::ast::Span;
use stave_lang::placement::PartAxis;

fn example(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(name)
        .canonicalize()
        .unwrap()
}

fn print(entries: &[OutlineEntry], depth: usize, out: &mut String) {
    for entry in entries {
        let kind = match entry.kind {
            OutlineKind::Header => "header".to_owned(),
            OutlineKind::Item(kind) => kind.word().to_owned(),
            OutlineKind::Block => "block".to_owned(),
        };
        writeln!(out, "{}{kind} {}", "  ".repeat(depth), entry.name).unwrap();
        print(&entry.children, depth + 1, out);
    }
}

#[test]
fn the_outline_lists_items_and_the_blocks_inside_them() {
    let folder = example("clothing_cabinet");
    let analysis = Analysis::load(&folder, &Overlay::new()).unwrap();
    let mut shown = String::new();
    for name in ["cabinet.stave", "carcass.stave", "doors.stave"] {
        let file = analysis.file(&Document::File(folder.join(name))).unwrap();
        writeln!(shown, "# {name}").unwrap();
        print(&analysis.outline(file), 0, &mut shown);
    }
    insta::assert_snapshot!(shown);
}

/// An analysis of one file, `box.stave`, in a fresh folder.
fn one_file(text: &str) -> (tempfile_folder::Folder, Analysis, stave_lang::FileId) {
    let folder = tempfile_folder::Folder::new(text);
    let analysis = Analysis::load(folder.path(), &Overlay::new()).unwrap();
    let file = analysis
        .file(&Document::File(folder.path().join("box.stave")))
        .unwrap();
    (folder, analysis, file)
}

/// A folder under the target directory holding one file, removed when
/// dropped.
mod tempfile_folder {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNT: AtomicUsize = AtomicUsize::new(0);

    pub struct Folder(PathBuf);

    impl Folder {
        pub fn new(text: &str) -> Folder {
            let number = COUNT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("stave-ide-outline-{}-{number}", std::process::id()));
            std::fs::create_dir_all(&path).unwrap();
            std::fs::write(path.join("box.stave"), text).unwrap();
            Folder(path.canonicalize().unwrap())
        }

        pub fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

const HALF_TYPED: &str = "stave 1

// A box.
// Two lines of comment.
project \"Box\" {
  units: mm
}

import \"std:joinery\"
import \"std:hardware\"

part side : pine_19 {
  length: 400mm
  place {
    ground
  }
}

part shelf : pine_19 {
  length: fit
  place {
    against(side.top
";

#[test]
fn half_typed_text_still_has_an_outline_and_folds() {
    let (_folder, analysis, file) = one_file(HALF_TYPED);
    let mut shown = String::new();
    print(&analysis.outline(file), 0, &mut shown);
    assert_eq!(
        shown,
        "header project \"Box\"\npart side\n  block place\npart shelf\n  block place\n"
    );
    let folds = analysis.folds(file);
    let text = analysis.text(file).unwrap();
    let folded: Vec<(FoldKind, &str)> = folds
        .iter()
        .map(|fold| {
            let span = fold.span;
            (fold.kind, &text[span.start() as usize..span.end() as usize])
        })
        .collect();
    assert!(folded.contains(&(FoldKind::Comment, "// A box.\n// Two lines of comment.")));
    assert!(folded.contains(&(
        FoldKind::Imports,
        "import \"std:joinery\"\nimport \"std:hardware\""
    )));
    assert!(folded.contains(&(FoldKind::Block, "{\n    ground\n  }")));
    // The unclosed blocks of `shelf` fold nothing.
    assert!(!folded.iter().any(|(_, text)| text.contains("fit")));
}

#[test]
fn a_blank_line_or_code_ends_a_run_of_comments() {
    let text = "stave 1\n\n// One.\n// Two.\n\n// Three.\n// Four.\nproject \"Box\" {\n  units: mm // mm\n  // inside\n}\n";
    let (_folder, analysis, file) = one_file(text);
    let text = analysis.text(file).unwrap();
    let comments: Vec<&str> = analysis
        .folds(file)
        .iter()
        .filter(|fold| fold.kind == FoldKind::Comment)
        .map(|fold| &text[fold.span.start() as usize..fold.span.end() as usize])
        .collect();
    // The comment after `units: mm` belongs to its line, so it starts no
    // run with the comment below it.
    assert_eq!(comments, ["// One.\n// Two.", "// Three.\n// Four."]);
}

#[test]
fn a_help_line_that_names_the_text_is_a_fix() {
    let text = "stave 1\n\nproject \"Box\" {\n  units: mm\n}\n\nlet width = mid(base, up\n";
    let (_folder, analysis, file) = one_file(text);
    let fixes = analysis.help_fixes(file);
    let fix = fixes
        .first()
        .unwrap_or_else(|| panic!("{:?}", analysis.diagnostics(file).collect::<Vec<_>>()));
    let mut fixed = text.to_owned();
    fixed.replace_range(
        fix.span.start() as usize..fix.span.end() as usize,
        &fix.text,
    );
    assert!(fixed.contains("mid(base, up)\n"), "{fixed}");
}

#[test]
fn values_are_shown_only_where_their_text_is_unchanged() {
    let folder = example("clothing_cabinet");
    let path = folder.join("carcass.stave");
    let analysis = Analysis::load(&folder, &Overlay::new()).unwrap();
    let document = Document::File(path.clone());
    let file = analysis.file(&document).unwrap();
    let text = analysis.text(file).unwrap().to_owned();
    let start = text.find("length: fit").unwrap() + "length: ".len();
    let span = Span::new(start as u32, (start + 3) as u32);
    let literal = text.find("19mm").unwrap();
    let values = WorkedValues::new([
        ValueNote {
            document: document.clone(),
            span,
            covers: "fit".to_owned(),
            name: None,
            kind: ValueKind::Size(PartAxis::Length),
            literal: false,
            text: "844mm, cut 862mm".to_owned(),
        },
        ValueNote {
            document: document.clone(),
            span: Span::new(literal as u32, (literal + 4) as u32),
            covers: "19mm".to_owned(),
            name: None,
            kind: ValueKind::Size(PartAxis::Thickness),
            literal: true,
            text: "19mm".to_owned(),
        },
    ]);
    let hints = values.inlay_hints(&analysis, file);
    assert_eq!(hints.len(), 1, "a literal is not repeated: {hints:?}");
    assert_eq!(hints[0].offset, span.end());
    assert_eq!(hints[0].label, "= 844mm, cut 862mm");
    let hover = analysis
        .hover_with_values(
            FilePosition {
                file,
                offset: span.start() + 1,
            },
            &values,
        )
        .unwrap();
    assert_eq!(hover.value.as_deref(), Some("= 844mm, cut 862mm"));
    assert_eq!(hover.kind, "length");

    // A line typed above moves the text: the value is not shown until
    // the next check works it out again.
    let mut overlay = Overlay::new();
    overlay.insert(&path, format!("// a new first line\n{text}"));
    let changed = Analysis::load(&folder, &overlay).unwrap();
    let file = changed.file(&document).unwrap();
    assert_eq!(values.inlay_hints(&changed, file), Vec::new());
}

#[test]
fn a_name_in_an_expression_shows_its_own_value_not_the_expressions() {
    let text = "stave 1\n\nproject \"Box\" {\n  units: mm\n}\n\nparam width = 600mm\nlet half = width / 2\n";
    let (folder, analysis, file) = one_file(text);
    let document = Document::File(folder.path().join("box.stave"));
    let span_of = |needle: &str, from: usize| {
        let start = text[from..].find(needle).unwrap() + from;
        Span::new(start as u32, (start + needle.len()) as u32)
    };
    let let_line = text.find("let half").unwrap();
    let note = |span: Span, name: Span, kind: ValueKind, covers: &str, value: &str| ValueNote {
        document: document.clone(),
        span,
        covers: covers.to_owned(),
        name: Some((
            name,
            text[name.start() as usize..name.end() as usize].to_owned(),
        )),
        kind,
        literal: false,
        text: value.to_owned(),
    };
    let values = WorkedValues::new([
        note(
            span_of("600mm", 0),
            span_of("width", 0),
            ValueKind::Param,
            "600mm",
            "600mm",
        ),
        note(
            span_of("width / 2", let_line),
            span_of("half", let_line),
            ValueKind::Let,
            "width / 2",
            "300mm",
        ),
    ]);
    let hover_at = |offset: u32| {
        analysis
            .hover_with_values(FilePosition { file, offset }, &values)
            .and_then(|hover| hover.value)
    };
    // `width` inside `half`'s value is the param, worth 600mm.
    let reference = span_of("width", let_line);
    assert_eq!(hover_at(reference.start() + 1).as_deref(), Some("= 600mm"));
    // `half` itself, and the `/` between, are the `let`, worth 300mm.
    assert_eq!(
        hover_at(span_of("half", let_line).start() + 1).as_deref(),
        Some("= 300mm")
    );
    assert_eq!(
        hover_at(span_of("/", let_line).start()).as_deref(),
        Some("= 300mm")
    );
}
