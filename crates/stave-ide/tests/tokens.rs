//! Semantic tokens: snapshots of every example's colouring (classes,
//! unit spans and modifiers), and the rules every token list keeps.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use stave_ide::{
    Analysis, Document, SemanticToken, SymbolKind, TokenClass, TokenModifier, TokenModifiers,
};
use stave_lang::ast::Unit;

fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .canonicalize()
        .unwrap()
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
        .canonicalize()
        .unwrap()
}

fn load(folder: &Path) -> Analysis {
    Analysis::load(folder, &stave_ide::Overlay::new()).unwrap()
}

/// The name a snapshot gives a class.
fn class_name(class: TokenClass) -> String {
    match class {
        TokenClass::Keyword => "keyword".to_owned(),
        TokenClass::Comment => "comment".to_owned(),
        TokenClass::DocComment => "doc".to_owned(),
        TokenClass::Text => "text".to_owned(),
        TokenClass::Number => "number".to_owned(),
        TokenClass::Unit => "unit".to_owned(),
        TokenClass::Function => "function".to_owned(),
        TokenClass::Type => "type".to_owned(),
        TokenClass::Module => "module".to_owned(),
        TokenClass::EnumWord => "word".to_owned(),
        TokenClass::Field => "field".to_owned(),
        TokenClass::Item(kind) => kind.word().replace(' ', "_"),
        TokenClass::Punctuation => "punctuation".to_owned(),
        TokenClass::Error => "error".to_owned(),
        TokenClass::Plain => "plain".to_owned(),
    }
}

/// The short name a snapshot gives a modifier.
fn modifier_name(modifier: TokenModifier) -> &'static str {
    match modifier {
        TokenModifier::Declaration => "decl",
        TokenModifier::Reference => "ref",
        TokenModifier::StandardLibrary => "std",
        TokenModifier::Unresolved => "unresolved",
        TokenModifier::SolvedParam => "solved",
    }
}

/// One file's tokens, line by line: each source line that has any
/// tokens but punctuation and comments, then its tokens as
/// `text:class+modifier`.
fn render_file(text: &str, tokens: &[SemanticToken]) -> String {
    let mut out = String::new();
    let mut line_start = 0usize;
    let mut tokens = tokens.iter().peekable();
    for (number, line) in text.split('\n').enumerate() {
        let line_end = line_start + line.len();
        let mut described = Vec::new();
        while let Some(token) = tokens.next_if(|t| (t.span.start() as usize) <= line_end) {
            if matches!(
                token.class,
                TokenClass::Punctuation | TokenClass::Comment | TokenClass::DocComment
            ) {
                continue;
            }
            let piece = &text[token.span.start() as usize..token.span.end() as usize];
            let mut entry = format!("{piece}:{}", class_name(token.class));
            for modifier in token.modifiers.iter() {
                entry.push('+');
                entry.push_str(modifier_name(modifier));
            }
            described.push(entry);
        }
        if !described.is_empty() {
            writeln!(out, "{:>4} | {}", number + 1, line.trim_end()).unwrap();
            writeln!(out, "     = {}", described.join("  ")).unwrap();
        }
        line_start = line_end + 1;
    }
    out
}

/// Every project file of a folder's analysis, rendered, with its path.
fn render_project(folder: &Path) -> String {
    let analysis = load(folder);
    let mut out = String::new();
    for (file, document) in analysis.project_files() {
        let Document::File(path) = &document else {
            continue;
        };
        let label = path.strip_prefix(folder).unwrap_or(path);
        let text = analysis.text(file).unwrap();
        writeln!(out, "== {} ==", label.display()).unwrap();
        out.push_str(&render_file(text, &analysis.tokens(file)));
    }
    out
}

