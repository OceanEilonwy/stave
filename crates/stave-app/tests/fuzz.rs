//! Robustness of the whole front end (modules, names and types, placement
//! and sizes): a deterministic mutation loop over the example projects
//! edits one file of a project at a time and runs the pipeline. For every
//! mutant: no panic, no hang (a watchdog fails the test), every diagnostic
//! has a help line, and every span lies inside its file.
//!
//! `STAVE_FUZZ_ITERATIONS` sets the number of mutants (default 120; each
//! runs the whole pipeline), and `STAVE_FUZZ_SEED` the seed.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use stave_modules::MemoryFiles;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, bound: usize) -> usize {
        usize::try_from(self.next() % u64::try_from(bound.max(1)).unwrap()).unwrap()
    }
}

fn env_number(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
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

/// Edits that keep the file mostly intact, so the later stages see a
/// nearly valid design: names, numbers, units, operators, brackets, words.
const SNIPPETS: &[&str] = &[
    "(",
    ")",
    "{",
    "}",
    "[",
    "]",
    ",",
    ":",
    "\n",
    "-",
    "*",
    "/ 0",
    "0",
    "1",
    "0mm",
    "-1mm",
    "100000m",
    "1deg",
    "1/3in",
    "fit",
    "base",
    "same",
    "[*]",
    "[same]",
    "[0]",
    ".top",
    ".length",
    "x",
    "up",
    "left",
    "through",
    "true",
    "\"",
    "mirror(",
    "orient(length: up)",
    "ground",
    "flush(base.left)",
    "against(base.top)",
    "evenly",
    "between(",
    "extent(",
    "mid(",
];

fn char_boundary_at(text: &str, mut at: usize) -> usize {
    at = at.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

fn mutate(text: &str, rng: &mut Rng) -> String {
    let at = char_boundary_at(text, rng.below(text.len() + 1));
    match rng.below(5) {
        0 => {
            let end = char_boundary_at(text, at + 1 + rng.below(6));
            format!("{}{}", &text[..at], &text[end..])
        }
        1 | 2 => {
            let snippet = SNIPPETS[rng.below(SNIPPETS.len())];
            format!("{}{snippet}{}", &text[..at], &text[at..])
        }
        3 => {
            // Replace a word with another word of the file.
            let words: Vec<&str> = text
                .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .filter(|w| !w.is_empty())
                .collect();
            let rest = &text[at..];
            let length = rest
                .char_indices()
                .find(|(_, c)| !(c.is_alphanumeric() || *c == '_'))
                .map_or(rest.len(), |(i, _)| i);
            let word = words.get(rng.below(words.len())).copied().unwrap_or("x");
            format!("{}{word}{}", &text[..at], &rest[length..])
        }
        _ => {
            let mut lines: Vec<&str> = text.lines().collect();
            if lines.len() > 2 {
                let a = rng.below(lines.len());
                let b = rng.below(lines.len());
                lines.swap(a, b);
            }
            lines.join("\n") + "\n"
        }
    }
}

/// Runs the pipeline on the examples with one file replaced. Returns what
/// is wrong, if anything.
fn check(files: &[(PathBuf, String)], example: &Path) -> Result<(), String> {
    let mut memory = MemoryFiles::new();
    for (path, text) in files {
        memory.insert(Path::new("/").join(path), text.as_str());
    }
    let opened = stave_app::open_with(&Path::new("/").join(example), &memory)
        .map_err(|error| format!("the example doesn't load: {error}"))?;
    for diagnostic in &opened.diagnostics {
        if diagnostic.help.as_deref().is_none_or(str::is_empty) {
            return Err(format!("a diagnostic without a help line: {diagnostic:?}"));
        }
        for label in &diagnostic.labels {
            let file = opened
                .sources
                .get(label.file)
                .ok_or_else(|| format!("a label in no file: {diagnostic:?}"))?;
            let text = file.text();
            let (start, end) = (
                usize::try_from(label.span.start()).unwrap(),
                usize::try_from(label.span.end()).unwrap(),
            );
            if start > end
                || end > text.len()
                || !text.is_char_boundary(start)
                || !text.is_char_boundary(end)
            {
                return Err(format!("a span outside its file: {diagnostic:?}"));
            }
        }
    }
    Ok(())
}

#[test]
fn mutated_projects_never_panic_or_hang_and_report_well_formed_diagnostics() {
    let iterations = env_number("STAVE_FUZZ_ITERATIONS", 120);
    let seed = env_number("STAVE_FUZZ_SEED", 0x00f1_7ed5);
    let files = example_files();
    let mut examples: Vec<PathBuf> = std::fs::read_dir(workspace().join("examples"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_dir())
        .map(|path| path.strip_prefix(workspace()).unwrap().to_path_buf())
        .collect();
    examples.sort();
    let (progress, done) = mpsc::channel::<String>();
    let worker = std::thread::spawn(move || {
        let mut rng = Rng(seed | 1);
        let mut failures = Vec::new();
        for _ in 0..iterations {
            let example = examples[rng.below(examples.len())].clone();
            let own: Vec<usize> = files
                .iter()
                .enumerate()
                .filter(|(_, (path, _))| path.starts_with(&example))
                .map(|(index, _)| index)
                .collect();
            let target = own[rng.below(own.len())];
            let mut changed = files.clone();
            let mut text = changed[target].1.clone();
            for _ in 0..=rng.below(2) {
                text = mutate(&text, &mut rng);
            }
            changed[target].1 = text.clone();
            let label = format!("{}:\n{text}", changed[target].0.display());
            let _ = progress.send(label.clone());
            match catch_unwind(AssertUnwindSafe(|| check(&changed, &example))) {
                Ok(Ok(())) => {}
                Ok(Err(problem)) => failures.push(format!("{problem}\n--- in {label}")),
                Err(_) => failures.push(format!("panicked\n--- in {label}")),
            }
            if failures.len() >= 3 {
                break;
            }
        }
        failures
    });
    let mut last = String::new();
    loop {
        match done.recv_timeout(Duration::from_secs(60)) {
            Ok(label) => last = label,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => panic!("the pipeline hangs on {last}"),
        }
    }
    let failures = worker
        .join()
        .expect("the worker reports instead of panicking");
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn the_deepest_expression_the_parser_accepts_checks_without_overflowing() {
    // Just under the parser's nesting limit, through every stage.
    let deep = format!("{}1mm{}", "(".repeat(120), ")".repeat(120));
    let long = vec!["1mm"; 120].join(" + ");
    let members = "base.stock.thickness";
    let opened = std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(move || {
            let body = format!(
                "let deep = {deep}\nlet long = {long}\nlet member = {members}\nrequire r : deep == 1mm && long == 120mm \"deep\""
            );
            let mut memory = MemoryFiles::new();
            memory.insert(
                "/project/project.stave",
                format!(
                    "stave 1\n\nproject \"T\" {{\n  units: mm\n}}\n\nstock s {{ species: pine, form: solid, thickness: 19mm }}\n\npart base : s {{\n  length: 1mm\n  width:  1mm\n  place {{\n    ground\n  }}\n}}\n{body}\n"
                ),
            );
            let opened = stave_app::open_with(Path::new("/project"), &memory).unwrap();
            opened
                .diagnostics
                .iter()
                .map(|d| d.message.clone())
                .collect::<Vec<_>>()
        })
        .unwrap()
        .join()
        .expect("no stack overflow");
    assert!(opened.is_empty(), "{opened:?}");
}
