//! The query layer: an [`AnalysisHost`] re-runs only what a change
//! touches, and gives exactly what a fresh [`Analysis::load`] gives.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use stave_ide::{Analysis, AnalysisHost, Document, FilePosition, Overlay};
use stave_lang::FileId;

fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .canonicalize()
        .unwrap()
}

/// Every example folder, sorted.
fn example_folders() -> Vec<PathBuf> {
    let mut folders: Vec<PathBuf> = std::fs::read_dir(examples())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_dir())
        .collect();
    folders.sort();
    folders
}

fn file_of(analysis: &Analysis, path: &Path) -> FileId {
    analysis
        .file(&Document::File(path.to_path_buf()))
        .unwrap_or_else(|| panic!("{} isn't loaded", path.display()))
}

/// Everything an editor reads from an analysis, written out: for every
/// loaded file its tokens, lenses and fast diagnostics, and for every
/// symbol its declaration and uses.
fn everything(analysis: &Analysis) -> String {
    let mut out = String::new();
    let mut files: Vec<(FileId, Document)> = analysis.project_files();
    for name in ["dado", "FramePanelDoor", "euro_hinge"] {
        if let Some(found) = analysis.search(name, 1).first() {
            files.push((found.location.file, found.location.document.clone()));
        }
    }
    for (file, document) in &files {
        writeln!(out, "file {document:?}").unwrap();
        writeln!(out, "  tokens {:?}", analysis.tokens(*file)).unwrap();
        writeln!(out, "  lenses {:?}", analysis.lenses(*file)).unwrap();
        let diagnostics: Vec<_> = analysis.diagnostics(*file).collect();
        writeln!(out, "  diagnostics {diagnostics:?}").unwrap();
        let text = analysis.text(*file).unwrap();
        let mut offset = 0u32;
        for word in text.split_inclusive(|c: char| !c.is_alphanumeric() && c != '_') {
            let position = FilePosition {
                file: *file,
                offset,
            };
            if let Some(symbol) = analysis.symbol_at(position) {
                let uses = analysis.uses(symbol).unwrap();
                writeln!(out, "  {offset} {symbol:?} {uses:?}").unwrap();
            }
            offset += u32::try_from(word.len()).unwrap();
        }
    }
    writeln!(out, "search {:?}", analysis.search("a", 1000)).unwrap();
    out
}

/// A file's text with `from` (which must occur) replaced by `to` once.
fn edited(path: &Path, from: &str, to: &str) -> String {
    let text = std::fs::read_to_string(path).unwrap();
    assert!(text.contains(from), "{} has no {from:?}", path.display());
    text.replacen(from, to, 1)
}

#[test]
fn an_edit_inside_one_item_parses_and_resolves_only_its_file() {
    let folder = examples().join("clothing_cabinet");
    let carcass = folder.join("carcass.stave");
    let mut host = AnalysisHost::new();
    let first = host.analyse(&folder, &Overlay::new()).unwrap();
    let loaded = host.stats().clone();
    assert!(loaded.parsed.len() > 10, "the first load parses everything");
    assert_eq!(loaded.parsed.len(), loaded.resolved.len());

    // Change a value inside `part top`.
    let mut overlay = Overlay::new();
    overlay.insert(
        &carcass,
        edited(
            &carcass,
            "cabinet_width + 2 * overhang",
            "cabinet_width + 3 * overhang",
        ),
    );
    let second = host.analyse(&folder, &overlay).unwrap();
    let file = file_of(&second, &carcass);
    let stats = host.stats();
    assert_eq!(stats.parsed, [file]);
    assert_eq!(stats.items, [file]);
    assert_eq!(stats.resolved, [file]);
    assert_eq!(stats.module_names.len(), 1, "only the project's names");
    assert_eq!(file_of(&first, &carcass), file);

    // Nothing changed: nothing runs.
    host.analyse(&folder, &overlay).unwrap();
    let stats = host.stats();
    assert!(stats.parsed.is_empty() && stats.items.is_empty() && stats.resolved.is_empty());
    assert!(stats.module_names.is_empty());
}

#[test]
fn renaming_an_item_resolves_only_the_files_that_name_it() {
    let folder = examples().join("clothing_cabinet");
    let mut host = AnalysisHost::new();
    let before = host.analyse(&folder, &Overlay::new()).unwrap();
    let symbol = before.project_symbol("cabinet_depth").unwrap().clone();
    let declared_in = before.document(symbol.file).unwrap();
    let Document::File(path) = &declared_in else {
        panic!("not a file");
    };
    let naming: BTreeSet<FileId> = before
        .uses(symbol.id)
        .unwrap()
        .uses
        .iter()
        .map(|found| found.location.file)
        .collect();
    assert!(naming.len() > 1, "named in more than its own file");

    // Rename the declaration only: every use now names nothing.
    let mut overlay = Overlay::new();
    overlay.insert(
        path,
        edited(path, "param cabinet_depth", "param cabinet_depth_x"),
    );
    let after = host.analyse(&folder, &overlay).unwrap();
    let resolved: BTreeSet<FileId> = host.stats().resolved.iter().copied().collect();
    assert_eq!(resolved, naming);
    assert_eq!(host.stats().parsed, [symbol.file]);
    assert_eq!(
        everything(&after),
        everything(&Analysis::load(&folder, &overlay).unwrap())
    );
}

