//! The navigation layer on the example projects.

use std::path::{Path, PathBuf};
use std::time::Instant;

use stave_ide::{
    Analysis, Buffers, Document, FilePosition, Overlay, SymbolKind, TokenClass, UseKind,
};

fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .canonicalize()
        .unwrap()
}

fn cabinet() -> (PathBuf, Analysis) {
    let folder = examples().join("clothing_cabinet");
    let analysis = Analysis::load(&folder, &Overlay::new()).unwrap();
    (folder, analysis)
}

/// The position of the `nth` (from 0) occurrence of `needle` on 1-based
/// `line` of `file`, plus `into` bytes.
fn at(analysis: &Analysis, folder: &Path, file: &str, line: usize, needle: &str) -> FilePosition {
    // `std/casework/drawer.stave` is a standard file; anything else is in
    // the project's folder.
    let document = if file.starts_with("std/") {
        Document::from_source_path(file)
    } else {
        Document::File(folder.join(file))
    };
    let id = analysis
        .file(&document)
        .unwrap_or_else(|| panic!("{file} isn't loaded"));
    let start = analysis.line_start(id, line).unwrap();
    let text = analysis.text(id).unwrap();
    let rest = &text[usize::try_from(start).unwrap()..];
    let line_text = rest.lines().next().unwrap();
    let column = line_text
        .find(needle)
        .unwrap_or_else(|| panic!("'{needle}' isn't on {file}:{line}: {line_text}"));
    FilePosition {
        file: id,
        offset: start + u32::try_from(column).unwrap() + 1,
    }
}

#[test]
fn a_component_from_another_module() {
    let (folder, analysis) = cabinet();
    let position = at(&analysis, &folder, "doors.stave", 9, "FramePanelDoor");
    let definition = analysis.definition(position).unwrap();
    assert_eq!(
        definition.label(&folder),
        "std/casework/frame_panel_door.stave:4"
    );
    // The module name before it goes to the module's library item.
    let position = at(&analysis, &folder, "doors.stave", 9, "casework");
    let definition = analysis.definition(position).unwrap();
    assert_eq!(definition.label(&folder), "std/casework/library.stave:15");
}

#[test]
fn uses_say_what_each_use_does() {
    let (folder, analysis) = cabinet();
    let position = at(&analysis, &folder, "doors.stave", 18, "door_l");
    let uses = analysis.uses_at(position).unwrap();
    assert_eq!(uses.name, "door_l");
    let listed: Vec<(String, String)> = uses
        .uses
        .iter()
        .map(|u| (u.location.label(&folder), u.description.clone()))
        .collect();
    assert_eq!(
        listed.first(),
        Some(&("doors.stave:9".to_owned(), "declared".to_owned()))
    );
    assert!(listed.contains(&("doors.stave:18".to_owned(), "door_r mirrors it".to_owned())));
    assert!(listed.contains(&(
        "cabinet.stave:35".to_owned(),
        "scene all_open poses it".to_owned()
    )));
    assert_eq!(uses.count(), uses.uses.len() - 1);
    // The lens above the declaration counts the same uses.
    let doors = position.file;
    let lens = analysis
        .lenses(doors)
        .into_iter()
        .find(|l| l.name == "door_l")
        .unwrap();
    assert_eq!(lens.uses, uses.count());
    assert!(lens.shows_in_3d());
    assert_eq!(lens.location.line, 9);
}

#[test]
fn hover_gives_the_doc_comment_and_the_declaration() {
    let (folder, analysis) = cabinet();
    let hover = analysis
        .hover(at(&analysis, &folder, "doors.stave", 9, "FramePanelDoor"))
        .unwrap();
    assert_eq!(hover.kind, "component");
    assert_eq!(hover.signature, "component FramePanelDoor");
    assert_eq!(
        hover.docs.as_deref(),
        Some("A frame-and-panel door: two stiles, two rails and a floating panel.")
    );
    let hover = analysis
        .hover(at(&analysis, &folder, "doors.stave", 9, "door_l"))
        .unwrap();
    assert_eq!(hover.signature, "assembly door_l : casework.FramePanelDoor");
    // A built-in function and an enum word say what they are.
    let hover = analysis
        .hover(at(&analysis, &folder, "doors.stave", 12, "overlay"))
        .unwrap();
    assert_eq!(hover.kind, "built-in function");
    assert!(hover.signature.starts_with("overlay(left:?"));
    let hover = analysis
        .hover(at(&analysis, &folder, "doors.stave", 18, "across"))
        .unwrap();
    assert_eq!(hover.docs.as_deref(), Some("'across' is an axis."));
}

