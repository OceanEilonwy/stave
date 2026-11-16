//! Robustness: the lexer, parser and formatter never panic and never hang,
//! whatever the input. A deterministic mutation loop over the corpus
//! (examples, std and the broken designs) deletes, inserts, duplicates and
//! swaps bytes, tokens and lines, and checks every mutant:
//!
//! - parsing finishes (a watchdog fails the test on a hang) and doesn't
//!   panic;
//! - the tree is lossless, every diagnostic span lies inside the file on
//!   character boundaries, and every diagnostic has a help line;
//! - when the mutant parses without errors, formatting it is idempotent and
//!   re-parses to the same tree.
//!
//! `STAVE_FUZZ_ITERATIONS` sets the number of mutants (default 1500), and
//! `STAVE_FUZZ_SEED` the seed.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use stave_lang::{Parse, SourceMap, format_file, parse_file, parse_patch_file};

/// A small deterministic random number generator (xorshift64*).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    /// A number in `0..bound` (`bound` > 0).
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

fn collect(dir: &Path, files: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect(&path, files);
        } else if path.extension().is_some_and(|e| e == "stave") {
            files.push(path);
        }
    }
}

fn corpus() -> Vec<String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    for folder in ["examples", "std", "tests/broken"] {
        collect(&root.join(folder), &mut files);
    }
    files
        .into_iter()
        .map(|path| std::fs::read_to_string(path).unwrap())
        .collect()
}

/// Snippets worth inserting: brackets, separators, keywords, odd literals
/// and characters that need care (multi-byte, a byte order mark).
const SNIPPETS: &[&str] = &[
    "(",
    ")",
    "{",
    "}",
    "[",
    "]",
    ",",
    ":",
    ".",
    "..",
    "=",
    "-",
    "*",
    "/",
    "!",
    "\"",
    "\n",
    "\n\n",
    "\\",
    "{x}",
    "\\u{e9}",
    "/*",
    "*/",
    "//",
    "///",
    "1/0in",
    "3inch",
    "1-1/2in",
    "99999999999999999999mm",
    "0.",
    "part",
    "place",
    "same",
    "fit",
    "within",
    "joint",
    "stave 1",
    "import \"./x\"",
    "é",
    "\u{feff}",
    "\t",
    " ",
    "_",
    "[same]",
    "[*]",
    "mirror(",
    "about:",
    "kind k : joint {",
    "patch \"p\" {",
    "add",
    "set a = 1",
];