#[test]
fn tokens_worked_out_once_are_kept_for_unchanged_files() {
    let folder = examples().join("clothing_cabinet");
    let carcass = folder.join("carcass.stave");
    let doors = folder.join("doors.stave");
    let mut host = AnalysisHost::new();
    let first = host.analyse(&folder, &Overlay::new()).unwrap();
    let doors_tokens = first.tokens(file_of(&first, &doors));
    first.tokens(file_of(&first, &carcass));

    let mut overlay = Overlay::new();
    overlay.insert(&carcass, edited(&carcass, "depth: 9mm", "depth: 8mm"));
    let second = host.analyse(&folder, &overlay).unwrap();
    assert_eq!(host.stats().tokens_kept, [file_of(&second, &doors)]);
    assert_eq!(second.tokens(file_of(&second, &doors)), doors_tokens);
}

#[test]
fn completions_are_worked_out_once_per_analysis() {
    let folder = examples().join("clothing_cabinet");
    let carcass = folder.join("carcass.stave");
    let analysis = Analysis::load(&folder, &Overlay::new()).unwrap();
    let file = file_of(&analysis, &carcass);
    let text = analysis.text(file).unwrap();
    let offset =
        u32::try_from(text.find("against(base.top)").unwrap() + "against(base.".len()).unwrap();
    let position = FilePosition { file, offset };
    let first = analysis.completions(position);
    assert!(first.iter().any(|completion| completion.label == "top"));
    assert_eq!(analysis.clone().completions(position), first);
}

/// Edits made one after another, each compared with a fresh load.
#[test]
fn every_result_after_edits_is_what_a_fresh_load_gives() {
    for folder in example_folders() {
        let files = stave_ide::project_files(&folder);
        let Some(first) = files.first() else {
            continue;
        };
        let original = std::fs::read_to_string(first).unwrap();
        let middle = original.len() / 2;
        let middle = (0..=middle)
            .rev()
            .find(|i| original.is_char_boundary(*i))
            .unwrap();
        // The first declared name of the file, renamed.
        let mut host = AnalysisHost::new();
        let loaded = host.analyse(&folder, &Overlay::new()).unwrap();
        let renamed = loaded
            .project_items()
            .into_iter()
            .find(|item| item.file == file_of(&loaded, first))
            .map(|item| {
                let span = item.name_span;
                let start = usize::try_from(span.start()).unwrap();
                let end = usize::try_from(span.end()).unwrap();
                format!(
                    "{}{}_renamed{}",
                    &original[..start],
                    &original[start..end],
                    &original[end..]
                )
            });
        let mut steps = vec![
            format!("{original}\n// a comment\n"),
            format!("{}\npart\n{}", &original[..middle], &original[middle..]),
            format!("{}{{{}", &original[..middle], &original[middle..]),
            String::new(),
        ];
        steps.extend(renamed);
        steps.push(original.clone());
        for (step, text) in steps.iter().enumerate() {
            let mut overlay = Overlay::new();
            overlay.insert(first, text.clone());
            // A load can fail (an empty folder); then both fail.
            let incremental = host.analyse(&folder, &overlay);
            let fresh = Analysis::load(&folder, &overlay);
            match (incremental, fresh) {
                (Ok(incremental), Ok(fresh)) => assert_eq!(
                    everything(&incremental),
                    everything(&fresh),
                    "{} after step {step}",
                    folder.display()
                ),
                (Err(_), Err(_)) => {}
                (incremental, fresh) => panic!(
                    "{} step {step}: {:?} vs {:?}",
                    folder.display(),
                    incremental.err(),
                    fresh.err()
                ),
            }
        }
    }
}

/// A rename that a file elsewhere resolves differently afterwards: the
/// project's new name now hides a name of the prelude.
#[test]
fn a_new_name_that_hides_the_prelude_resolves_its_users_again() {
    let folder = examples().join("clothing_cabinet");
    let carcass = folder.join("carcass.stave");
    let mut host = AnalysisHost::new();
    host.analyse(&folder, &Overlay::new()).unwrap();
    // `dado` is the standard joint kind the carcass uses; a project item
    // named `dado` now takes its place.
    let mut overlay = Overlay::new();
    overlay.insert(&carcass, edited(&carcass, "part shelf :", "part dado :"));
    let after = host.analyse(&folder, &overlay).unwrap();
    assert_eq!(
        everything(&after),
        everything(&Analysis::load(&folder, &overlay).unwrap())
    );
}