/// The rules every token list keeps: in order, not overlapping, inside
/// the text, a quantity always split into its number and a real unit.
fn check_rules(folder: &Path) {
    let analysis = load(folder);
    for (file, document) in analysis.project_files() {
        let text = analysis.text(file).unwrap();
        let tokens = analysis.tokens(file);
        let mut end = 0;
        for (index, token) in tokens.iter().enumerate() {
            assert!(
                token.span.start() >= end && token.span.start() < token.span.end(),
                "{document:?}: token {index} at {:?} overlaps or is empty",
                token.span
            );
            end = token.span.end();
            let piece = &text[token.span.start() as usize..token.span.end() as usize];
            #[expect(
                clippy::wildcard_enum_match_arm,
                reason = "only units and numbers have a rule here"
            )]
            match token.class {
                TokenClass::Unit => {
                    assert!(Unit::from_suffix(piece).is_some(), "'{piece}' is no unit");
                    let number = tokens[index - 1];
                    assert_eq!(number.class, TokenClass::Number);
                    assert_eq!(number.span.end(), token.span.start());
                }
                TokenClass::Number => assert!(
                    !piece.chars().any(|c| c.is_ascii_alphabetic()),
                    "number '{piece}' keeps its unit"
                ),
                _ => {}
            }
            let named = matches!(
                token.class,
                TokenClass::Item(_) | TokenClass::Module | TokenClass::Type
            );
            let declared = token.modifiers.contains(TokenModifier::Declaration);
            let referenced = token.modifiers.contains(TokenModifier::Reference);
            assert!(
                !(declared && referenced),
                "'{piece}' is both declared and named"
            );
            if declared || referenced {
                assert!(
                    named,
                    "'{piece}' names an item but is coloured {:?}",
                    token.class
                );
            }
            if token.modifiers.contains(TokenModifier::Unresolved) {
                assert!(!declared && !referenced, "'{piece}' resolves and doesn't");
            }
        }
        assert!(end as usize <= text.len());
    }
}

#[test]
fn every_example_keeps_the_token_rules() {
    let mut count = 0;
    for entry in std::fs::read_dir(examples()).unwrap() {
        let folder = entry.unwrap().path();
        if folder.is_dir() {
            check_rules(&folder);
            // Every example checks, so no name in it is unresolved: a word
            // of the language marked so is missing from `stave_ide::words`.
            let analysis = load(&folder);
            for (file, document) in analysis.project_files() {
                let text = analysis.text(file).unwrap();
                for token in analysis.tokens(file) {
                    assert!(
                        !token.modifiers.contains(TokenModifier::Unresolved),
                        "{document:?}: '{}' is marked unresolved",
                        &text[token.span.start() as usize..token.span.end() as usize]
                    );
                }
            }
            count += 1;
        }
    }
    assert!(count >= 10, "only {count} examples");
    check_rules(&fixture("colouring"));
}

#[test]
fn modifiers_are_a_set() {
    let set: TokenModifiers = [TokenModifier::SolvedParam, TokenModifier::Reference]
        .into_iter()
        .collect();
    assert!(set.contains(TokenModifier::Reference));
    assert!(!set.contains(TokenModifier::Declaration));
    assert_eq!(
        set.iter().collect::<Vec<_>>(),
        [TokenModifier::Reference, TokenModifier::SolvedParam]
    );
    assert!(TokenModifiers::NONE.is_empty());
    assert_eq!(set.with(TokenModifier::Reference), set);
    let words: Vec<&str> = TokenModifier::ALL.iter().map(|m| m.word()).collect();
    assert_eq!(
        words,
        [
            "declaration",
            "reference",
            "standard_library",
            "unresolved",
            "solved_param"
        ]
    );
}

/// The tokens but punctuation of the first occurrence of `needle` in the
/// fixture.
fn tokens_of(needle: &str) -> Vec<(String, TokenClass, TokenModifiers)> {
    let folder = fixture("colouring");
    let analysis = load(&folder);
    let file = analysis
        .file(&Document::File(folder.join("project.stave")))
        .unwrap();
    let text = analysis.text(file).unwrap();
    let start = u32::try_from(text.find(needle).unwrap()).unwrap();
    let end = start + u32::try_from(needle.len()).unwrap();
    analysis
        .tokens(file)
        .into_iter()
        .filter(|t| {
            t.span.start() >= start && t.span.end() <= end && t.class != TokenClass::Punctuation
        })
        .map(|t| {
            (
                text[t.span.start() as usize..t.span.end() as usize].to_owned(),
                t.class,
                t.modifiers,
            )
        })
        .collect()
}