#[test]
fn breadcrumbs_follow_the_cursor() {
    let (folder, analysis) = cabinet();
    let crumbs: Vec<String> = analysis
        .breadcrumbs(at(&analysis, &folder, "doors.stave", 12, "18mm"))
        .into_iter()
        .map(|c| c.label)
        .collect();
    assert_eq!(crumbs, ["doors.stave", "assembly door_l", "mount"]);
    let crumbs: Vec<String> = analysis
        .breadcrumbs(at(
            &analysis,
            &folder,
            "std/casework/frame_panel_door.stave",
            27,
            "door",
        ))
        .into_iter()
        .map(|c| c.label)
        .collect();
    assert_eq!(
        crumbs,
        [
            "std/casework/frame_panel_door.stave",
            "component FramePanelDoor",
            "part stile",
            "place"
        ]
    );
}

#[test]
fn names_resolve_by_namespace() {
    let (folder, analysis) = cabinet();
    let go = |file: &str, line: usize, needle: &str| {
        analysis
            .definition(at(&analysis, &folder, file, line, needle))
            .map(|d| d.label(&folder))
    };
    // A label in [ ] is the array's label.
    assert_eq!(
        go("doors.stave", 10, "left").as_deref(),
        Some("carcass.stave:117")
    );
    assert_eq!(
        go("doors.stave", 10, "door_bay").as_deref(),
        Some("carcass.stave:117")
    );
    // A field of an assembly is its component's param.
    assert_eq!(
        go("doors.stave", 10, "opening").as_deref(),
        Some("std/casework/frame_panel_door.stave:5")
    );
    // A module's item through an `as` name.
    assert_eq!(
        go("doors.stave", 15, "brass_knob_26").as_deref(),
        Some("hardware/handles.stave:9")
    );
    assert_eq!(
        go("doors.stave", 15, "hw").as_deref(),
        Some("hardware/library.stave:8")
    );
    // A prelude kind is a read-only standard document.
    let dado = analysis
        .definition(at(&analysis, &folder, "carcass.stave", 99, "dado(["))
        .unwrap();
    assert_eq!(dado.document.uri(), "stave-std:///joinery/dado.stave");
    assert!(dado.document.is_read_only());
    // A named argument of a kind call is the kind's param.
    let depth = analysis
        .definition(at(&analysis, &folder, "carcass.stave", 99, "depth"))
        .unwrap();
    assert_eq!(depth.document.uri(), "stave-std:///joinery/dado.stave");
    // A component's own names, from inside it.
    assert_eq!(
        go("std/casework/frame_panel_door.stave", 23, "stile_width").as_deref(),
        Some("std/casework/frame_panel_door.stave:10")
    );
    // Enum words, built-ins and placement terms are no items.
    assert_eq!(go("doors.stave", 18, "across"), None);
    assert_eq!(go("doors.stave", 12, "overlay"), None);
    assert_eq!(go("std/casework/frame_panel_door.stave", 27, "flush"), None);
}

#[test]
fn colours_follow_the_namespaces() {
    let (folder, analysis) = cabinet();
    let doors = analysis
        .file(&Document::File(folder.join("doors.stave")))
        .unwrap();
    let text = analysis.text(doors).unwrap().to_owned();
    let tokens = analysis.tokens(doors);
    let class_of = |word: &str| {
        let start = u32::try_from(text.find(word).unwrap()).unwrap();
        tokens
            .iter()
            .find(|t| t.span.start() == start)
            .map(|t| t.class)
    };
    assert_eq!(class_of("assembly"), Some(TokenClass::Keyword));
    assert_eq!(class_of("FramePanelDoor"), Some(TokenClass::Type));
    assert_eq!(class_of("casework."), Some(TokenClass::Module));
    assert_eq!(class_of("18mm"), Some(TokenClass::Number));
    assert_eq!(class_of("overlay"), Some(TokenClass::Function));
    assert_eq!(class_of("\"std:casework\""), Some(TokenClass::Text));
    assert_eq!(class_of("// Two"), Some(TokenClass::Comment));
    assert_eq!(class_of("side_l"), Some(TokenClass::Item(SymbolKind::Part)));
    assert_eq!(class_of("across"), Some(TokenClass::EnumWord));
    assert_eq!(
        class_of("hinge_side"),
        Some(TokenClass::Item(SymbolKind::Param))
    );
}