/// Edits in one module that change what other modules' files find: a
/// member made private, renamed, a module losing its header, an assembly
/// made an instance of another component, an import renamed, applied one
/// on top of the other and then undone, each compared with a fresh load.
#[test]
fn edits_across_modules_give_what_a_fresh_load_gives() {
    let folder = examples().join("clothing_cabinet");
    let handles = folder.join("hardware/handles.stave");
    let library = folder.join("hardware/library.stave");
    let doors = folder.join("doors.stave");
    let cabinet = folder.join("cabinet.stave");
    let steps: [(&Path, &str, &str); 6] = [
        (&handles, "kind brass_knob_26", "private kind brass_knob_26"),
        (
            &handles,
            "private kind brass_knob_26",
            "private kind brass_knob_27",
        ),
        (&library, "library \"Shop hardware\"", ""),
        (&doors, "casework.FramePanelDoor", "casework.Drawer"),
        (&doors, "as hw ", "as hx "),
        (&cabinet, "project \"Clothing cabinet\"", "settings"),
    ];
    let mut host = AnalysisHost::new();
    host.analyse(&folder, &Overlay::new()).unwrap();
    let mut texts: Vec<(PathBuf, String)> = Vec::new();
    let check = |host: &mut AnalysisHost, texts: &[(PathBuf, String)], what: &str| {
        let mut overlay = Overlay::new();
        for (path, text) in texts {
            overlay.insert(path, text.clone());
        }
        let incremental = host.analyse(&folder, &overlay).unwrap();
        let fresh = Analysis::load(&folder, &overlay).unwrap();
        assert_eq!(everything(&incremental), everything(&fresh), "{what}");
    };
    for (path, from, to) in steps {
        let text = match texts.iter().position(|(kept, _)| kept == path) {
            Some(at) => texts.remove(at).1,
            None => std::fs::read_to_string(path).unwrap(),
        };
        assert!(text.contains(from), "{} has no {from:?}", path.display());
        texts.push((path.to_path_buf(), text.replacen(from, to, 1)));
        check(&mut host, &texts, &format!("{from:?} -> {to:?}"));
    }
    // Undone one file at a time.
    while let Some((path, _)) = texts.pop() {
        check(&mut host, &texts, &format!("{} undone", path.display()));
    }
}

fn mean(durations: &[Duration]) -> Duration {
    let total: Duration = durations.iter().sum();
    total / u32::try_from(durations.len().max(1)).unwrap()
}

/// What a keystroke costs on the largest example and its imports: a
/// field typed letter by letter into a part, each keystroke followed by
/// the tokens of the file and the completions at the cursor. Run with
/// `cargo test --release -p stave-ide --test incremental -- --ignored
/// --nocapture`.
#[test]
#[ignore = "a benchmark: run in release mode with --ignored --nocapture"]
fn keystrokes_on_the_largest_example() {
    let folder = examples().join("clothing_cabinet");
    let carcass = folder.join("carcass.stave");
    let original = std::fs::read_to_string(&carcass).unwrap();
    let anchor = "  width:  cabinet_depth          // wider";
    let at = original.find(anchor).unwrap();
    let typed = "  depth: cabinet_depth - overhang\n";
    let rounds = 20;

    let mut full = Vec::new();
    let mut analyse = Vec::new();
    let mut tokens = Vec::new();
    let mut completions = Vec::new();
    let mut host = AnalysisHost::new();
    host.analyse(&folder, &Overlay::new()).unwrap();
    let mut files_loaded = 0;
    for _ in 0..rounds {
        for typed_length in 1..=typed.len() {
            let text = format!(
                "{}{}{}",
                &original[..at],
                &typed[..typed_length],
                &original[at..]
            );
            let mut overlay = Overlay::new();
            overlay.insert(&carcass, text);

            let started = Instant::now();
            let analysis = host.analyse(&folder, &overlay).unwrap();
            analyse.push(started.elapsed());
            assert!(host.stats().resolved.len() <= 1, "{:?}", host.stats());

            let file = file_of(&analysis, &carcass);
            let started = Instant::now();
            std::hint::black_box(analysis.tokens(file));
            tokens.push(started.elapsed());

            let offset = u32::try_from(at + typed_length).unwrap();
            let started = Instant::now();
            std::hint::black_box(analysis.completions(FilePosition { file, offset }));
            completions.push(started.elapsed());

            if typed_length % 8 == 0 {
                let started = Instant::now();
                let fresh = Analysis::load(&folder, &overlay).unwrap();
                full.push(started.elapsed());
                files_loaded = fresh.project_files().len();
            }
        }
    }
    println!(
        "clothing_cabinet ({files_loaded} project files plus the standard library), {} keystrokes:",
        analyse.len()
    );
    println!("  full load (Analysis::load)       {:?}", mean(&full));
    println!("  incremental (AnalysisHost)       {:?}", mean(&analyse));
    println!("  tokens of the edited file        {:?}", mean(&tokens));
    println!(
        "  completions at the cursor        {:?}",
        mean(&completions)
    );
    let keystroke = mean(&analyse) + mean(&tokens) + mean(&completions);
    println!("  a keystroke, all three           {keystroke:?}");
}