fn char_boundary_at(text: &str, mut at: usize) -> usize {
    at = at.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// One random edit of `text`.
fn mutate(text: &str, rng: &mut Rng) -> String {
    let length = text.len();
    let at = char_boundary_at(text, rng.below(length + 1));
    match rng.below(7) {
        // Delete a few bytes.
        0 => {
            let end = char_boundary_at(text, at + 1 + rng.below(8));
            format!("{}{}", &text[..at], &text[end..])
        }
        // Insert a snippet.
        1 | 2 => {
            let snippet = SNIPPETS[rng.below(SNIPPETS.len())];
            format!("{}{snippet}{}", &text[..at], &text[at..])
        }
        // Delete a word or a punctuation run (roughly a token).
        3 => {
            let rest = &text[at..];
            let word = rest
                .char_indices()
                .find(|(_, c)| !(c.is_alphanumeric() || *c == '_'))
                .map_or(rest.len(), |(i, _)| i)
                .max(rest.chars().next().map_or(0, char::len_utf8));
            format!("{}{}", &text[..at], &rest[word..])
        }
        // Duplicate or swap lines.
        4 | 5 => {
            let mut lines: Vec<&str> = text.lines().collect();
            if lines.len() < 2 {
                return text.to_owned();
            }
            let a = rng.below(lines.len());
            let b = rng.below(lines.len());
            if rng.below(2) == 0 {
                lines.swap(a, b);
            } else {
                let line = lines[a];
                lines.insert(b, line);
            }
            lines.join("\n") + "\n"
        }
        // Cut the file short.
        _ => text[..at].to_owned(),
    }
}

/// Checks one parse of `text`. Returns a description of what is wrong.
fn check_parse<T: std::fmt::Debug>(text: &str, parse: &Parse<T>) -> Result<(), String> {
    if parse.syntax().to_string() != text {
        return Err("the tree is not lossless".into());
    }
    for diagnostic in &parse.diagnostics {
        for label in &diagnostic.labels {
            let (start, end) = (
                usize::try_from(label.span.start()).unwrap(),
                usize::try_from(label.span.end()).unwrap(),
            );
            if start > end
                || end > text.len()
                || !text.is_char_boundary(start)
                || !text.is_char_boundary(end)
            {
                return Err(format!("a span outside the file: {diagnostic:?}"));
            }
        }
        if diagnostic.help.as_deref().is_none_or(str::is_empty) {
            return Err(format!("a diagnostic without a help line: {diagnostic:?}"));
        }
    }
    Ok(())
}

/// The `Debug` text of a tree with every span blanked.
fn without_spans(debug: &str) -> String {
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

fn check_mutant(text: &str) -> Result<(), String> {
    let mut sources = SourceMap::new();
    let file = sources.add("fuzz.stave", text).map_err(|e| e.to_string())?;
    if stave_lang::is_patch_source(text) {
        let parse = parse_patch_file(file, text);
        return check_parse(text, &parse);
    }
    let parse = parse_file(file, text);
    check_parse(text, &parse)?;
    if parse.has_errors() {
        return Ok(());
    }
    let formatted = format_file(file, text).map_err(|e| format!("can't format: {e}"))?;
    let again = format_file(file, &formatted).map_err(|e| format!("can't format twice: {e}"))?;
    if again != formatted {
        return Err(format!(
            "formatting is not idempotent:\n--- once\n{formatted}\n--- twice\n{again}"
        ));
    }
    let reparsed = parse_file(file, &formatted);
    if reparsed.has_errors() {
        return Err(format!(
            "the formatted text doesn't parse: {:?}\n{formatted}",
            reparsed.diagnostics
        ));
    }
    if without_spans(&format!("{:?}", reparsed.ast)) != without_spans(&format!("{:?}", parse.ast)) {
        return Err(format!("formatting changed the tree:\n{formatted}"));
    }
    Ok(())
}

#[test]
fn mutated_sources_never_panic_hang_or_lose_text() {
    let iterations = env_number("STAVE_FUZZ_ITERATIONS", 1500);
    let seed = env_number("STAVE_FUZZ_SEED", 0x5eed_57a7e);
    let corpus = corpus();
    assert!(corpus.len() >= 40, "the corpus went missing");
    let (progress, done) = mpsc::channel::<(u64, String)>();
    let worker = std::thread::spawn(move || {
        let mut rng = Rng(seed | 1);
        let mut failures = Vec::new();
        for iteration in 0..iterations {
            let source = &corpus[rng.below(corpus.len())];
            // One to three edits on top of each other.
            let mut text = source.clone();
            for _ in 0..=rng.below(3) {
                text = mutate(&text, &mut rng);
            }
            let _ = progress.send((iteration, text.clone()));
            match catch_unwind(AssertUnwindSafe(|| check_mutant(&text))) {
                Ok(Ok(())) => {}
                Ok(Err(problem)) => failures.push(format!("{problem}\n--- input\n{text:?}")),
                Err(_) => failures.push(format!("panicked\n--- input\n{text:?}")),
            }
            if failures.len() >= 5 {
                break;
            }
        }
        failures
    });
    // A watchdog: each mutant must finish within a generous bound.
    let mut last = String::new();
    loop {
        match done.recv_timeout(Duration::from_secs(20)) {
            Ok((_, text)) => last = text,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                panic!("parsing or formatting hangs on:\n{last:?}")
            }
        }
    }
    let failures = worker
        .join()
        .expect("the worker reports instead of panicking");
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn pathological_inputs_finish() {
    for text in [
        "stave 1\n".to_owned() + &"(".repeat(10_000),
        "stave 1\n".to_owned() + &"[".repeat(10_000) + &"]".repeat(10_000),
        "stave 1\nlet a = ".to_owned() + &"1 + ".repeat(5_000) + "1\n",
        "stave 1\nlet a = b".to_owned() + &".c".repeat(5_000) + "\n",
        "stave 1\nlet a = f".to_owned() + &"()".repeat(5_000) + "\n",
        "stave 1\nlet a = x".to_owned() + &"[0]".repeat(5_000) + "\n",
        "stave 1\nlet a = ".to_owned() + &"-".repeat(5_000) + "1\n",
        "stave 1\n".to_owned() + &"part p : s {\n".repeat(2_000),
        "stave 1\n".to_owned() + &"\"".repeat(5_001),
        "stave 1\n".to_owned() + &"/*".repeat(5_000),
        "stave 1\n".to_owned() + &"///\n".repeat(5_000),
        "\u{feff}".repeat(100),
        "é".repeat(1_000),
    ] {
        let result = catch_unwind(|| check_mutant(&text));
        assert!(
            matches!(result, Ok(Ok(()))),
            "{:?} on {:?}...",
            result.map(|r| r.err()),
            &text[..text.len().min(60)]
        );
    }
}