#[test]
fn search_finds_files_and_names() {
    let (_, analysis) = cabinet();
    let found = analysis.search("door", 20);
    let labels: Vec<&str> = found.iter().map(|f| f.label.as_str()).collect();
    assert_eq!(labels.first(), Some(&"doors.stave"));
    assert!(labels.contains(&"door_l"));
    assert!(labels.contains(&"FramePanelDoor.door"));
    assert!(analysis.search("  ", 20).is_empty());
}

#[test]
fn open_buffers_are_read_in_place_of_their_files() {
    let folder = examples().join("clothing_cabinet");
    let doors = Document::File(folder.join("doors.stave"));
    let mut buffers = Buffers::new();
    let text = buffers.open(&doors).unwrap().to_owned();
    // Rename door_l in the buffer only, and leave a half-typed line.
    let edited = text.replace("door_l", "left_door") + "\nassembly half :\n";
    buffers.edit(&doors, &edited).unwrap();
    let analysis = Analysis::load(&folder, &buffers.overlay()).unwrap();
    assert!(analysis.project_symbol("left_door").is_some());
    assert!(analysis.project_symbol("door_l").is_none());
    let file = analysis.file(&doors).unwrap();
    assert!(analysis.diagnostics(file).next().is_some());
    // A typo is an unresolved name; a field name, an enum word and a name
    // that resolves are not.
    let typo = buffers
        .text(&doors)
        .unwrap()
        .replace("hinge_side:  side_l", "hinge_side:  side_q");
    buffers.edit(&doors, &typo).unwrap();
    let analysis = Analysis::load(&folder, &buffers.overlay()).unwrap();
    let file = analysis.file(&doors).unwrap();
    let text = analysis.text(file).unwrap();
    let at = |needle: &str| FilePosition {
        file,
        offset: u32::try_from(text.find(needle).unwrap()).unwrap() + 1,
    };
    assert!(analysis.is_unresolved(at("side_q")));
    assert!(!analysis.is_unresolved(at("hinge_side")));
    assert!(!analysis.is_unresolved(at("across")));
    assert!(!analysis.is_unresolved(at("oak_19")));
    // The rest of the file still resolves.
    let uses = analysis
        .uses(analysis.project_symbol("left_door").unwrap().id)
        .unwrap();
    assert!(
        uses.uses
            .iter()
            .any(|u| u.kind != UseKind::Declared && u.description == "door_r mirrors it")
    );
}

#[test]
fn every_example_and_the_standard_library_index() {
    let mut folders: Vec<PathBuf> = std::fs::read_dir(examples())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .collect();
    folders.sort();
    for folder in folders {
        let started = Instant::now();
        let analysis = Analysis::load(&folder, &Overlay::new()).unwrap();
        let elapsed = started.elapsed();
        assert!(
            elapsed.as_millis() < 2000,
            "{} took {elapsed:?}",
            folder.display()
        );
        for (file, _) in analysis.project_files() {
            let text = analysis.text(file).unwrap();
            let length = u32::try_from(text.len()).unwrap();
            // Every token is covered in order, and every declaration's
            // definition is itself.
            let mut last = 0;
            for token in analysis.tokens(file) {
                assert!(token.span.start() >= last && token.span.end() <= length);
                last = token.span.end();
            }
            for lens in analysis.lenses(file) {
                let position = FilePosition {
                    file,
                    offset: lens.location.span.start(),
                };
                assert_eq!(analysis.symbol_at(position), Some(lens.symbol));
            }
        }
    }
}
