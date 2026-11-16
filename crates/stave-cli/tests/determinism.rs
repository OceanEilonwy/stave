//! Determinism (CLAUDE.md, "Rust standards"; grammar, "Modules"): the
//! same design gives byte-identical results run to run, whatever order the
//! files are listed or created in, and however a module is split into
//! files.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use stave_lang::render_diagnostics;
use stave_modules::{FileSource, MemoryFiles};

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The example project folders.
fn examples() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(workspace().join("examples"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_dir())
        .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// Every `.stave` file under `examples/`, relative to the workspace.
fn example_files() -> Vec<(PathBuf, String)> {
    let mut files = Vec::new();
    let mut stack = vec![workspace().join("examples")];
    while let Some(folder) = stack.pop() {
        for entry in std::fs::read_dir(&folder).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "stave") {
                let relative = path.strip_prefix(workspace()).unwrap().to_path_buf();
                files.push((relative, std::fs::read_to_string(&path).unwrap()));
            }
        }
    }
    files.sort();
    files
}

/// The pipeline's observable result: the solved design as JSON (as
/// `stave dump --json` writes it) and the rendered diagnostics.
fn result(opened: &stave_app::Opened) -> (String, String) {
    let json = opened
        .design
        .as_ref()
        .map(|design| serde_json::to_string_pretty(design).unwrap())
        .unwrap_or_default();
    (
        json,
        render_diagnostics(&opened.sources, &opened.diagnostics),
    )
}

/// Files in memory, listed in reverse order.
struct Reversed(MemoryFiles);

impl FileSource for Reversed {
    fn list_stave_files(&self, folder: &Path) -> io::Result<Vec<PathBuf>> {
        let mut files = self.0.list_stave_files(folder)?;
        files.sort();
        files.reverse();
        Ok(files)
    }

    fn read(&self, path: &Path) -> io::Result<String> {
        self.0.read(path)
    }

    fn canonical(&self, folder: &Path) -> io::Result<PathBuf> {
        self.0.canonical(folder)
    }
}

fn memory(files: &[(PathBuf, String)]) -> MemoryFiles {
    let mut memory = MemoryFiles::new();
    for (path, text) in files {
        memory.insert(Path::new("/").join(path), text.as_str());
    }
    memory
}

#[test]
fn every_example_gives_identical_results_run_to_run() {
    for example in examples() {
        let folder = workspace().join("examples").join(&example);
        let first = result(&stave_app::open(&folder).unwrap());
        let second = result(&stave_app::open(&folder).unwrap());
        assert!(first == second, "{example}: two runs differ");
    }
}

#[test]
fn stave_dump_json_is_byte_identical_run_to_run() {
    for example in examples() {
        let run = || {
            Command::new(env!("CARGO_BIN_EXE_stave"))
                .args(["dump", "--json", &format!("examples/{example}")])
                .current_dir(workspace())
                .output()
                .unwrap()
        };
        let (first, second) = (run(), run());
        assert_eq!(first.stdout, second.stdout, "{example}");
        assert_eq!(first.stderr, second.stderr, "{example}");
    }
}

#[test]
fn the_order_files_are_listed_in_does_not_change_anything() {
    let files = example_files();
    for example in examples() {
        let folder = Path::new("/examples").join(&example);
        let forward = result(&stave_app::open_with(&folder, &memory(&files)).unwrap());
        let backward = result(&stave_app::open_with(&folder, &Reversed(memory(&files))).unwrap());
        assert!(
            forward == backward,
            "{example}: listing order changed the result"
        );
    }
}

#[test]
fn the_order_files_are_created_on_disk_in_does_not_change_anything() {
    let files = example_files();
    let root = std::env::temp_dir().join(format!("stave-determinism-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for (order, list) in [
        ("forward", files.clone()),
        ("backward", {
            let mut reversed = files.clone();
            reversed.reverse();
            reversed
        }),
    ] {
        for (path, text) in list {
            let target = root.join(order).join(path);
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::write(target, text).unwrap();
        }
    }
    for example in examples() {
        let open = |order: &str| {
            let folder = root.join(order).join("examples").join(&example);
            let (json, diagnostics) = result(&stave_app::open(&folder).unwrap());
            // Diagnostics name their files; the two copies live in different
            // folders.
            (
                json,
                diagnostics.replace(&root.join(order).display().to_string(), ""),
            )
        };
        assert!(
            open("forward") == open("backward"),
            "{example}: creation order changed the result"
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// Merges the files of a module into one: one header, every file's imports
/// (each once), then every file's items, in file-name order.
fn merge(texts: &[&str]) -> String {
    let mut imports: Vec<String> = Vec::new();
    let mut bodies = String::new();
    for text in texts {
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed == "stave 1" {
                continue;
            }
            if trimmed.starts_with("import ") {
                let import = trimmed.split("//").next().unwrap().trim().to_owned();
                if !imports.contains(&import) {
                    imports.push(import);
                }
                continue;
            }
            bodies.push_str(line);
            bodies.push('\n');
        }
        bodies.push('\n');
    }
    format!("stave 1\n\n{}\n\n{bodies}", imports.join("\n"))
}

#[test]
fn merging_a_modules_files_into_one_does_not_change_the_design() {
    let files = example_files();
    let mut merged_examples = Vec::new();
    for example in examples() {
        let folder = PathBuf::from("examples").join(&example);
        let own: Vec<&(PathBuf, String)> = files
            .iter()
            .filter(|(path, _)| path.parent() == Some(folder.as_path()))
            .collect();
        if own.len() < 2 {
            continue;
        }
        let merged_text = merge(
            &own.iter()
                .map(|(_, text)| text.as_str())
                .collect::<Vec<_>>(),
        );
        let merged: Vec<(PathBuf, String)> = files
            .iter()
            .filter(|(path, _)| path.parent() != Some(folder.as_path()))
            .cloned()
            .chain([(folder.join("all.stave"), merged_text)])
            .collect();
        let at = Path::new("/").join(&folder);
        let split = stave_app::open_with(&at, &memory(&files)).unwrap();
        let joined = stave_app::open_with(&at, &memory(&merged)).unwrap();
        assert!(
            joined.diagnostics.is_empty(),
            "{example}: {}",
            render_diagnostics(&joined.sources, &joined.diagnostics)
        );
        assert_eq!(
            result(&split).0,
            result(&joined).0,
            "{example}: merging changed the design"
        );
        merged_examples.push(example);
    }
    assert!(
        merged_examples.contains(&"clothing_cabinet".to_owned()),
        "{merged_examples:?}"
    );
}