fn modifiers(list: &[TokenModifier]) -> TokenModifiers {
    list.iter().copied().collect()
}

#[test]
fn quantities_split_into_number_and_unit() {
    let none = TokenModifiers::NONE;
    assert_eq!(
        tokens_of("3/4in"),
        [
            ("3/4".to_owned(), TokenClass::Number, none),
            ("in".to_owned(), TokenClass::Unit, none)
        ]
    );
    assert_eq!(
        tokens_of("1-1/2in"),
        [
            ("1-1/2".to_owned(), TokenClass::Number, none),
            ("in".to_owned(), TokenClass::Unit, none)
        ]
    );
    assert_eq!(
        tokens_of("18.5mm"),
        [
            ("18.5".to_owned(), TokenClass::Number, none),
            ("mm".to_owned(), TokenClass::Unit, none)
        ]
    );
    // A plain number stays one token.
    assert_eq!(
        tokens_of("/ 2"),
        [("2".to_owned(), TokenClass::Number, none)]
    );
}

#[test]
fn each_modifier_where_it_belongs() {
    use TokenModifier::{Declaration, Reference, SolvedParam, StandardLibrary, Unresolved};
    let param = TokenClass::Item(SymbolKind::Param);
    assert_eq!(
        tokens_of("solved param overall_width")[2],
        (
            "overall_width".to_owned(),
            param,
            modifiers(&[Declaration, SolvedParam])
        )
    );
    assert_eq!(
        tokens_of("length: overall_width")[1],
        (
            "overall_width".to_owned(),
            param,
            modifiers(&[Reference, SolvedParam])
        )
    );
    assert_eq!(
        tokens_of("overall_depth + missing_name")[0],
        ("overall_depth".to_owned(), param, modifiers(&[Reference]))
    );
    assert_eq!(
        tokens_of("missing_name")[0],
        (
            "missing_name".to_owned(),
            TokenClass::Plain,
            modifiers(&[Unresolved])
        )
    );
    assert_eq!(
        tokens_of("dado")[0],
        (
            "dado".to_owned(),
            TokenClass::Type,
            modifiers(&[Reference, StandardLibrary])
        )
    );
    assert_eq!(
        tokens_of("part side_l")[1],
        (
            "side_l".to_owned(),
            TokenClass::Item(SymbolKind::Part),
            modifiers(&[Declaration])
        )
    );
    // Enum words and built-in calls are words of the language, not
    // unresolved names.
    assert_eq!(
        tokens_of("white_oak")[0],
        (
            "white_oak".to_owned(),
            TokenClass::EnumWord,
            TokenModifiers::NONE
        )
    );
}

#[test]
fn fixture_tokens() {
    insta::assert_snapshot!(render_project(&fixture("colouring")));
}

macro_rules! example_snapshots {
    ($($test:ident => $folder:literal),* $(,)?) => {
        $(
            #[test]
            fn $test() {
                insta::assert_snapshot!(render_project(&examples().join($folder)));
            }
        )*
    };
}

example_snapshots! {
    adjustable_bookcase_tokens => "adjustable_bookcase",
    bookshelf_tokens => "bookshelf",
    box_tokens => "box",
    chest_of_drawers_tokens => "chest_of_drawers",
    clothing_cabinet_tokens => "clothing_cabinet",
    clothing_cabinet_assemblies_tokens => "clothing_cabinet_assemblies",
    clothing_cabinet_from_box_tokens => "clothing_cabinet_from_box",
    face_frame_tokens => "face_frame",
    japanese_joinery_tokens => "japanese_joinery",
    joint_samples_tokens => "joint_samples",
    nightstand_tokens => "nightstand",
    picture_frame_tokens => "picture_frame",
    side_table_tokens => "side_table",
    side_table_from_box_tokens => "side_table_from_box",
    stool_tokens => "stool",
    wall_shelf_tokens => "wall_shelf",
}
